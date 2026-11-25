//! Composition: how the execution store is opened and how the order pipeline is built from
//! config (F10, BZ-205 slice 5).
//!
//! - [`open_store`] is the only way to open the execution store (audit chain verified).
//! - [`build_pipeline`] wires the real gate adapter, guards and the SQLite repos around an
//!   `Executor` the caller supplies; it returns the pipeline *unspawned* so startup can run
//!   `reload()` (and abort on error) before `spawn()`.
//! - [`SqliteGateContext`] is the production `GateContext`: the kill switch state and the
//!   quantity of other pending exits come from the store, never from the caller.
//! - [`UnknownCost`]: until the F9 cost model is wired the cost is `Unknown`, which blocks.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::clock::Clock;
use bazaar_core::config::{RiskConfig, parse_str};
use bazaar_core::ports::{KillSwitchRepo, OrderRepo, QuoteSource, RepoError};
use bazaar_execution::audit::chain_hash;
use bazaar_execution::breaker::{DailyLossBreaker, PnlSource};
use bazaar_execution::executor::{ExecError, Executor};
use bazaar_execution::gate_adapter::{GateContext, GateInputsOwned, RiskGateAdapter};
use bazaar_execution::guarded::{GuardedExecutor, SafetyGuard};
use bazaar_execution::killswitch::KillSwitch;
use bazaar_execution::pipeline::{Pipeline, PipelineConfig};
use bazaar_execution::ratelimit::RateLimiter;
use bazaar_execution::types::{IntentId, OrderIntent};
use bazaar_risk::baseline::LimitsBaseline;
use bazaar_risk::config::GateConfig;
use bazaar_risk::gate::{CostModel, ModeEvidence};
use bazaar_risk::types::{Side, Tri};
use bazaar_store::SqliteStore;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::runtime::Handle;

/// Open the SQLite store for execution work with the audit chain and the anchor file beside the
/// database verified (ADR 0003): a tampered, cut or swapped log is `Err(RepoError::Corrupt)` and
/// is never appended to; a non-empty log with no anchor needs the human
/// `history-checkpoint accept-current`. Never use `SqliteStore::open` for the execution store
/// (CR-128-2, SEC-128-2); `tests/compose.rs` fails if that happens.
///
/// # Errors
/// The store's `RepoError`; `Corrupt` when any audit row was altered, removed or reordered, or
/// the anchor check fails.
pub async fn open_store(path: &Path) -> Result<SqliteStore, RepoError> {
    SqliteStore::open_anchored(path, chain_hash).await
}

/// Mode evidence for a production run (RISK-136-5): no approval digest (the pipeline binds the
/// user's approval itself), every `auto` unlock flag `Known(false)` so `auto` stays locked, and
/// `live` `Known(false)` without `live-trading` (`Unknown`, so blocked, with it).
#[must_use]
pub fn prod_mode_evidence() -> ModeEvidence {
    ModeEvidence {
        approval: None,
        compliance_fresh: Tri::Known(false),
        readiness_report: Tri::Known(false),
        owner_opt_in: Tri::Known(false),
        live: if cfg!(feature = "live-trading") {
            Tri::Unknown
        } else {
            Tri::Known(false)
        },
    }
}

/// Cost model with no source: every cost is `Unknown`, so the gate blocks. The F9 cost model
/// replaces it when wired; nothing here ever guesses a charge.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnknownCost;

impl CostModel for UnknownCost {
    fn cost(&self, _: Side, _: u32, _: Decimal) -> Tri<Decimal> {
        Tri::Unknown
    }
}

/// Most orders read per state when summing pending exits; more than this is "unknown" (Block).
const STATE_SCAN_LIMIT: u32 = 1_000;

/// Order states in which an exit may still shrink (or be about to shrink) the position.
const LIVE_EXIT_STATES: [&str; 5] = [
    "pending_approval",
    "approved",
    "sent",
    "partially_filled",
    "unknown",
];

/// Quantity of `reduce_only` intents for `symbol`, other than `exclude`, that are pending,
/// approved, sent, partially filled or unknown (a partly filled exit counts in full: fail
/// closed). `None` when a list fails, is too large or a stored order cannot be decoded: unknown
/// blocks `reduce_only` in the adapter.
pub async fn pending_exit_qty<O: OrderRepo>(
    orders: &O,
    symbol: &str,
    exclude: IntentId,
    now: chrono::DateTime<chrono_tz::Tz>,
) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    for st in LIVE_EXIT_STATES {
        let rows = orders.list_in_state(st, STATE_SCAN_LIMIT).await.ok()?;
        for r in rows {
            let id = IntentId(r.intent_id);
            if id == exclude {
                continue;
            }
            let i = OrderIntent::from_payload(id, &r.payload, now)?;
            if i.reduce_only && i.symbol == symbol {
                total = total.checked_add(Decimal::from(i.qty))?;
            }
        }
    }
    Some(total)
}

/// Production `GateContext`: `base` supplies the market-side inputs (snapshot, data, instrument
/// master, stop, proposed price); this wrapper overrides what the store owns: the kill switch
/// state (unreadable = `Unknown`) and `pending_exit_qty` (unknown = `None` = Block).
///
/// `inputs` runs inside the pipeline's `spawn_blocking`, so it may block on the store; each
/// store call is bounded by the I/O timeout.
pub struct SqliteGateContext<B: GateContext> {
    base: B,
    store: SqliteStore,
    handle: Handle,
    io_timeout: Duration,
    clock: Arc<dyn Clock>,
}

impl<B: GateContext> std::fmt::Debug for SqliteGateContext<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteGateContext").finish_non_exhaustive()
    }
}

impl<B: GateContext> SqliteGateContext<B> {
    /// Takes the handle of the runtime that runs the store; the caller passes it in, so this
    /// never reaches for an ambient runtime.
    #[must_use]
    pub fn new(
        base: B,
        store: SqliteStore,
        handle: Handle,
        cfg: PipelineConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            base,
            store,
            handle,
            io_timeout: cfg.io_timeout(),
            clock,
        }
    }
}

impl<B: GateContext> GateContext for SqliteGateContext<B> {
    fn inputs(&self, intent: &OrderIntent) -> Option<GateInputsOwned> {
        // SEC-136-1: `Handle::block_on` below panics when called on an async worker (tokio checks
        // it in every build), so a call outside `spawn_blocking` fails loudly, never deadlocks;
        // the server's `runtime` tests pin that.
        let mut inp = self.base.inputs(intent)?;
        let now = self.clock.now_ist();
        let (killed, pending) = self.handle.block_on(async {
            let t = self.io_timeout;
            let killed = tokio::time::timeout(t, self.store.is_engaged())
                .await
                .ok()
                .and_then(Result::ok);
            let pending = tokio::time::timeout(
                t,
                pending_exit_qty(&self.store, &intent.symbol, intent.id, now),
            )
            .await
            .ok()
            .flatten();
            (killed, pending)
        });
        inp.snapshot.kill_switch = killed.map_or(Tri::Unknown, Tri::Known);
        inp.pending_exit_qty = pending;
        Some(inp)
    }
}

/// What the caller supplies; everything else comes from `risk.toml` and the store.
pub struct Ports<E, P, Q, B> {
    /// The final step; paper or live differ only here.
    pub executor: E,
    /// Day P&L for the breaker.
    pub pnl: P,
    pub quotes: Q,
    /// Market-side gate inputs (see [`SqliteGateContext`]).
    pub base: B,
    pub clock: Arc<dyn Clock>,
    pub calendar: MarketCalendar,
    pub baseline: Arc<dyn LimitsBaseline + Send + Sync>,
    pub cost: Arc<dyn CostModel>,
}

impl<E, P, Q, B> std::fmt::Debug for Ports<E, P, Q, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ports").finish_non_exhaustive()
    }
}

/// Why the pipeline could not be built.
#[derive(Debug, Error)]
pub enum ComposeError {
    #[error("risk config rejected: {0}")]
    Config(String),
    #[error("guard setup failed: {0}")]
    Guard(#[from] ExecError),
}

/// The pipeline [`build_pipeline`] returns.
pub type BuiltPipeline<E, P, Q, B> = Pipeline<
    GuardedExecutor<E, SqliteStore, P, SqliteStore>,
    RiskGateAdapter<SqliteGateContext<B>>,
    Q,
    SqliteStore,
    SqliteStore,
>;

/// Build the pipeline from `risk.toml` text and an opened store. The caller must run
/// `reload()` (abort on error) and only then `spawn()`.
///
/// # Errors
/// `Config` for a rejected `risk.toml`; `Guard` when the kill switch phrase, rate limits or the
/// breaker (log unreadable, loss limit out of range) cannot be set up.
pub async fn build_pipeline<E, P, Q, B>(
    store: &SqliteStore,
    risk_toml: &str,
    ports: Ports<E, P, Q, B>,
) -> Result<BuiltPipeline<E, P, Q, B>, ComposeError>
where
    E: Executor,
    P: PnlSource,
    Q: QuoteSource,
    B: GateContext,
{
    let risk: RiskConfig = parse_str(risk_toml).map_err(|e| ComposeError::Config(e.to_string()))?;
    let gate_cfg =
        GateConfig::from_toml_str(risk_toml).map_err(|e| ComposeError::Config(e.to_string()))?;
    let ex = &risk.execution;
    let cfg = PipelineConfig::from_section(ex);
    let now = ports.clock.now_ist();

    let kill = KillSwitch::new(
        store.clone(),
        ports.clock.clone(),
        ex.kill_switch_release_phrase.clone(),
        ex.kill_allow_reduce_only,
    )
    .map_err(|_| ExecError::InvalidInput("kill switch release phrase too short"))?;
    let breaker = DailyLossBreaker::new(
        ports.pnl,
        Arc::new(store.clone()),
        ports.calendar,
        risk.limits.max_daily_loss_pct,
        risk.hard_limits.max_daily_loss_pct,
        now,
    )
    .await?;
    let limiter = RateLimiter::new(
        ex.max_orders_per_second,
        risk.limits.max_orders_per_minute,
        risk.limits.max_orders_per_day,
    )?;
    let guard = SafetyGuard::new(kill, breaker, limiter, ex.breaker_engages_kill_switch);
    let guarded = GuardedExecutor::new(ports.executor, guard, ports.clock.clone());
    let ctx = SqliteGateContext::new(
        ports.base,
        store.clone(),
        Handle::current(),
        cfg,
        ports.clock.clone(),
    );
    let gate = Arc::new(RiskGateAdapter::new(
        Arc::new(gate_cfg),
        ports.baseline,
        ports.clock.clone(),
        ports.cost,
        ctx,
        ex,
    ));
    Ok(Pipeline::new(
        guarded,
        gate,
        ports.quotes,
        Arc::new(store.clone()),
        store.clone(),
        cfg,
        ports.clock,
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::HashMap;

    use bazaar_core::ports::{AuditDraft, ChainFn, OrderRecord};
    use bazaar_execution::types::{OrderKind, Product};
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    use super::*;

    /// Orders by state; a state may be made to fail.
    #[derive(Default)]
    struct Orders {
        by_state: HashMap<&'static str, Result<Vec<OrderRecord>, RepoError>>,
    }

    impl OrderRepo for Orders {
        fn insert(
            &self,
            _: OrderRecord,
            _: AuditDraft,
            _: ChainFn,
        ) -> impl Future<Output = Result<(), RepoError>> + Send {
            std::future::ready(Err(RepoError::Unavailable))
        }
        fn get(
            &self,
            _: u128,
        ) -> impl Future<Output = Result<Option<OrderRecord>, RepoError>> + Send {
            std::future::ready(Err(RepoError::Unavailable))
        }
        fn advance(
            &self,
            _: u128,
            _: u32,
            _: String,
            _: String,
            _: AuditDraft,
            _: ChainFn,
        ) -> impl Future<Output = Result<u32, RepoError>> + Send {
            std::future::ready(Err(RepoError::Unavailable))
        }
        fn list_in_state(
            &self,
            state: &str,
            _: u32,
        ) -> impl Future<Output = Result<Vec<OrderRecord>, RepoError>> + Send {
            std::future::ready(
                self.by_state
                    .get(state)
                    .cloned()
                    .unwrap_or_else(|| Ok(vec![])),
            )
        }
    }

    fn now() -> chrono::DateTime<chrono_tz::Tz> {
        Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap()
    }

    fn row(id: u128, symbol: &str, qty: u32, reduce_only: bool) -> OrderRecord {
        let i = OrderIntent {
            id: IntentId(id),
            symbol: symbol.into(),
            side: Side::Sell,
            qty,
            kind: OrderKind::Limit(Decimal::from(50)),
            product: Product::Mis,
            reduce_only,
            created_at: now(),
        };
        OrderRecord {
            intent_id: id,
            version: 1,
            state: "approved".into(),
            payload: i.to_payload(),
        }
    }

    async fn qty(o: &Orders) -> Option<Decimal> {
        pending_exit_qty(o, "ABC", IntentId(1), now()).await
    }

    #[tokio::test]
    async fn sums_only_other_reduce_only_orders_for_the_symbol() {
        let mut o = Orders::default();
        o.by_state.insert(
            "approved",
            Ok(vec![
                row(1, "ABC", 100, true), // the intent itself
                row(2, "ABC", 3, true),
                row(3, "ABC", 50, false), // an entry
                row(4, "XYZ", 70, true),  // another symbol
            ]),
        );
        o.by_state
            .insert("unknown", Ok(vec![row(5, "ABC", 4, true)]));
        assert_eq!(qty(&o).await, Some(Decimal::from(7)));
        assert_eq!(qty(&Orders::default()).await, Some(Decimal::ZERO));
    }

    #[tokio::test]
    async fn a_list_error_a_too_large_list_or_a_bad_payload_is_unknown() {
        for fail in [RepoError::Unavailable, RepoError::TooLarge] {
            let mut o = Orders::default();
            o.by_state.insert("sent", Err(fail));
            assert_eq!(qty(&o).await, None);
        }
        let mut o = Orders::default();
        let mut bad = row(9, "ABC", 1, true);
        bad.payload = "not json".into();
        o.by_state.insert("pending_approval", Ok(vec![bad]));
        assert_eq!(qty(&o).await, None);
    }

    #[test]
    fn unknown_cost_is_unknown() {
        assert_eq!(UnknownCost.cost(Side::Buy, 1, Decimal::ONE), Tri::Unknown);
    }
}
