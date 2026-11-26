//! Production composition (BZ-511 S1): opens the verified store, builds the one pipeline, runs
//! `reload()` once and only then `spawn()`; an `Err` there aborts startup before any bind.
//!
//! Every source here fails closed: no quote source means the approval-time re-gate is `Blocked`
//! for every order (owner decision 2026-11-23); no P&L, charge or instrument-spec source means
//! `Unavailable`/`None`, never a guess. The executor is paper; `live = Known(false)` unless the
//! build has `live-trading`, in which case it is `Unknown` (blocked) until a reconciler exists.

mod driver;
mod quotes;

pub use driver::{ALARM_SQUARE_OFF, Driver, LedgerPnl, STOP_WAIT, serve_while_driven, shutdown};
pub use quotes::GuardedQuotes;

use std::path::Path;
use std::sync::Arc;

use bazaar_cli::compose::{BuiltPipeline, ComposeError, Ports, UnknownCost, build_pipeline};
use bazaar_cli::costs::{F9Charges, GateCost};
use bazaar_cli::specs::InstrumentSnapshot;
use bazaar_core::calendar::MarketCalendar;
use bazaar_core::clock::Clock;
use bazaar_core::config::{ChargesConfig, RiskConfig, parse_str};
use bazaar_core::ports::{
    OrderRepo, PaperLedgerRepo, QuoteSnapshot, QuoteSource, RepoError, SourceError,
};
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_execution::executor::{ApprovedOrder, ExecAck, ExecError, Executor};
use bazaar_execution::gate_adapter::{GateContext, GateInputsOwned};
use bazaar_execution::pipeline::{PipelineHandle, ReloadReport};
use bazaar_execution::types::OrderIntent;
use bazaar_paper::account::{
    ChargeSource, LedgerKind, PaperAccount, RefuseAll, ResetReason, ResetRequest,
};
use bazaar_paper::executor::PaperExecutor;
use bazaar_paper::fill::SimConfig;
use bazaar_paper::squareoff::ValidatedSquareOffTime;
use bazaar_risk::baseline::{BaselineError, LimitsBaseline};
use bazaar_risk::config::Baseline;
use bazaar_risk::gate::CostModel;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::Tri;
use bazaar_store::{KillSwitchRow, SqliteStore};
use rust_decimal::Decimal;
use thiserror::Error;

use crate::state::{
    KillSwitchRead, KillSwitchReading, PendingApproval, PendingFuture, PendingRead, ReadFuture,
};

/// No quote feed is wired: every quote read fails, so the re-gate blocks (fail closed).
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableQuotes;

impl QuoteSource for UnavailableQuotes {
    fn quote(&self, _: &str) -> impl Future<Output = Result<QuoteSnapshot, SourceError>> + Send {
        std::future::ready(Err(SourceError::Unavailable))
    }
}

/// Paper charges: the real F9 model, or `Unavailable` when `charges.toml` could not be turned
/// into one (the account then refuses every fill; it never books a zero or default charge).
#[derive(Debug, Clone)]
pub enum RuntimeCharges {
    Real(F9Charges),
    Unavailable,
}

impl ChargeSource for RuntimeCharges {
    fn charge(
        &self,
        symbol: &str,
        is_buy: bool,
        qty: u32,
        price: Decimal,
        product: &str,
    ) -> Result<Decimal, SourceError> {
        match self {
            Self::Real(c) => c.charge(symbol, is_buy, qty, price, product),
            Self::Unavailable => Err(SourceError::Unavailable),
        }
    }
}

/// The charge and gate-cost adapters from `charges.toml`. Any failure (unreadable, placeholder,
/// parse) logs ONE warning and gives `Unavailable` + `UnknownCost`, so every fill and approve is
/// refused; the server still binds (SEC-150-4, RISK-138-7). No default or zero cost, ever.
/// Known limit (RISK-152-5): `GateCost` has no symbol, so it prices every intent as NSE equity
/// (worst of MIS and CNC); the paper executor refuses non-NSE-EQ symbols through the spec
/// snapshot, and a future executor must only be fed equity intents.
fn cost_adapters(
    config_dir: &Path,
    slippage_bps: u32,
    clock: &Arc<dyn Clock>,
) -> (RuntimeCharges, Arc<dyn CostModel>) {
    let built = bazaar_core::config::load::<ChargesConfig>(config_dir)
        .map_err(|_| ())
        .and_then(|c| {
            let c = Arc::new(c);
            let f9 = F9Charges::new(c.clone(), clock.clone()).map_err(|_| ())?;
            let gate = GateCost::new(c, clock.clone(), slippage_bps).map_err(|_| ())?;
            Ok((f9, gate))
        });
    if let Ok((f9, gate)) = built {
        return (RuntimeCharges::Real(f9), Arc::new(gate));
    }
    // Fixed category only: the parser text can carry file lines and snippets (SEC-152-1).
    tracing::warn!(
        category = "charges_unavailable",
        "charges unavailable: every paper fill and approve is refused"
    );
    (RuntimeCharges::Unavailable, Arc::new(UnknownCost))
}

/// No broker baseline: `Baseline::none()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBaseline;

impl LimitsBaseline for NoBaseline {
    fn read(&self) -> Result<Baseline, BaselineError> {
        Ok(Baseline::none())
    }
}

/// Mode evidence for a production run (RISK-136-5); defined in `bazaar-cli` so the pipeline
/// tests exercise the very same function.
pub use bazaar_cli::compose::prod_mode_evidence;

/// Production market-side gate inputs: nothing is known (no snapshot, data, instrument or stop),
/// so every entry and exit is blocked until real sources replace this. Only the mode evidence
/// is real. `SqliteGateContext` adds the store-owned inputs on top.
#[derive(Debug, Clone, Default)]
pub struct ProdBase;

impl GateContext for ProdBase {
    fn inputs(&self, _: &OrderIntent) -> Option<GateInputsOwned> {
        Some(GateInputsOwned {
            snapshot: PortfolioSnapshot::unknown(),
            data: DataInputs {
                inputs_as_of: Tri::Unknown,
                evidence: EvidenceInputs {
                    oos_trades: Tri::Unknown,
                    profit_factor: Tri::Unknown,
                    expectancy_r: Tri::Unknown,
                    walk_forward_pass: Tri::Unknown,
                },
                ban_list: Tri::Unknown,
                stock_fno_underlying: Tri::Unknown,
                events: EventFlags {
                    results_window: Tri::Unknown,
                    policy_day: Tri::Unknown,
                    option_on_own_expiry: Tri::Unknown,
                },
                quote_at: Tri::Unknown,
                session_open: Tri::Unknown,
                margin: MarginInputs {
                    qty: 0,
                    required: Tri::Unknown,
                    available: Tri::Unknown,
                },
            },
            mode: prod_mode_evidence(),
            entry_zone: Tri::Unknown,
            exposure: None,
            defined_max_loss: None,
            instrument: None,
            stop: None,
            proposed_price: None,
            pending_exit_qty: None,
        })
    }
}

/// Seed of the paper fill simulator (fixed, so paper fills are reproducible).
const PAPER_SIM_SEED: u64 = 0;

/// The paper executor type the runtime uses.
pub type Paper = PaperExecutor<SqliteStore, RuntimeCharges, SqliteStore, InstrumentSnapshot>;

/// Shares the paper executor with the tick driver (S2) while the pipeline owns the sending half.
#[derive(Clone, Debug)]
pub struct SharedPaper(pub Arc<Paper>);

impl Executor for SharedPaper {
    async fn place(&self, order: &ApprovedOrder) -> Result<ExecAck, ExecError> {
        self.0.place(order).await
    }
}

/// The pipeline the runtime builds.
pub type Built<Q, B> = BuiltPipeline<SharedPaper, LedgerPnl, GuardedQuotes<Q>, B>;

/// Why startup stopped; any of these means the server must not bind.
#[derive(Debug, Error)]
pub enum StartError {
    #[error("store: {0}")]
    Store(#[from] RepoError),
    #[error("config: {0}")]
    Config(String),
    #[error("paper setup: {0}")]
    Paper(String),
    #[error("pipeline build: {0}")]
    Compose(#[from] ComposeError),
    #[error("reload failed: {0}")]
    Reload(ExecError),
}

/// A built, reloaded and spawned runtime.
pub struct Runtime {
    pub handle: PipelineHandle,
    pub report: ReloadReport,
    pub kill_switch: Arc<dyn KillSwitchRead>,
    pub paper: Arc<Paper>,
    pub store: SqliteStore,
    /// Orders awaiting a decision (read only).
    pub pending: Arc<dyn PendingRead>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

/// Kill switch, read-only: the persisted state OR the pipeline's in-process latch. Either one
/// engaged means engaged; an unreadable store is engaged and `readable: false`.
struct StoreKillSwitch {
    store: SqliteStore,
    handle: PipelineHandle,
}

impl KillSwitchRead for StoreKillSwitch {
    fn read(&self) -> ReadFuture<'_> {
        Box::pin(async move {
            // Read the latch first: an engage that lands during the store read is not missed.
            let latched = self.handle.kill_switch_latched();
            // One row for state and reason, so the two cannot disagree.
            stored_reading(self.store.kill_switch_get().await, latched)
        })
    }
}

/// The reason is given only when the stored switch is engaged: a released row holds the release
/// text, which must not be shown as why a latched switch is engaged. Unreadable: no reason.
fn stored_reading(row: Result<KillSwitchRow, RepoError>, latched: bool) -> KillSwitchReading {
    match row {
        Ok(row) => KillSwitchReading {
            engaged: row.engaged || latched,
            reason: row.engaged.then_some(row.reason),
            readable: true,
            latched,
        },
        Err(_) => KillSwitchReading {
            engaged: true,
            reason: None,
            readable: false,
            latched,
        },
    }
}

/// Most pending orders listed; more is an unreadable list (503), never a silent cut.
const PENDING_LIMIT: u32 = 500;

/// Pending orders from the store, with their approval expiry.
pub struct StorePending {
    /// The verified store.
    pub store: SqliteStore,
    /// Time source for decoding payloads.
    pub clock: Arc<dyn Clock>,
    /// Approval TTL from `risk.toml`.
    pub ttl: chrono::Duration,
}

impl std::fmt::Debug for StorePending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorePending").finish_non_exhaustive()
    }
}

impl PendingRead for StorePending {
    fn list(&self) -> PendingFuture<'_> {
        Box::pin(async move {
            let state = bazaar_execution::state::OrderState::PendingApproval.as_str();
            let Ok(rows) = OrderRepo::list_in_state(&self.store, state, PENDING_LIMIT).await else {
                tracing::warn!(category = "store_read", "pending approvals unreadable");
                return None;
            };
            let now = self.clock.now_ist();
            let mut out = Vec::with_capacity(rows.len());
            for r in rows {
                let id = bazaar_execution::types::IntentId(r.intent_id);
                // An undecodable row is not listed: reload expires it and audits that.
                let Some(i) = OrderIntent::from_payload(id, &r.payload, now) else {
                    tracing::warn!(category = "undecodable_row", "pending order skipped");
                    continue;
                };
                out.push(PendingApproval {
                    id: r.intent_id,
                    symbol: i.symbol,
                    side: match i.side {
                        bazaar_risk::types::Side::Buy => "buy",
                        bazaar_risk::types::Side::Sell => "sell",
                    },
                    qty: i.qty,
                    created_at: i.created_at.to_rfc3339(),
                    // Counted from creation: never later than the live path, which counts from
                    // queue time (CR-144-4, RISK-144-5).
                    expires_at: (i.created_at + self.ttl).to_rfc3339(),
                });
            }
            Some(out)
        })
    }
}

/// Open the verified store at `db`, build the pipeline, `reload()` once, then `spawn()`.
///
/// # Errors
/// Any [`StartError`]; the caller must exit non-zero and not bind (RISK-133-3).
pub async fn start(
    db: &Path,
    risk_toml: &str,
    config_dir: &Path,
    clock: Arc<dyn Clock>,
    calendar: MarketCalendar,
) -> Result<Runtime, StartError> {
    open_with(
        db,
        risk_toml,
        config_dir,
        clock,
        calendar,
        UnavailableQuotes,
        ProdBase,
    )
    .await
}

/// Test seam: the production wiring with a chosen quote source and gate base. The quote source
/// still goes behind `GuardedQuotes`. Compiled only for tests, so `main.rs` cannot reach it.
#[cfg(test)]
async fn start_with<Q: QuoteSource + 'static, B: GateContext + 'static>(
    db: &Path,
    risk_toml: &str,
    config_dir: &Path,
    clock: Arc<dyn Clock>,
    calendar: MarketCalendar,
    quotes: Q,
    base: B,
) -> Result<Runtime, StartError> {
    open_with(db, risk_toml, config_dir, clock, calendar, quotes, base).await
}

async fn open_with<Q: QuoteSource + 'static, B: GateContext + 'static>(
    db: &Path,
    risk_toml: &str,
    config_dir: &Path,
    clock: Arc<dyn Clock>,
    calendar: MarketCalendar,
    quotes: Q,
    base: B,
) -> Result<Runtime, StartError> {
    let store = bazaar_cli::compose::open_store(db).await?;
    let risk: RiskConfig = parse_str(risk_toml).map_err(|e| StartError::Config(e.to_string()))?;
    let pending = Arc::new(StorePending {
        store: store.clone(),
        clock: clock.clone(),
        ttl: chrono::Duration::seconds(i64::from(risk.execution.approval_ttl_secs)),
    });
    let (mut pipe, paper) =
        build(&store, risk_toml, config_dir, clock, calendar, quotes, base).await?;
    let report = pipe.reload().await.map_err(StartError::Reload)?;
    let handle = pipe.spawn();
    Ok(Runtime {
        kill_switch: Arc::new(StoreKillSwitch {
            store: store.clone(),
            handle: handle.clone(),
        }),
        handle,
        report,
        paper,
        store,
        pending,
    })
}

/// RISK-136-4: ledger lines carry no symbol, quantity or product, so the paper position book
/// cannot be rebuilt from them. Fail closed: any buy or sell line in the current epoch may be an
/// open lot, so startup refuses to serve until the account is reset (new epoch). An unreadable
/// or corrupt ledger refuses too.
async fn ensure_no_open_lots(store: &SqliteStore) -> Result<(), StartError> {
    let lines = PaperLedgerRepo::current_epoch(store)
        .await
        .map_err(|e| StartError::Paper(format!("ledger unreadable: {e}")))?;
    for l in &lines {
        match LedgerKind::parse(&l.kind) {
            Some(LedgerKind::Opening | LedgerKind::Charge) => {}
            Some(LedgerKind::Buy | LedgerKind::Sell) => {
                return Err(StartError::Paper(
                    "open paper lots may exist and cannot be restored from the ledger; reset the paper account"
                        .into(),
                ));
            }
            None => return Err(StartError::Paper("ledger has an unknown line kind".into())),
        }
    }
    Ok(())
}

async fn build<Q: QuoteSource + 'static, B: GateContext + 'static>(
    store: &SqliteStore,
    risk_toml: &str,
    config_dir: &Path,
    clock: Arc<dyn Clock>,
    calendar: MarketCalendar,
    quotes: Q,
    base: B,
) -> Result<(Built<Q, B>, Arc<Paper>), StartError> {
    let risk: RiskConfig = parse_str(risk_toml).map_err(|e| StartError::Config(e.to_string()))?;
    let ex = &risk.execution;
    let (charges, cost) = cost_adapters(config_dir, ex.paper_slippage_bps.equity, &clock);
    let (specs, warnings) = InstrumentSnapshot::load(store, config_dir, &*clock).await;
    if !warnings.is_empty() {
        // Fixed category and a count; the warning texts are not echoed (SEC-152-2).
        tracing::warn!(
            category = "instrument_master",
            count = warnings.len(),
            "instrument master warnings: symbols without specs are refused"
        );
    }
    let account = Arc::new(PaperAccount::new(
        store.clone(),
        charges,
        clock.clone(),
        ex.paper_starting_cash_inr,
    ));
    let opened = account
        .balances()
        .await
        .map_err(|e| StartError::Paper(e.to_string()))?;
    if opened.epoch.is_none() {
        // First open only: the guard is never consulted for `Initial`. `RefuseAll` stays the
        // only production guard through slice 9: the real guard is built in slice 9e and wired
        // in BZ-F10-S10. Do not replace it before then.
        let req = ResetRequest {
            reason: ResetReason::Initial,
            actor: "system.startup".to_owned(),
        };
        account
            .reset(&req, &RefuseAll)
            .await
            .map_err(|e| StartError::Paper(e.to_string()))?;
    }
    ensure_no_open_lots(store).await?;
    let cal = Arc::new(calendar.clone());
    let segs = [(Segment::Equity, InstrumentKind::CashNonFno)];
    // RISK-138-5: validate on the latest trading day (today when it is one), so a start on a
    // weekend or holiday still works and the kill-switch route stays reachable. Covered:
    // Equity / CashNonFno only (the segments the paper executor accepts today).
    let today = clock.now_ist().date_naive();
    let on = if cal
        .is_trading_day(today, Segment::Equity)
        .map_err(|e| StartError::Paper(e.to_string()))?
    {
        today
    } else {
        cal.prev_trading_day(today, Segment::Equity)
            .map_err(|e| StartError::Paper(e.to_string()))?
    };
    let square_off = ValidatedSquareOffTime::new(&cal, on, ex.paper_mis_square_off_ist, &segs)
        .map_err(|e| StartError::Paper(e.to_string()))?;
    let paper = Arc::new(PaperExecutor::new(
        account,
        store.clone(),
        specs,
        clock.clone(),
        cal,
        SimConfig::from_section(ex, PAPER_SIM_SEED),
        square_off,
    ));
    let ports = Ports {
        executor: SharedPaper(paper.clone()),
        pnl: LedgerPnl::new(
            store.clone(),
            {
                let p = paper.clone();
                Arc::new(move || p.has_open_positions())
            },
            clock.clone(),
        ),
        quotes: GuardedQuotes::new(
            quotes,
            clock.clone(),
            ex.io_timeout(),
            risk.data.max_quote_age_secs,
        ),
        base,
        clock,
        calendar,
        baseline: Arc::new(NoBaseline),
        cost,
    };
    Ok((build_pipeline(store, risk_toml, ports).await?, paper))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unused_async,
    clippy::unused_async_trait_impl
)]
mod tests {
    use bazaar_core::config::{CalendarConfig, load};
    use bazaar_core::ports::{ApprovalDecision, ApprovalRepo, AuditDraft, OrderRepo};
    use bazaar_core::testutil::FakeClock;
    use bazaar_execution::approval::Outcome;
    use bazaar_execution::audit::chain_hash;
    use bazaar_execution::types::{IntentId, OrderKind, Product};
    use bazaar_risk::types::Side;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;

    use super::*;

    const RISK: &str = include_str!("../../../fixtures/config/risk.toml");

    fn clock() -> Arc<FakeClock> {
        Arc::new(FakeClock::at(
            Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap(),
        ))
    }

    fn cfg() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")
    }

    fn calendar() -> MarketCalendar {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config");
        MarketCalendar::from_config(&load::<CalendarConfig>(Path::new(dir)).unwrap())
    }

    fn intent(c: &FakeClock, id: u128, side: Side, reduce_only: bool) -> OrderIntent {
        OrderIntent {
            id: IntentId(id),
            symbol: "ABC".into(),
            side,
            qty: 1,
            kind: OrderKind::Limit(Decimal::from(50)),
            product: Product::Mis,
            reduce_only,
            created_at: c.now_ist(),
        }
    }

    fn blocked(o: &Outcome) -> bool {
        matches!(o, Outcome::GateBlocked(_))
    }

    /// The gate's refusal text; fails the test for any other outcome.
    fn reason(o: &Outcome) -> &str {
        match o {
            Outcome::GateBlocked(r) => r,
            other => panic!("expected GateBlocked, got {other:?}"),
        }
    }

    #[test]
    fn mode_evidence_keeps_auto_locked_and_live_false() {
        let m = prod_mode_evidence();
        assert_eq!(m.approval, None);
        assert_eq!(m.compliance_fresh, Tri::Known(false));
        assert_eq!(m.readiness_report, Tri::Known(false));
        assert_eq!(m.owner_opt_in, Tri::Known(false));
        if cfg!(feature = "live-trading") {
            assert_eq!(m.live, Tri::Unknown);
        } else {
            assert_eq!(m.live, Tri::Known(false));
        }
        let base = ProdBase
            .inputs(&intent(&clock(), 1, Side::Buy, false))
            .unwrap();
        assert_eq!(base.mode, m);
        assert!(base.instrument.is_none() && base.proposed_price.is_none());
    }

    #[tokio::test]
    async fn no_quote_source_means_the_approve_is_blocked_through_the_real_gate_context() {
        let dir = tempfile::tempdir().unwrap();
        let c = clock();
        let rt = start(
            &dir.path().join("x.db"),
            RISK,
            &cfg(),
            c.clone(),
            calendar(),
        )
        .await
        .unwrap();
        assert_eq!(rt.report, ReloadReport::default());
        // Submit may already block (nothing known); either way nothing is ever Placed.
        let out = rt
            .handle
            .submit(intent(&c, 1, Side::Buy, false))
            .await
            .unwrap();
        if out == Outcome::Queued {
            c.advance(chrono::Duration::seconds(2));
            let out = rt
                .handle
                .decide(IntentId(1), ApprovalDecision::Approve, "owner")
                .await
                .unwrap();
            assert!(blocked(&out), "{out:?}");
        } else {
            assert!(blocked(&out), "{out:?}");
        }
        assert_eq!(rt.paper.working_count(), 0);
    }

    #[tokio::test]
    async fn a_failed_start_returns_err_so_no_route_is_ever_served() {
        let dir = tempfile::tempdir().unwrap();
        // Bad config.
        let e = start(
            &dir.path().join("a.db"),
            "nonsense = 1",
            &cfg(),
            clock(),
            calendar(),
        )
        .await;
        assert!(matches!(e, Err(StartError::Config(_))), "{e:?}");
        // The store path is a directory: the verified open fails.
        let e = start(dir.path(), RISK, &cfg(), clock(), calendar()).await;
        assert!(matches!(e, Err(StartError::Store(_))), "{e:?}");
    }

    /// AC4 / RISK-133-3: a failing `reload` makes `start` return `StartError::Reload`: no
    /// `Runtime` (so no handle, no state, nothing for `main` to bind) and no stored order touched.
    /// The failure is real: more `approved` rows than the reload limit is a storage error.
    #[tokio::test]
    async fn a_failing_reload_aborts_start_with_reload_error_and_serves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("x.db");
        let c = clock();
        let store = bazaar_cli::compose::open_store(&db).await.unwrap();
        let payload = intent(&c, 1, Side::Buy, false).to_payload();
        // One over the limit (1000) in one state.
        for id in 1..=1001_u128 {
            OrderRepo::insert(
                &store,
                bazaar_core::ports::OrderRecord {
                    intent_id: id,
                    version: 0,
                    state: "approved".into(),
                    payload: payload.clone(),
                },
                AuditDraft {
                    ts_ist_rfc3339: c.now_ist().to_rfc3339(),
                    actor: "owner".into(),
                    kind: "seed".into(),
                    payload: "{}".into(),
                },
                chain_hash,
            )
            .await
            .unwrap();
        }
        store.anchor_flush().await.unwrap();
        drop(store);
        let e = start(&db, RISK, &cfg(), c.clone(), calendar()).await;
        assert!(
            matches!(e, Err(StartError::Reload(ExecError::Storage))),
            "{e:?}"
        );
        // Nothing was half-processed: every row is still `approved`, untouched.
        let store = bazaar_cli::compose::open_store(&db).await.unwrap();
        let rows = OrderRepo::list_in_state(&store, "approved", 2000)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1001);
        assert!(rows.iter().all(|r| r.version == 0));
    }

    /// `main` must open (and reload) the runtime strictly before it binds the port.
    #[test]
    fn main_starts_the_runtime_before_binding_the_listener() {
        let src = include_str!("main.rs");
        let start = src.find("= runtime::start(").unwrap();
        let bind = src.find("TcpListener::bind(").unwrap();
        assert!(start < bind, "bind must come after a successful start");
    }

    /// RISK-175-1/2 wiring pin (behaviour is tested on `shutdown` in driver.rs): `main` watches
    /// the store's anchor, hands the flush to `shutdown`, and returns the serve result only after.
    #[test]
    fn main_wires_the_anchor_and_routes_shutdown_through_the_tested_fn() {
        let src = include_str!("main.rs");
        assert!(src.contains(".with_anchor(rt.store.clone())"));
        let serve = src.find("serve_while_driven(").unwrap();
        let shut = src.find("runtime::shutdown(").unwrap();
        let flush = src.find("rt.store.anchor_flush()").unwrap();
        let ret = src.find("served??;").unwrap();
        assert!(serve < shut && shut < flush && flush < ret);
        assert!(!src.contains("runtime::stop("), "stop only via shutdown");
        assert!(
            !src[serve..shut].contains("?;"),
            "no early `?` before shutdown"
        );
    }

    #[tokio::test]
    async fn an_approved_unsent_order_is_blocked_at_restart_and_never_sent() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("x.db");
        let c = clock();
        let store = bazaar_cli::compose::open_store(&db).await.unwrap();
        let i = intent(&c, 7, Side::Buy, false);
        let draft = |kind: &str| AuditDraft {
            ts_ist_rfc3339: c.now_ist().to_rfc3339(),
            actor: "owner".into(),
            kind: kind.into(),
            payload: "{}".into(),
        };
        OrderRepo::insert(
            &store,
            bazaar_core::ports::OrderRecord {
                intent_id: 7,
                version: 0,
                state: "pending_approval".into(),
                payload: i.to_payload(),
            },
            draft("proposed"),
            chain_hash,
        )
        .await
        .unwrap();
        ApprovalRepo::decide_and_advance(
            &store,
            7,
            ApprovalDecision::Approve,
            "owner",
            0,
            "approved".into(),
            "approve".into(),
            draft("approve"),
            chain_hash,
        )
        .await
        .unwrap();
        drop(store);
        c.advance(chrono::Duration::seconds(5));
        let rt = start(&db, RISK, &cfg(), c, calendar()).await.unwrap();
        assert_eq!(
            (rt.report.blocked, rt.report.resumed),
            (1, 0),
            "{:?}",
            rt.report
        );
        let row = OrderRepo::get(&rt.store, 7).await.unwrap().unwrap();
        assert_eq!(row.state, "blocked");
        assert_eq!(rt.paper.working_count(), 0);
    }

    #[tokio::test]
    async fn a_non_exit_is_refused_while_the_kill_switch_is_engaged() {
        let dir = tempfile::tempdir().unwrap();
        let c = clock();
        let rt = start(
            &dir.path().join("x.db"),
            RISK,
            &cfg(),
            c.clone(),
            calendar(),
        )
        .await
        .unwrap();
        assert!(!rt.kill_switch.read().await.engaged);
        rt.handle
            .set_kill_switch(true, "owner", "stop")
            .await
            .unwrap();
        let r = rt.kill_switch.read().await;
        assert!(r.engaged && r.readable);
        let out = rt
            .handle
            .submit(intent(&c, 1, Side::Buy, false))
            .await
            .unwrap();
        // The reason text is pinned in the cli pipeline tests, where the quote is known; here
        // (no quote source) the block is "no fresh quote" and the store state is what matters.
        assert!(blocked(&out), "{out:?}");
        assert_eq!(rt.paper.working_count(), 0);
    }

    /// Unreadable kill-switch state is reported unreadable and orders stay blocked.
    #[tokio::test]
    async fn an_unreadable_kill_switch_state_is_reported_unreadable_and_blocks() {
        use sqlx::{ConnectOptions, Connection, sqlite::SqliteConnectOptions};
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("x.db");
        let c = clock();
        let rt = start(&db, RISK, &cfg(), c.clone(), calendar())
            .await
            .unwrap();
        let mut conn = SqliteConnectOptions::new()
            .filename(&db)
            .connect()
            .await
            .unwrap();
        sqlx::query("DROP TABLE kill_switch")
            .execute(&mut conn)
            .await
            .unwrap();
        conn.close().await.unwrap();
        assert!(!rt.kill_switch.read().await.readable);
        let out = rt
            .handle
            .submit(intent(&c, 1, Side::Buy, false))
            .await
            .unwrap();
        assert!(blocked(&out), "{out:?}");
    }

    /// RISK-138-5: a start on a Saturday (or holiday) still succeeds, so the kill-switch route
    /// is reachable.
    #[tokio::test]
    async fn start_succeeds_on_a_weekend() {
        let dir = tempfile::tempdir().unwrap();
        let sat = Arc::new(FakeClock::at(
            Kolkata.with_ymd_and_hms(2026, 11, 21, 11, 0, 0).unwrap(),
        ));
        let rt = start(&dir.path().join("x.db"), RISK, &cfg(), sat, calendar()).await;
        assert!(rt.is_ok(), "{:?}", rt.err());
    }

    /// TS-176-4: the only production reset. Its audit row is pinned field by field, and a
    /// restart adds neither a second row nor a second opening line.
    #[tokio::test]
    async fn first_open_is_audited_once_and_a_restart_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("x.db");
        for run in ["first open", "restart"] {
            let rt = start(&db, RISK, &cfg(), clock(), calendar()).await.unwrap();
            let rows = AuditRepo::read_all(&rt.store).await.unwrap();
            assert_eq!(rows[0].draft.kind, "paper_reset", "{run}");
            let resets = rows.iter().filter(|r| r.draft.kind == "paper_reset");
            assert_eq!(resets.count(), 1, "{run}");
            assert_eq!(rows[0].draft.actor, "system.startup", "{run}");
            assert_eq!(
                rows[0].draft.payload,
                r#"{"actor":"system.startup","epoch":1,"opening":"250000","reason":"initial"}"#,
                "{run}"
            );
            let lines = PaperLedgerRepo::current_epoch(&rt.store).await.unwrap();
            assert_eq!(lines.len(), 1, "{run}");
            assert_eq!(
                (lines[0].epoch, lines[0].kind.as_str(), lines[0].amount),
                (1, "opening", Decimal::from(250_000)),
                "{run}"
            );
        }
    }

    /// RISK-136-4: a ledger with fills cannot restore the position book, so startup refuses.
    #[tokio::test]
    async fn reopen_refuses_while_the_ledger_may_hold_open_lots() {
        use bazaar_core::ports::LedgerEntry;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("x.db");
        let c = clock();
        // A flat ledger (opening only) reopens fine.
        drop(
            start(&db, RISK, &cfg(), c.clone(), calendar())
                .await
                .unwrap(),
        );
        drop(
            start(&db, RISK, &cfg(), c.clone(), calendar())
                .await
                .unwrap(),
        );
        let store = bazaar_cli::compose::open_store(&db).await.unwrap();
        let epoch = PaperLedgerRepo::current_epoch(&store).await.unwrap()[0].epoch;
        PaperLedgerRepo::append(
            &store,
            LedgerEntry {
                epoch,
                kind: "buy".into(),
                amount: Decimal::from(-500),
                ref_id: Some(9),
                ts_ist_rfc3339: c.now_ist().to_rfc3339(),
            },
        )
        .await
        .unwrap();
        drop(store);
        let e = start(&db, RISK, &cfg(), c, calendar()).await;
        match e {
            Err(StartError::Paper(m)) => assert!(m.contains("open paper lots may exist"), "{m}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// RISK-142-7: charge lines alone are fine; an unknown line kind is refused.
    #[tokio::test]
    async fn reopen_accepts_charge_lines_and_refuses_an_unknown_kind() {
        use bazaar_core::ports::LedgerEntry;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("x.db");
        let c = clock();
        drop(
            start(&db, RISK, &cfg(), c.clone(), calendar())
                .await
                .unwrap(),
        );
        let store = bazaar_cli::compose::open_store(&db).await.unwrap();
        let epoch = PaperLedgerRepo::current_epoch(&store).await.unwrap()[0].epoch;
        let add = |kind: &str| LedgerEntry {
            epoch,
            kind: kind.into(),
            amount: Decimal::from(-1),
            ref_id: Some(3),
            ts_ist_rfc3339: c.now_ist().to_rfc3339(),
        };
        PaperLedgerRepo::append(&store, add("charge"))
            .await
            .unwrap();
        assert!(ensure_no_open_lots(&store).await.is_ok());
        PaperLedgerRepo::append(&store, add("mystery"))
            .await
            .unwrap();
        match ensure_no_open_lots(&store).await {
            Err(StartError::Paper(m)) => assert_eq!(m, "ledger has an unknown line kind"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_real_risk_toml_is_paper_mode() {
        let r = parse_str::<RiskConfig>(include_str!("../../../config/risk.toml")).unwrap();
        assert_eq!(
            r.mode.execution_mode,
            bazaar_core::mode::ExecutionMode::Paper
        );
    }

    /// SEC-136-1: the store-backed context blocks on the store, so it works inside
    /// `spawn_blocking` (as the pipeline calls it) and panics loudly on an async worker.
    #[tokio::test]
    async fn gate_context_must_run_inside_spawn_blocking() {
        use bazaar_cli::compose::SqliteGateContext;
        use bazaar_execution::pipeline::PipelineConfig;
        let dir = tempfile::tempdir().unwrap();
        let store = bazaar_cli::compose::open_store(&dir.path().join("g.db"))
            .await
            .unwrap();
        let cfg = PipelineConfig::from_section(&parse_str::<RiskConfig>(RISK).unwrap().execution);
        let c = clock();
        let ctx = Arc::new(SqliteGateContext::new(
            ProdBase,
            store,
            tokio::runtime::Handle::current(),
            cfg,
            c.clone(),
        ));
        let i = intent(&c, 1, Side::Buy, false);
        let (c2, i2) = (ctx.clone(), i.clone());
        let got = tokio::task::spawn_blocking(move || c2.inputs(&i2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.snapshot.kill_switch, Tri::Known(false));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctx.inputs(&i)));
        assert!(r.is_err(), "must panic on an async worker");
    }

    // ---- BZ-521 S2 wiring: real charges and specs, guarded quotes, the test seam -------------

    use std::collections::BTreeMap;

    use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
    use bazaar_core::ports::InstrumentRepo;
    use bazaar_paper::fill::Tick;
    use bazaar_risk::rules::behaviour::EntryZone;
    use bazaar_risk::rules::exposure::ExposureInputs;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    /// Passes the gate for a flat paper account (the cli pipeline test's fixture, TESTEQ).
    #[derive(Clone)]
    struct OpenBase(Arc<FakeClock>);
    impl GateContext for OpenBase {
        fn inputs(&self, i: &OrderIntent) -> Option<GateInputsOwned> {
            let now = self.0.now_ist();
            let mut s = PortfolioSnapshot::unknown();
            s.as_of = Tri::Known(now);
            s.positions = Tri::Known(vec![]);
            s.consecutive_losses = Tri::Known(0);
            s.day_pnl = Tri::Known(Decimal::ZERO);
            s.week_pnl = Tri::Known(Decimal::ZERO);
            s.week_start = Tri::Known(chrono::NaiveDate::from_ymd_opt(2026, 11, 30).unwrap());
            for f in [
                &mut s.entries_today,
                &mut s.orders_last_minute,
                &mut s.orders_today,
                &mut s.overrides_today,
                &mut s.overrides_week,
            ] {
                *f = Tri::Known(0);
            }
            s.kill_switch = Tri::Known(false);
            s.charges_buffer = Tri::Known(d("20"));
            s.starting_cash = Tri::Known(d("250000"));
            s.realised_pnl = Tri::Known(Decimal::ZERO);
            s.equity_mtm = Tri::Known(d("250000"));
            s.marks = Tri::Known(vec![]);
            s.last_losing_exits = Tri::Known(vec![]);
            let end = now.date_naive();
            let series = (0..100u32)
                .map(|i| {
                    (
                        end - chrono::Duration::days(i64::from(99 - i)),
                        Decimal::new(i64::from(i % 7) - 3, 3),
                    )
                })
                .collect();
            let mut g = ProdBase.inputs(&intent(&self.0, 0, Side::Buy, false))?;
            g.snapshot = s;
            g.data.inputs_as_of = Tri::Known(now);
            g.data.evidence = EvidenceInputs {
                oos_trades: Tri::Known(100),
                profit_factor: Tri::Known(d("2")),
                expectancy_r: Tri::Known(d("0.3")),
                walk_forward_pass: Tri::Known(true),
            };
            g.data.stock_fno_underlying = Tri::Known(None);
            g.data.events = EventFlags {
                results_window: Tri::Known(false),
                policy_day: Tri::Known(false),
                option_on_own_expiry: Tri::Known(false),
            };
            g.data.session_open = Tri::Known(true);
            g.data.margin = MarginInputs {
                qty: 10,
                required: Tri::Known(d("5000")),
                available: Tri::Known(d("250000")),
            };
            g.entry_zone = Tri::Known(EntryZone {
                low: d("1930"),
                high: d("1960"),
            });
            g.exposure = Some(ExposureInputs {
                sectors: BTreeMap::from([(i.symbol.clone(), "IT".to_string())]),
                returns: BTreeMap::from([(i.symbol.clone(), series)]),
            });
            g.instrument = Some(Instrument {
                tradingsymbol: i.symbol.clone(),
                ..master_row()
            });
            g.stop = Some(d("1940"));
            g.proposed_price = Some(d("1950"));
            g.pending_exit_qty = Some(Decimal::ZERO);
            Some(g)
        }
    }

    /// The fixture calendar closes at 14:00, so the paper square-off moves to 13:30. The sizing
    /// floor on the charges allowance is lowered to Rs 1: the real cost of ONE share is far
    /// below the Rs 15 fixture floor (see the F10 record, BZ-521 S2 note).
    fn fixture_risk() -> String {
        let out = RISK.replace("15:15", "13:30").replace(
            "min_charges_slippage_buffer = \"15\"",
            "min_charges_slippage_buffer = \"1\"",
        );
        assert!(out.contains("13:30"), "square-off replace was a no-op");
        assert!(
            out.contains("min_charges_slippage_buffer = \"1\""),
            "floor replace was a no-op"
        );
        out
    }

    fn master_row() -> Instrument {
        Instrument {
            instrument_token: 1001,
            exchange_token: 4,
            tradingsymbol: "TESTEQ".into(),
            name: "TEST EQUITY".into(),
            exchange: Exchange::Nse,
            instrument_type: InstrumentType::Equity,
            segment: "NSE".into(),
            expiry: None,
            strike: None,
            tick_size: d("0.05"),
            lot_size: 1,
        }
    }

    /// A quote at 1950, stamped with the fake clock's now.
    struct GoodQuote(Arc<FakeClock>);
    impl QuoteSource for GoodQuote {
        async fn quote(&self, _: &str) -> Result<QuoteSnapshot, SourceError> {
            Ok(QuoteSnapshot {
                last: d("1950"),
                lower_circuit: None,
                upper_circuit: None,
                bar_volume: None,
                at_ist_rfc3339: self.0.now_ist().to_rfc3339(),
            })
        }
    }

    struct Env {
        _data: tempfile::TempDir,
        cfg: tempfile::TempDir,
        db: std::path::PathBuf,
        clock: Arc<FakeClock>,
    }

    /// Temp data dir whose master (fetched today) holds TESTEQ, and a temp config dir with the
    /// fixture calendar and charges and a watchlist of `symbols`.
    async fn env(symbols: &[&str]) -> Env {
        env_aged(symbols, 0).await
    }

    /// As `env`, with the master fetched `master_age_days` days ago. Also holds a BSE equity and
    /// an NSE future on the master, which the paper universe must refuse.
    async fn env_aged(symbols: &[&str], master_age_days: i64) -> Env {
        let data = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/config");
        std::fs::copy(fx.join("calendar.toml"), cfg.path().join("calendar.toml")).unwrap();
        // The cli charges fixture: equity rows for both products (the config fixture has fewer).
        let charges =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../cli/tests/fixtures/charges.toml");
        std::fs::copy(charges, cfg.path().join("charges.toml")).unwrap();
        let list = symbols
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(",");
        std::fs::write(
            cfg.path().join("watchlists.toml"),
            format!(
                "schema_version = 1\nverified_on = \"2026-01-05\"\nsource = \"test\"\n[[watchlist]]\nname = \"w\"\nsymbols = [{list}]\n"
            ),
        )
        .unwrap();
        let clock = Arc::new(FakeClock::at(
            Kolkata.with_ymd_and_hms(2026, 12, 1, 10, 30, 0).unwrap(),
        ));
        let db = data.path().join("x.db");
        let store = bazaar_cli::compose::open_store(&db).await.unwrap();
        let bse = Instrument {
            instrument_token: 1002,
            exchange_token: 5,
            tradingsymbol: "TESTBSE".into(),
            exchange: Exchange::Bse,
            ..master_row()
        };
        let fut = Instrument {
            instrument_token: 1003,
            exchange_token: 6,
            tradingsymbol: "TESTFUT".into(),
            exchange: Exchange::Nfo,
            instrument_type: InstrumentType::Future,
            segment: "NFO-FUT".into(),
            ..master_row()
        };
        let fetched = clock.now_ist().date_naive() - chrono::Duration::days(master_age_days);
        store
            .upsert_instruments(&[master_row(), bse, fut], fetched)
            .await
            .unwrap();
        drop(store);
        Env {
            _data: data,
            cfg,
            db,
            clock,
        }
    }

    impl Env {
        fn calendar(&self) -> MarketCalendar {
            MarketCalendar::from_config(&load::<CalendarConfig>(self.cfg.path()).unwrap())
        }

        async fn open(&self) -> Runtime {
            start_with(
                &self.db,
                &fixture_risk(),
                self.cfg.path(),
                self.clock.clone(),
                self.calendar(),
                GoodQuote(self.clock.clone()),
                OpenBase(self.clock.clone()),
            )
            .await
            .unwrap()
        }

        fn order(&self, symbol: &str) -> OrderIntent {
            OrderIntent {
                symbol: symbol.into(),
                qty: 10,
                kind: bazaar_execution::types::OrderKind::Limit(d("1950")),
                ..intent(&self.clock, 1, Side::Buy, false)
            }
        }

        /// Submit then approve one order; returns the approve outcome.
        async fn approve(&self, rt: &Runtime, symbol: &str) -> Outcome {
            let out = rt.handle.submit(self.order(symbol)).await.unwrap();
            assert_eq!(out, Outcome::Queued, "{out:?}");
            self.clock.advance(chrono::Duration::seconds(2));
            rt.handle
                .decide(IntentId(1), ApprovalDecision::Approve, "owner")
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn a_paper_fill_books_the_real_f9_charge_through_the_seam() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let out = e.approve(&rt, "TESTEQ").await;
        assert!(matches!(out, Outcome::Placed(_)), "{out:?}");
        assert_eq!(rt.paper.working_count(), 1);
        // A limit buy fills only when the price trades through the limit.
        let mut t = bazaar_paper::executor::TickOutcome::default();
        for price in ["1950", "1945"] {
            e.clock.advance(chrono::Duration::seconds(1));
            t = rt
                .paper
                .on_tick(&Tick {
                    symbol: "TESTEQ".into(),
                    price: d(price),
                    volume: 1_000_000,
                    lower_circuit: None,
                    upper_circuit: None,
                    at: e.clock.now_ist(),
                })
                .await;
        }
        assert!(t.rejected.is_empty(), "{:?}", t.rejected);
        assert_eq!(t.fills.len(), 1, "{:?}", t.fills);
        let fill = &t.fills[0];
        assert_eq!((fill.qty, fill.price), (10, d("1950")));
        // Hand-derived from the cli fixture charges.toml (NSE equity_intraday buy, turnover
        // 10 x FILLPRICE): brokerage = lower of Rs 20 and 0.03%; STT 0; exchange 0.00307%; SEBI
        // Rs 10 per crore; IPFT 0.0000001%; stamp 0.003%; GST 18% on brokerage + SEBI + exchange. Lines round half-up: 5.85 + 0.60 + 0.02 + 0 + stamp 1 (0.585 to whole rupees) + GST 1.16 = 8.63.
        let want = d("8.63");
        let lines = PaperLedgerRepo::current_epoch(&rt.store).await.unwrap();
        let charge: Vec<_> = lines.iter().filter(|l| l.kind == "charge").collect();
        assert_eq!(charge.len(), 1, "{lines:?}");
        assert_eq!(charge[0].amount, -want);
        let buy: Vec<_> = lines.iter().filter(|l| l.kind == "buy").collect();
        assert_eq!(buy.len(), 1, "{lines:?}");
        assert_eq!(buy[0].amount, -d("19500"));
        // Cash after the fill: the opening cash less the buy and the charge, nothing else moved.
        assert_eq!(lines.len(), 3, "{lines:?}");
        let opening: Decimal = lines
            .iter()
            .filter(|l| l.kind == "opening")
            .map(|l| l.amount)
            .sum();
        let total: Decimal = lines.iter().map(|l| l.amount).sum();
        assert_eq!(total, opening - d("19500") - want);
        let pos = rt
            .paper
            .position("TESTEQ", bazaar_execution::types::Product::Mis);
        assert_eq!(pos, 10);
    }

    #[tokio::test]
    async fn a_passing_quote_source_alone_does_not_open_production() {
        // The production base with a perfect quote source: still Blocked, never placed. The
        // refusal is the Unknown gate inputs (no proposed price), not a quote problem.
        let e = env(&["TESTEQ"]).await;
        let rt = start_with(
            &e.db,
            &fixture_risk(),
            e.cfg.path(),
            e.clock.clone(),
            e.calendar(),
            GoodQuote(e.clock.clone()),
            ProdBase,
        )
        .await
        .unwrap();
        let out = rt.handle.submit(e.order("TESTEQ")).await.unwrap();
        assert_eq!(
            reason(&out),
            "no proposed price to measure quote drift from"
        );
        assert_eq!(rt.paper.working_count(), 0);
    }

    #[tokio::test]
    async fn production_start_blocks_a_stored_pending_order_at_approve() {
        // Queue an order through the test composition, then restart as production (real
        // charges and specs, no quote source) and approve it: the re-gate must block it.
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        assert_eq!(
            rt.handle.submit(e.order("TESTEQ")).await.unwrap(),
            Outcome::Queued
        );
        drop(rt);
        e.clock.advance(chrono::Duration::seconds(2));
        let rt = start(
            &e.db,
            &fixture_risk(),
            e.cfg.path(),
            e.clock.clone(),
            e.calendar(),
        )
        .await
        .unwrap();
        assert_eq!(rt.report.requeued, 1, "{:?}", rt.report);
        let out = rt
            .handle
            .decide(IntentId(1), ApprovalDecision::Approve, "owner")
            .await
            .unwrap();
        assert_eq!(reason(&out), "no fresh quote");
        assert_eq!(rt.paper.working_count(), 0);
    }

    #[tokio::test]
    async fn bad_charges_refuse_the_order_and_the_server_still_starts() {
        for body in [
            None,
            Some("schema_version = 1\nplaceholder = true\n"),
            Some("garbage"),
        ] {
            let e = env(&["TESTEQ"]).await;
            let f = e.cfg.path().join("charges.toml");
            let clk: Arc<dyn Clock> = e.clock.clone();
            // Control: the good file builds the real adapters; the bad one does not.
            assert!(matches!(
                cost_adapters(e.cfg.path(), 5, &clk).0,
                RuntimeCharges::Real(_)
            ));
            match body {
                None => std::fs::remove_file(&f).unwrap(),
                Some(b) => std::fs::write(&f, b).unwrap(),
            }
            assert!(matches!(
                cost_adapters(e.cfg.path(), 5, &clk).0,
                RuntimeCharges::Unavailable
            ));
            let rt = e.open().await; // starts: the kill switch stays reachable
            assert!(rt.kill_switch.read().await.readable);
            let out = rt.handle.submit(e.order("TESTEQ")).await.unwrap();
            assert!(reason(&out).starts_with("G02:"), "{body:?}: {out:?}");
            assert_eq!(rt.paper.working_count(), 0);
        }
    }

    #[tokio::test]
    async fn only_nse_equity_in_the_master_is_paper_tradable() {
        // OTHER is not in the master; TESTBSE (BSE equity) and TESTFUT (NSE future) are, and on
        // the watchlist, but only NSE equity is paper-tradable (QUAN-150-5).
        for symbol in ["OTHER", "TESTBSE", "TESTFUT"] {
            let e = env(&["TESTEQ", "TESTBSE", "TESTFUT"]).await;
            let rt = e.open().await;
            let out = e.approve(&rt, symbol).await;
            assert_eq!(
                out,
                Outcome::ExecFailed(ExecError::InvalidInput("unknown instrument")),
                "{symbol}"
            );
            assert_eq!(rt.paper.working_count(), 0, "{symbol}");
            let lines = PaperLedgerRepo::current_epoch(&rt.store).await.unwrap();
            assert!(
                lines.iter().all(|l| l.kind == "opening"),
                "{symbol}: {lines:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_stale_master_gives_no_specs_so_the_order_is_refused() {
        let e = env_aged(&["TESTEQ"], 30).await;
        // Precondition: the master really is stale (the snapshot reports a warning).
        let store = bazaar_cli::compose::open_store(&e.db).await.unwrap();
        let (_, warnings) = InstrumentSnapshot::load(&store, e.cfg.path(), &*e.clock).await;
        assert_ne!(warnings.len(), 0);
        drop(store);
        let rt = e.open().await;
        let out = e.approve(&rt, "TESTEQ").await; // asserts Queued first
        assert_eq!(
            out,
            Outcome::ExecFailed(ExecError::InvalidInput("unknown instrument"))
        );
        assert_eq!(rt.paper.working_count(), 0);
    }

    /// A quote an hour old.
    struct OldQuote(Arc<FakeClock>);
    impl QuoteSource for OldQuote {
        #[allow(clippy::unused_async)] // the QuoteSource trait requires an async fn
        async fn quote(&self, _: &str) -> Result<QuoteSnapshot, SourceError> {
            Ok(QuoteSnapshot {
                last: d("1950"),
                lower_circuit: None,
                upper_circuit: None,
                bar_volume: None,
                at_ist_rfc3339: (self.0.now_ist() - chrono::Duration::hours(1)).to_rfc3339(),
            })
        }
    }

    #[tokio::test]
    async fn build_wraps_the_quote_source_in_guarded_quotes() {
        let e = env(&["TESTEQ"]).await;
        let rt = start_with(
            &e.db,
            &fixture_risk(),
            e.cfg.path(),
            e.clock.clone(),
            e.calendar(),
            OldQuote(e.clock.clone()),
            OpenBase(e.clock.clone()),
        )
        .await
        .unwrap();
        let out = rt.handle.submit(e.order("TESTEQ")).await.unwrap();
        assert_eq!(reason(&out), "no fresh quote");
        assert_eq!(rt.paper.working_count(), 0);
    }

    #[test]
    fn unavailable_charges_never_return_a_charge() {
        let r = RuntimeCharges::Unavailable.charge("TESTEQ", true, 10, d("50"), "mis");
        assert_eq!(r, Err(SourceError::Unavailable));
    }

    // ---- BZ-521 S3: the latch over HTTP, through the seam (RISK-144-4) ----------------------

    use axum::body::Body;
    use axum::http::{Request, header};
    use bazaar_core::ports::{AuditRepo, KillSwitchRepo, PaperLedgerRepo};
    use http_body_util::BodyExt;
    use sqlx::ConnectOptions;
    use tower::ServiceExt;

    const HOST: &str = "127.0.0.1:8787";
    const ORIGIN: &str = "http://127.0.0.1:8787";

    /// The real router over a started test composition, logged in.
    struct Http {
        router: axum::Router,
        cookie: String,
        csrf: String,
    }

    async fn call(r: &axum::Router, req: Request<Body>) -> (u16, serde_json::Value) {
        let res = r.clone().oneshot(req).await.unwrap();
        let status = res.status().as_u16();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    async fn http(rt: &Runtime, clock: &Arc<FakeClock>) -> Http {
        let security = crate::security::Security::new(8787, false).unwrap();
        let state = crate::state::AppState::new(
            security.clone(),
            clock.clone(),
            bazaar_core::mode::ExecutionMode::Paper,
            std::path::PathBuf::new(),
        )
        .with_pipeline(rt.handle.clone(), rt.kill_switch.clone())
        .with_pending(rt.pending.clone());
        let router = crate::app::build(state);
        let url = security.launch_url(8787, false).unwrap();
        let path = url.strip_prefix(ORIGIN).unwrap().to_owned();
        let res = router
            .clone()
            .oneshot(
                Request::get(path)
                    .header(header::HOST, HOST)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 303);
        let cookie = res.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let me = Request::get("/api/v1/auth/me")
            .header(header::HOST, HOST)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let (_, me) = call(&router, me).await;
        let csrf = me["csrf_token"].as_str().unwrap().to_owned();
        Http {
            router,
            cookie,
            csrf,
        }
    }

    impl Http {
        async fn get_kill(&self) -> serde_json::Value {
            let req = Request::get("/api/v1/killswitch")
                .header(header::HOST, HOST)
                .header(header::COOKIE, &self.cookie)
                .body(Body::empty())
                .unwrap();
            let (status, v) = call(&self.router, req).await;
            assert_eq!(status, 200, "{v}");
            v
        }

        async fn post_engage(&self) -> (u16, serde_json::Value) {
            let req = Request::post("/api/v1/killswitch/engage")
                .header(header::HOST, HOST)
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &self.cookie)
                .header("x-csrf-token", &self.csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"reason":"stop"}"#))
                .unwrap();
            call(&self.router, req).await
        }

        async fn approve(&self, id: u128) -> serde_json::Value {
            self.approve_with(id, "key-0000001").await
        }

        async fn approve_with(&self, id: u128, key: &str) -> serde_json::Value {
            let (status, v) = self.approve_raw(id, key).await;
            assert_eq!(status, 200, "{v}");
            v
        }

        async fn approve_raw(&self, id: u128, key: &str) -> (u16, serde_json::Value) {
            let req = Request::post(format!("/api/v1/approvals/{id:032x}/decision"))
                .header(header::HOST, HOST)
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &self.cookie)
                .header("x-csrf-token", &self.csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .body(Body::from(r#"{"decision":"approve"}"#))
                .unwrap();
            call(&self.router, req).await
        }
    }

    /// Queue one order, let the approval delay pass; returns the HTTP rig.
    async fn queued(e: &Env, rt: &Runtime) -> Http {
        assert_eq!(
            rt.handle.submit(e.order("TESTEQ")).await.unwrap(),
            Outcome::Queued
        );
        e.clock.advance(chrono::Duration::seconds(2));
        http(rt, &e.clock).await
    }

    fn audit(kind: &str) -> AuditDraft {
        AuditDraft {
            ts_ist_rfc3339: "2026-12-01T10:31:00+05:30".into(),
            actor: "test".into(),
            kind: kind.into(),
            payload: "{}".into(),
        }
    }

    fn flags(v: &serde_json::Value) -> (bool, bool, bool) {
        (
            v["engaged"].as_bool().unwrap(),
            v["readable"].as_bool().unwrap(),
            v["latched"].as_bool().unwrap(),
        )
    }

    /// Run one SQL statement on the test database behind the runtime's back.
    async fn sql(e: &Env, stmt: &str) {
        let mut c = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&e.db)
            .connect()
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(stmt.to_owned()))
            .execute(&mut c)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_engage_whose_save_fails_is_503_then_shows_latched_and_refuses_the_approve() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let h = queued(&e, &rt).await;
        // The store stays readable (off) but refuses the engage write.
        sql(
            &e,
            "CREATE TRIGGER no_engage BEFORE UPDATE ON kill_switch \
             BEGIN SELECT RAISE(ABORT, 'write refused'); END",
        )
        .await;
        let (status, v) = h.post_engage().await;
        assert_eq!(status, 503, "{v}");
        assert_eq!(v["code"], "kill_switch_not_saved", "{v}");
        assert!(rt.handle.kill_switch_latched());
        let k = h.get_kill().await;
        assert_eq!(flags(&k), (true, true, true));
        assert!(k["reason"].is_null(), "latch only: no stored reason: {k}");
        let v = h.approve(1).await;
        assert_eq!(v["outcome"], "guard_blocked", "{v}");
        assert_eq!(v["detail"], "kill switch engaged", "{v}");
        assert_eq!(rt.paper.working_count(), 0);
        assert_eq!(
            OrderRepo::get(&rt.store, 1).await.unwrap().unwrap().state,
            "blocked"
        );
    }

    #[tokio::test]
    async fn a_latched_engage_with_the_store_unreadable_shows_engaged_unreadable_latched() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let h = http(&rt, &e.clock).await;
        rt.handle
            .set_kill_switch(true, "owner", "stop")
            .await
            .unwrap();
        assert_eq!(h.get_kill().await["reason"], "stop");
        sql(&e, "DROP TABLE kill_switch").await;
        let k = h.get_kill().await;
        assert_eq!(flags(&k), (true, false, true));
        assert!(k["reason"].is_null(), "unreadable: {k}");
    }

    #[tokio::test]
    async fn an_engaged_latch_with_the_store_off_shows_engaged_and_refuses_the_approve() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let h = queued(&e, &rt).await;
        assert_eq!(flags(&h.get_kill().await), (false, true, false));
        // Engage, then force the stored state off behind the latch's back.
        rt.handle
            .set_kill_switch(true, "owner", "stop")
            .await
            .unwrap();
        KillSwitchRepo::set(
            &rt.store,
            false,
            "forced",
            audit("kill_switch_released"),
            chain_hash,
        )
        .await
        .unwrap();
        assert!(!KillSwitchRepo::is_engaged(&rt.store).await.unwrap());
        let k = h.get_kill().await;
        assert_eq!(flags(&k), (true, true, true));
        // The stored row is off and says "forced": that is not why orders are refused.
        assert!(k["reason"].is_null(), "old reason shown: {k}");
        // The approve is refused by the kill switch: guard_blocked, no placement, no ledger line.
        let v = h.approve(1).await;
        assert_eq!(v["outcome"], "guard_blocked", "{v}");
        assert_eq!(v["detail"], "kill switch engaged", "{v}");
        assert_eq!(rt.paper.working_count(), 0);
        let lines = PaperLedgerRepo::current_epoch(&rt.store).await.unwrap();
        assert!(lines.iter().all(|l| l.kind == "opening"), "{lines:?}");
        let row = OrderRepo::get(&rt.store, 1).await.unwrap().unwrap();
        assert_eq!(row.state, "blocked");
        let rows = AuditRepo::read_all(&rt.store).await.unwrap();
        let blocked: Vec<_> = rows
            .iter()
            .filter(|r| r.draft.kind == "guard_blocked")
            .collect();
        assert_eq!(blocked.len(), 1, "{rows:?}");
        // Pin every audit row that moved: nothing placed, one guard_blocked, nothing else new.
        let ks: Vec<&str> = rows.iter().map(|r| r.draft.kind.as_str()).collect();
        assert_eq!(
            ks,
            [
                "paper_reset",
                "proposed",
                "gate_passed",
                "kill_switch_engaged",
                "kill_switch_released",
                "approve",
                "guard_blocked"
            ],
            "no placed/fill row"
        );
        // The approvals table holds exactly the one decision, for order 1.
        let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&e.db)
            .connect()
            .await
            .unwrap();
        let ids: Vec<String> = sqlx::query_scalar("SELECT order_id FROM approvals")
            .fetch_all(&mut conn)
            .await
            .unwrap();
        assert_eq!(ids, ["1"]);
    }

    #[tokio::test]
    async fn an_unreadable_store_with_the_latch_off_is_engaged_and_not_readable() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let h = http(&rt, &e.clock).await;
        let mut c = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&e.db)
            .connect()
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe("DROP TABLE kill_switch".to_owned()))
            .execute(&mut c)
            .await
            .unwrap();
        assert!(!rt.handle.kill_switch_latched());
        let k = h.get_kill().await;
        assert_eq!(flags(&k), (true, false, false));
        assert!(k["reason"].is_null(), "unreadable: {k}");
    }

    #[tokio::test]
    async fn a_stored_engage_returns_its_reason_escaped_without_the_latch() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let h = http(&rt, &e.clock).await;
        assert!(h.get_kill().await["reason"].is_null(), "never engaged");
        KillSwitchRepo::set(
            &rt.store,
            true,
            "bad\u{1b}[31m\nfeed\u{202e}",
            audit("kill_switch_engaged"),
            chain_hash,
        )
        .await
        .unwrap();
        let k = h.get_kill().await;
        assert_eq!(flags(&k), (true, true, false));
        assert_eq!(k["reason"], "bad\\x1b[31m\\x0afeed\\u{202e}");
    }

    #[tokio::test]
    async fn the_engage_reply_carries_the_reason_and_a_stored_release_clears_it() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let h = http(&rt, &e.clock).await;
        let (status, v) = h.post_engage().await;
        assert_eq!(status, 200, "{v}");
        assert_eq!(v["reason"], "stop", "{v}");
        // Released in the store while this process stays latched.
        KillSwitchRepo::set(
            &rt.store,
            false,
            "released",
            audit("kill_switch_released"),
            chain_hash,
        )
        .await
        .unwrap();
        let k = h.get_kill().await;
        assert_eq!(flags(&k), (true, true, true));
        assert!(k["reason"].is_null(), "old release reason shown: {k}");
    }

    #[test]
    fn stored_reading_gives_a_reason_only_for_a_readable_engaged_row() {
        let row = |engaged| {
            Ok(KillSwitchRow {
                engaged,
                version: 3,
                actor: "owner".into(),
                reason: "why".into(),
                changed_ns: 0,
            })
        };
        for latched in [false, true] {
            let on = stored_reading(row(true), latched);
            assert_eq!(
                (on.engaged, on.readable, on.latched, on.reason.as_deref()),
                (true, true, latched, Some("why"))
            );
            let off = stored_reading(row(false), latched);
            assert_eq!(
                (off.engaged, off.readable, off.latched, off.reason),
                (latched, true, latched, None)
            );
            for err in [RepoError::Corrupt, RepoError::Other] {
                let bad = stored_reading(Err(err), latched);
                assert_eq!(
                    (bad.engaged, bad.readable, bad.latched, bad.reason),
                    (true, false, latched, None)
                );
            }
        }
    }

    #[tokio::test]
    async fn a_good_quote_places_exactly_once_over_http() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let h = queued(&e, &rt).await;
        let v = h.approve(1).await;
        assert_eq!(v["outcome"], "placed", "{v}");
        assert_eq!(rt.paper.working_count(), 1);
        // A second approve of the same order places nothing more.
        let (status, again) = h.approve_raw(1, "key-0000002").await;
        assert_eq!(
            (status, again["code"].as_str()),
            (409, Some("already_decided")),
            "{again}"
        );
        assert_eq!(rt.paper.working_count(), 1);
    }

    /// A quote just past the drift limit of the proposed price (the limit is 50 bps).
    struct DriftQuote(Arc<FakeClock>);
    impl QuoteSource for DriftQuote {
        #[allow(clippy::unused_async)] // the QuoteSource trait requires an async fn
        async fn quote(&self, _: &str) -> Result<QuoteSnapshot, SourceError> {
            Ok(QuoteSnapshot {
                last: d("1960.50"),
                lower_circuit: None,
                upper_circuit: None,
                bar_volume: None,
                at_ist_rfc3339: self.0.now_ist().to_rfc3339(),
            })
        }
    }

    #[tokio::test]
    async fn a_drifted_quote_gives_quote_drifted_with_new_risk_and_places_nothing() {
        let e = env(&["TESTEQ"]).await;
        // Queue under a good quote, then restart with a quote past the drift limit (submit would
        // already see the drift).
        let first = e.open().await;
        assert_eq!(
            first.handle.submit(e.order("TESTEQ")).await.unwrap(),
            Outcome::Queued
        );
        drop(first);
        e.clock.advance(chrono::Duration::seconds(2));
        let rt = start_with(
            &e.db,
            &fixture_risk(),
            e.cfg.path(),
            e.clock.clone(),
            e.calendar(),
            DriftQuote(e.clock.clone()),
            OpenBase(e.clock.clone()),
        )
        .await
        .unwrap();
        assert_eq!(rt.report.requeued, 1, "{:?}", rt.report);
        let h = http(&rt, &e.clock).await;
        let v = h.approve(1).await;
        assert_eq!(v["outcome"], "quote_drifted", "{v}");
        assert_eq!(v["detail"], "new_risk=238.5325", "{v}");
        let row = OrderRepo::get(&rt.store, 1).await.unwrap().unwrap();
        let rows = AuditRepo::read_all(&rt.store).await.unwrap();
        let ks: Vec<&str> = rows.iter().map(|r| r.draft.kind.as_str()).collect();
        assert_eq!(row.state, "blocked");
        assert_eq!(
            ks,
            ["paper_reset", "proposed", "gate_passed", "gate_blocked"],
            "refused by the drift re-gate"
        );
        let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&e.db)
            .connect()
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM approvals")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(n, 0, "a refused approve writes no approval row");
        assert_eq!(rt.paper.working_count(), 0);
        let lines = PaperLedgerRepo::current_epoch(&rt.store).await.unwrap();
        assert!(lines.iter().all(|l| l.kind == "opening"), "{lines:?}");
    }

    /// T2 (F10): the good-quote approve moves exactly these rows, and the drift refusal moves
    /// exactly the refusal rows (known-pitfalls #5: every field that moves is checked).
    #[tokio::test]
    async fn t2_good_approve_moves_order_approval_and_audit_rows_and_nothing_else() {
        let e = env(&["TESTEQ"]).await;
        let rt = e.open().await;
        let h = queued(&e, &rt).await;
        let v = h.approve(1).await;
        assert_eq!(v["outcome"], "placed", "{v}");
        let row = OrderRepo::get(&rt.store, 1).await.unwrap().unwrap();
        let rows = AuditRepo::read_all(&rt.store).await.unwrap();
        let ks: Vec<&str> = rows.iter().map(|r| r.draft.kind.as_str()).collect();
        let lines = PaperLedgerRepo::current_epoch(&rt.store).await.unwrap();
        let kinds: Vec<&str> = lines.iter().map(|l| l.kind.as_str()).collect();
        assert_eq!(row.state, "sent");
        assert_eq!(
            ks,
            ["paper_reset", "proposed", "gate_passed", "approve", "send"],
            "one send row"
        );
        assert_eq!(kinds, ["opening"], "a working order moves no cash");
        assert_eq!(rt.paper.working_count(), 1);
        let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&e.db)
            .connect()
            .await
            .unwrap();
        let ap: Vec<(String, String)> = sqlx::query_as("SELECT decision, actor FROM approvals")
            .fetch_all(&mut conn)
            .await
            .unwrap();
        assert_eq!(ap, [("approved".to_owned(), "owner".to_owned())]);
    }
}
