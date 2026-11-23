//! Production composition (BZ-511 S1): opens the verified store, builds the one pipeline, runs
//! `reload()` once and only then `spawn()`; an `Err` there aborts startup before any bind.
//!
//! Every source here fails closed: no quote source means the approval-time re-gate is `Blocked`
//! for every order (owner decision 2026-11-23); no P&L, charge or instrument-spec source means
//! `Unavailable`/`None`, never a guess. The executor is paper; `live = Known(false)` unless the
//! build has `live-trading`, in which case it is `Unknown` (blocked) until a reconciler exists.

mod driver;

pub use driver::{ALARM_SQUARE_OFF, Driver, LedgerPnl, STOP_WAIT, serve_while_driven, stop};

use std::path::Path;
use std::sync::Arc;

use bazaar_cli::compose::{BuiltPipeline, ComposeError, Ports, UnknownCost, build_pipeline};
use bazaar_core::calendar::MarketCalendar;
use bazaar_core::clock::Clock;
use bazaar_core::config::{RiskConfig, parse_str};
use bazaar_core::ports::{
    KillSwitchRepo, OrderRepo, PaperLedgerRepo, QuoteSnapshot, QuoteSource, RepoError, SourceError,
};
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_execution::executor::{ApprovedOrder, ExecAck, ExecError, Executor};
use bazaar_execution::gate_adapter::{GateContext, GateInputsOwned};
use bazaar_execution::pipeline::{PipelineHandle, ReloadReport};
use bazaar_execution::types::OrderIntent;
use bazaar_paper::account::{ChargeSource, LedgerKind, PaperAccount};
use bazaar_paper::executor::{PaperExecutor, SpecSource};
use bazaar_paper::fill::{InstrumentSpec, SimConfig};
use bazaar_paper::squareoff::ValidatedSquareOffTime;
use bazaar_risk::baseline::{BaselineError, LimitsBaseline};
use bazaar_risk::config::Baseline;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::PortfolioSnapshot;
use bazaar_risk::types::Tri;
use bazaar_store::SqliteStore;
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

/// No charge estimate wired: the paper account refuses every fill (never books a zero charge).
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableCharges;

impl ChargeSource for UnavailableCharges {
    fn charge(
        &self,
        _: &str,
        _: bool,
        _: u32,
        _: Decimal,
        _: &str,
    ) -> Result<Decimal, SourceError> {
        Err(SourceError::Unavailable)
    }
}

/// No instrument specs wired: every instrument is unknown, so paper refuses the order.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnknownSpecs;

impl SpecSource for UnknownSpecs {
    fn spec(&self, _: &str) -> Option<InstrumentSpec> {
        None
    }
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
pub type Paper = PaperExecutor<SqliteStore, UnavailableCharges, SqliteStore, UnknownSpecs>;

/// Shares the paper executor with the tick driver (S2) while the pipeline owns the sending half.
#[derive(Clone, Debug)]
pub struct SharedPaper(pub Arc<Paper>);

impl Executor for SharedPaper {
    async fn place(&self, order: &ApprovedOrder) -> Result<ExecAck, ExecError> {
        self.0.place(order).await
    }
}

/// The pipeline the runtime builds.
pub type Built = BuiltPipeline<SharedPaper, LedgerPnl, UnavailableQuotes, ProdBase>;

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

/// Persisted kill switch, read-only; an unreadable state is reported engaged (fail closed).
struct StoreKillSwitch(SqliteStore);

impl KillSwitchRead for StoreKillSwitch {
    fn read(&self) -> ReadFuture<'_> {
        Box::pin(async move {
            match self.0.is_engaged().await {
                Ok(engaged) => KillSwitchReading {
                    engaged,
                    reason: None,
                    readable: true,
                },
                Err(_) => KillSwitchReading {
                    engaged: true,
                    reason: None,
                    readable: false,
                },
            }
        })
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
            let rows = OrderRepo::list_in_state(&self.store, "pending_approval", PENDING_LIMIT)
                .await
                .ok()?;
            let now = self.clock.now_ist();
            let mut out = Vec::with_capacity(rows.len());
            for r in rows {
                let id = bazaar_execution::types::IntentId(r.intent_id);
                // An undecodable row is not listed: reload expires it and audits that.
                let Some(i) = OrderIntent::from_payload(id, &r.payload, now) else {
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
    clock: Arc<dyn Clock>,
    calendar: MarketCalendar,
) -> Result<Runtime, StartError> {
    let store = bazaar_cli::compose::open_store(db).await?;
    let risk: RiskConfig = parse_str(risk_toml).map_err(|e| StartError::Config(e.to_string()))?;
    let pending = Arc::new(StorePending {
        store: store.clone(),
        clock: clock.clone(),
        ttl: chrono::Duration::seconds(i64::from(risk.execution.approval_ttl_secs)),
    });
    let (mut pipe, paper) = build(&store, risk_toml, clock, calendar).await?;
    let report = pipe.reload().await.map_err(StartError::Reload)?;
    Ok(Runtime {
        handle: pipe.spawn(),
        report,
        kill_switch: Arc::new(StoreKillSwitch(store.clone())),
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

async fn build(
    store: &SqliteStore,
    risk_toml: &str,
    clock: Arc<dyn Clock>,
    calendar: MarketCalendar,
) -> Result<(Built, Arc<Paper>), StartError> {
    let risk: RiskConfig = parse_str(risk_toml).map_err(|e| StartError::Config(e.to_string()))?;
    let ex = &risk.execution;
    let account = Arc::new(PaperAccount::new(
        store.clone(),
        UnavailableCharges,
        clock.clone(),
        ex.paper_starting_cash_inr,
    ));
    let opened = account
        .balances()
        .await
        .map_err(|e| StartError::Paper(e.to_string()))?;
    if opened.epoch.is_none() {
        account
            .reset()
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
        UnknownSpecs,
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
        quotes: UnavailableQuotes,
        base: ProdBase,
        clock,
        calendar,
        baseline: Arc::new(NoBaseline),
        cost: Arc::new(UnknownCost),
    };
    Ok((build_pipeline(store, risk_toml, ports).await?, paper))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
        let rt = start(&dir.path().join("x.db"), RISK, c.clone(), calendar())
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
            clock(),
            calendar(),
        )
        .await;
        assert!(matches!(e, Err(StartError::Config(_))), "{e:?}");
        // The store path is a directory: the verified open fails.
        let e = start(dir.path(), RISK, clock(), calendar()).await;
        assert!(matches!(e, Err(StartError::Store(_))), "{e:?}");
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
        let rt = start(&db, RISK, c, calendar()).await.unwrap();
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
        let rt = start(&dir.path().join("x.db"), RISK, c.clone(), calendar())
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
        let rt = start(&db, RISK, c.clone(), calendar()).await.unwrap();
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
        let rt = start(&dir.path().join("x.db"), RISK, sat, calendar()).await;
        assert!(rt.is_ok(), "{:?}", rt.err());
    }

    /// RISK-136-4: a ledger with fills cannot restore the position book, so startup refuses.
    #[tokio::test]
    async fn reopen_refuses_while_the_ledger_may_hold_open_lots() {
        use bazaar_core::ports::LedgerEntry;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("x.db");
        let c = clock();
        // A flat ledger (opening only) reopens fine.
        drop(start(&db, RISK, c.clone(), calendar()).await.unwrap());
        drop(start(&db, RISK, c.clone(), calendar()).await.unwrap());
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
        let e = start(&db, RISK, c, calendar()).await;
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
        drop(start(&db, RISK, c.clone(), calendar()).await.unwrap());
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
}
