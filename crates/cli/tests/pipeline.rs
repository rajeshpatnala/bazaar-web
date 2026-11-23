//! BZ-205 slice 5: the composed pipeline on a temp SQLite (real store, real gate adapter, real
//! guards; fake clock, quotes, executor and ids). AC1 (config), AC4 (approval-time checks, also
//! after a restart), AC8 (submit, approve, fill, close, reopen, tamper) and the kill-switch
//! reason. Nothing here touches the network, a broker or the owner's data.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unused_async
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use bazaar_cli::compose::{BuiltPipeline, Ports, UnknownCost, build_pipeline, open_store};
use bazaar_core::calendar::MarketCalendar;
use bazaar_core::clock::Clock;
use bazaar_core::config::{CalendarConfig, RiskConfig, parse_str};
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::ports::{
    ApprovalDecision, ApprovalRepo, AuditDraft, AuditRepo, OrderRecord, OrderRepo, QuoteSnapshot,
    QuoteSource, RepoError, SourceError,
};
use bazaar_core::testutil::FakeClock;
use bazaar_execution::approval::Outcome;
use bazaar_execution::audit::{chain_hash, verify_chain};
use bazaar_execution::breaker::{DayPnl, PnlSource};
use bazaar_execution::executor::{ApprovedOrder, ExecAck, ExecError, Executor};
use bazaar_execution::gate_adapter::{GateContext, GateInputsOwned};
use bazaar_execution::pipeline::{PipelineConfig, PipelineHandle, ReloadReport};
use bazaar_execution::types::{IntentId, OrderIntent, OrderKind, Product};
use bazaar_paper::account::{ChargeSource, FillBooking, PaperAccount};
use bazaar_risk::baseline::{BaselineError, LimitsBaseline};
use bazaar_risk::config::Baseline;
use bazaar_risk::gate::{CostModel, ModeEvidence};
use bazaar_risk::rules::behaviour::EntryZone;
use bazaar_risk::rules::exposure::ExposureInputs;
use bazaar_risk::rules::inputs::{DataInputs, EventFlags, EvidenceInputs, MarginInputs};
use bazaar_risk::snapshot::{Mark, PortfolioSnapshot, Position};
use bazaar_risk::types::{Side, Tri};
use bazaar_store::SqliteStore;
use chrono::{DateTime, Duration, NaiveDate, TimeZone};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Row};

const RISK: &str = include_str!("../../../fixtures/config/risk.toml");
const OWNER_RISK: &str = include_str!("../../../config/risk.toml");

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}
fn t0() -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 11, 19, 11, 0, 0).unwrap()
}

// ---- fakes ---------------------------------------------------------------------------------

#[derive(Default)]
struct Held(Mutex<Vec<Position>>);

fn long(qty: u32) -> Position {
    Position {
        symbol: "ABC".into(),
        side: Side::Buy,
        qty,
        stop: Tri::Known(d("40")),
        risk_price: Tri::Known(d("50")),
        avg_cost: Tri::Known(d("50")),
    }
}

/// Market-side gate inputs (the gate-adapter unit tests' fixture), with positions we control.
struct Base(Arc<Held>, Arc<FakeClock>);
impl GateContext for Base {
    #[allow(clippy::too_many_lines)]
    fn inputs(&self, _: &OrderIntent) -> Option<GateInputsOwned> {
        let held = self.0.0.lock().unwrap().clone();
        let end = t0().date_naive();
        let series = (0..100u32)
            .map(|i| {
                (
                    end - Duration::days(i64::from(99 - i)),
                    Decimal::new(i64::from(i % 7) - 3, 3),
                )
            })
            .collect();
        let mut s = PortfolioSnapshot::unknown();
        s.as_of = Tri::Known(self.1.now_ist());
        s.positions = Tri::Known(held.clone());
        s.consecutive_losses = Tri::Known(0);
        s.day_pnl = Tri::Known(Decimal::ZERO);
        s.week_pnl = Tri::Known(Decimal::ZERO);
        s.week_start = Tri::Known(NaiveDate::from_ymd_opt(2026, 11, 16).unwrap());
        s.entries_today = Tri::Known(0);
        s.orders_last_minute = Tri::Known(0);
        s.orders_today = Tri::Known(0);
        s.overrides_today = Tri::Known(0);
        s.overrides_week = Tri::Known(0);
        s.kill_switch = Tri::Known(false); // the production context overrides this from the store
        s.charges_buffer = Tri::Known(d("20"));
        s.starting_cash = Tri::Known(d("250000"));
        s.realised_pnl = Tri::Known(Decimal::ZERO);
        s.equity_mtm = Tri::Known(d("250000"));
        s.marks = Tri::Known(
            held.iter()
                .map(|p| Mark {
                    symbol: p.symbol.clone(),
                    price: d("50"),
                })
                .collect(),
        );
        s.last_losing_exits = Tri::Known(vec![]);
        Some(GateInputsOwned {
            snapshot: s,
            data: DataInputs {
                inputs_as_of: Tri::Known(self.1.now_ist()),
                evidence: EvidenceInputs {
                    oos_trades: Tri::Known(100),
                    profit_factor: Tri::Known(d("2")),
                    expectancy_r: Tri::Known(d("0.3")),
                    walk_forward_pass: Tri::Known(true),
                },
                ban_list: Tri::Unknown,
                stock_fno_underlying: Tri::Known(None),
                events: EventFlags {
                    results_window: Tri::Known(false),
                    policy_day: Tri::Known(false),
                    option_on_own_expiry: Tri::Known(false),
                },
                quote_at: Tri::Unknown, // the adapter takes it from the quote
                session_open: Tri::Known(true),
                margin: MarginInputs {
                    qty: 10,
                    required: Tri::Known(d("5000")),
                    available: Tri::Known(d("250000")),
                },
            },
            mode: ModeEvidence {
                approval: None,
                compliance_fresh: Tri::Known(false),
                readiness_report: Tri::Known(false),
                owner_opt_in: Tri::Known(false),
                live: Tri::Known(false),
            },
            entry_zone: Tri::Known(EntryZone {
                low: d("49.5"),
                high: d("50.1"),
            }),
            exposure: Some(ExposureInputs {
                sectors: BTreeMap::from([("ABC".to_string(), "IT".to_string())]),
                returns: BTreeMap::from([("ABC".to_string(), series)]),
            }),
            defined_max_loss: None,
            instrument: Some(Instrument {
                instrument_token: 1,
                exchange_token: 1,
                tradingsymbol: "ABC".into(),
                name: "ABC".into(),
                exchange: Exchange::Nse,
                instrument_type: InstrumentType::Equity,
                segment: "NSE".into(),
                expiry: None,
                strike: None,
                tick_size: d("0.05"),
                lot_size: 1,
            }),
            stop: Some(d("40")),
            proposed_price: Some(d("50")),
            // The production context replaces this; a wrong value here would show.
            pending_exit_qty: Some(Decimal::from(9999)),
        })
    }
}

#[derive(Debug)]
struct KnownCost;
impl CostModel for KnownCost {
    fn cost(&self, _: Side, _: u32, _: Decimal) -> Tri<Decimal> {
        Tri::Known(d("20"))
    }
}

struct NoBaseline;
impl LimitsBaseline for NoBaseline {
    fn read(&self) -> Result<Baseline, BaselineError> {
        Ok(Baseline::none())
    }
}

struct Pnl;
impl PnlSource for Pnl {
    async fn day_pnl(&self) -> Result<DayPnl, SourceError> {
        tokio::task::yield_now().await;
        Ok(DayPnl {
            pnl: Decimal::ZERO,
            equity_base: d("250000"),
        })
    }
}

struct Fee;
impl ChargeSource for Fee {
    fn charge(
        &self,
        _: &str,
        _: bool,
        _: u32,
        _: Decimal,
        _: &str,
    ) -> Result<Decimal, SourceError> {
        Ok(Decimal::ONE)
    }
}

/// Fake quote feed: price is settable, the quote time is the (fake) clock's now.
struct QuoteState {
    last: Mutex<Decimal>,
    lag_secs: Mutex<i64>,
    clock: Arc<FakeClock>,
}
struct Quotes(Arc<QuoteState>);
impl QuoteSource for Quotes {
    async fn quote(&self, _: &str) -> Result<QuoteSnapshot, SourceError> {
        tokio::task::yield_now().await;
        Ok(QuoteSnapshot {
            last: *self.0.last.lock().unwrap(),
            lower_circuit: None,
            upper_circuit: None,
            bar_volume: None,
            at_ist_rfc3339: (self.0.clock.now_ist()
                - Duration::seconds(*self.0.lag_secs.lock().unwrap()))
            .to_rfc3339(),
        })
    }
}

/// Fake broker/paper executor: fills in full, books the paper ledger and the position; or, when
/// told, loses the answer (`unknown`). Records every placement attempt.
struct Broker {
    placed: Arc<Mutex<Vec<u128>>>,
    lose: Arc<AtomicBool>,
    held: Arc<Held>,
    account: Arc<PaperAccount<SqliteStore, Fee>>,
}
impl Executor for Broker {
    async fn place(&self, order: &ApprovedOrder) -> Result<ExecAck, ExecError> {
        let i = order.intent();
        self.placed.lock().unwrap().push(i.id.0);
        if self.lose.load(SeqCst) {
            return Err(ExecError::Unknown);
        }
        let held_qty = self.held.0.lock().unwrap().first().map_or(0, |p| p.qty);
        let is_buy = i.side == Side::Buy;
        self.account
            .apply_fill(&FillBooking {
                ref_id: i.id.0,
                symbol: i.symbol.clone(),
                is_buy,
                qty: i.qty,
                price: d("50"),
                product: i.product.as_str().into(),
                held_qty,
            })
            .await
            .map_err(|_| ExecError::Storage)?;
        let mut h = self.held.0.lock().unwrap();
        if is_buy {
            h.push(long(i.qty));
        } else if held_qty == i.qty {
            h.clear();
        }
        Ok(ExecAck {
            intent_id: i.id,
            external_id: format!("paper-{}", i.id.0),
            filled_qty: i.qty,
            avg_price: Some(d("50")),
        })
    }
}

// ---- harness -------------------------------------------------------------------------------

struct Env {
    _dir: tempfile::TempDir,
    path: PathBuf,
    clock: Arc<FakeClock>,
    quotes: Arc<QuoteState>,
    held: Arc<Held>,
    placed: Arc<Mutex<Vec<u128>>>,
    lose: Arc<AtomicBool>,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(FakeClock::at(t0()));
        Self {
            path: dir.path().join("exec.db"),
            _dir: dir,
            quotes: Arc::new(QuoteState {
                last: Mutex::new(d("50")),
                lag_secs: Mutex::new(0),
                clock: clock.clone(),
            }),
            clock,
            held: Arc::default(),
            placed: Arc::default(),
            lose: Arc::default(),
        }
    }
    fn advance(&self, secs: i64) {
        self.clock.advance(Duration::seconds(secs));
    }
    fn price(&self, p: &str) {
        *self.quotes.last.lock().unwrap() = d(p);
    }
    fn placed(&self) -> Vec<u128> {
        self.placed.lock().unwrap().clone()
    }
    fn intent(&self, id: u128, side: Side, qty: u32, reduce_only: bool) -> OrderIntent {
        OrderIntent {
            id: IntentId(id),
            symbol: "ABC".into(),
            side,
            qty,
            kind: OrderKind::Limit(d("50")),
            product: Product::Mis,
            reduce_only,
            created_at: self.clock.now_ist(),
        }
    }
}

type Pipe = BuiltPipeline<Broker, Pnl, Quotes, Base>;

fn calendar() -> MarketCalendar {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config");
    MarketCalendar::from_config(
        &bazaar_core::config::load::<CalendarConfig>(std::path::Path::new(dir)).unwrap(),
    )
}

async fn build(env: &Env, cost: Arc<dyn CostModel>) -> (SqliteStore, Pipe) {
    build_with(env, cost, RISK).await
}

async fn build_with(env: &Env, cost: Arc<dyn CostModel>, risk: &str) -> (SqliteStore, Pipe) {
    let store = open_store(&env.path).await.unwrap();
    let cfg: RiskConfig = parse_str(risk).unwrap();
    let account = Arc::new(PaperAccount::new(
        store.clone(),
        Fee,
        env.clock.clone(),
        cfg.execution.paper_starting_cash_inr,
    ));
    if account.balances().await.unwrap().epoch.is_none() {
        account.reset().await.unwrap();
    }
    let ports = Ports {
        executor: Broker {
            placed: env.placed.clone(),
            lose: env.lose.clone(),
            held: env.held.clone(),
            account,
        },
        pnl: Pnl,
        quotes: Quotes(env.quotes.clone()),
        base: Base(env.held.clone(), env.clock.clone()),
        clock: env.clock.clone(),
        calendar: calendar(),
        baseline: Arc::new(NoBaseline),
        cost,
    };
    let pipe = build_pipeline(&store, risk, ports).await.unwrap();
    (store, pipe)
}

/// Boot like startup does: build, reload (must succeed), spawn.
async fn boot(env: &Env) -> (SqliteStore, PipelineHandle, ReloadReport) {
    let (store, mut pipe) = build(env, Arc::new(KnownCost)).await;
    let rep = pipe.reload().await.unwrap();
    (store, pipe.spawn(), rep)
}

async fn state(s: &SqliteStore, id: u128) -> String {
    OrderRepo::get(s, id).await.unwrap().unwrap().state
}

const OWNER: &str = "owner";

fn blocked(o: &Outcome) -> bool {
    matches!(o, Outcome::GateBlocked(_))
}

// ---- AC1 -----------------------------------------------------------------------------------

#[test]
fn owner_confirmed_values_reach_the_pipeline_config() {
    let r: RiskConfig = parse_str(OWNER_RISK).unwrap();
    assert_eq!(r.execution.io_timeout_secs, 5);
    assert!(r.execution.breaker_engages_kill_switch);
    let cfg = PipelineConfig::from_section(&r.execution);
    assert_eq!(cfg.io_timeout(), std::time::Duration::from_secs(5));
    assert_eq!(cfg.approval_ttl_secs(), 300);
    // RISK-136-6: the owner-confirmed keys are asserted by name and value, not by a comment count.
    let e = &r.execution;
    assert_eq!(e.approval_ttl_secs, 300);
    assert_eq!(e.max_quote_drift_bps, 50);
    assert_eq!(e.max_orders_per_second, 1);
    assert!(e.kill_allow_reduce_only);
    assert_eq!(e.kill_switch_release_phrase, "RELEASE KILL SWITCH");
    assert_eq!(e.paper_starting_cash_inr, d("100000"));
    assert_eq!(e.paper_max_participation, d("0.1"));
    assert_eq!(e.paper_stale_after_secs, 5);
    assert_eq!(e.paper_mis_square_off_ist.to_string(), "15:15:00");
    assert_eq!(e.paper_slippage_bps.equity, 5);
    assert_eq!(e.paper_slippage_bps.equity_derivatives, 5);
    assert_eq!(e.paper_slippage_bps.currency, 2);
    assert_eq!(e.paper_slippage_bps.commodity, 5);
}

// ---- AC8 -----------------------------------------------------------------------------------

#[tokio::test]
async fn ac8_submit_approve_fill_close_reopen_is_consistent_and_tamper_is_corrupt() {
    let env = Env::new();
    let (store, h, rep) = boot(&env).await;
    assert_eq!(rep, ReloadReport::default());

    // Entry: submit, approve, filled.
    assert_eq!(
        h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap(),
        Outcome::Queued
    );
    env.advance(2);
    let out = h
        .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(
        matches!(out, Outcome::Placed(ref a) if a.filled_qty == 10),
        "{out:?}"
    );
    assert_eq!(state(&store, 1).await, "filled");
    assert_eq!(env.held.0.lock().unwrap().len(), 1);

    // Close: a reduce_only exit for the whole position. At approval the exit is re-gated with
    // the position held and no OTHER pending exit: it must not net against itself.
    env.advance(2);
    assert_eq!(
        h.submit(env.intent(2, Side::Sell, 10, true)).await.unwrap(),
        Outcome::Queued
    );
    env.advance(2);
    let out = h
        .decide(IntentId(2), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Placed(_)), "{out:?}");
    assert_eq!(state(&store, 2).await, "filled");
    assert!(env.held.0.lock().unwrap().is_empty());
    assert_eq!(env.placed(), vec![1, 2]);

    // Ledger: opening 250000, buy 500 + 1 charge, sell 500 + 1 charge.
    let account = PaperAccount::new(store.clone(), Fee, env.clock.clone(), d("250000"));
    let bal = account.balances().await.unwrap();
    assert_eq!(
        (bal.cash, bal.charges_paid, bal.entries),
        (d("249998"), d("2"), 5)
    );

    // Events and chain.
    let ev1 = store.order_events("1").await.unwrap().len();
    let ev2 = store.order_events("2").await.unwrap().len();
    assert!(ev1 >= 5 && ev2 >= 5, "{ev1} {ev2}"); // gate_passed, approve, send, fill (+ proposed)
    let rows = AuditRepo::read_all(&store).await.unwrap();
    verify_chain(&rows).unwrap();
    let audit_len = rows.len();
    drop(h);
    drop(store);

    // Reopen through `open_store`: the same positions, ledger, events and chain.
    let store = open_store(&env.path).await.unwrap();
    assert_eq!(state(&store, 1).await, "filled");
    assert_eq!(state(&store, 2).await, "filled");
    assert_eq!(store.order_events("1").await.unwrap().len(), ev1);
    let rows = AuditRepo::read_all(&store).await.unwrap();
    assert_eq!(rows.len(), audit_len);
    verify_chain(&rows).unwrap();
    let account = PaperAccount::new(store.clone(), Fee, env.clock.clone(), d("250000"));
    assert_eq!(account.balances().await.unwrap(), bal);
    drop(account);
    drop(store);

    // Tamper with one audit row out of band: reopen is Corrupt.
    let mut c = SqliteConnectOptions::new()
        .filename(&env.path)
        .connect()
        .await
        .unwrap();
    sqlx::query("DROP TRIGGER audit_log_no_update")
        .execute(&mut c)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_log SET actor = 'mallory' WHERE seq = 3")
        .execute(&mut c)
        .await
        .unwrap();
    c.close().await.unwrap();
    assert_eq!(open_store(&env.path).await.err(), Some(RepoError::Corrupt));
}

// ---- production GateContext (RISK-109-3, RISK-134-3/8) ---------------------------------------

#[tokio::test]
async fn an_exit_must_shrink_the_position_net_of_other_pending_exits() {
    let env = Env::new();
    env.held.0.lock().unwrap().push(long(10));
    let (store, h, _) = boot(&env).await;
    assert_eq!(
        h.submit(env.intent(1, Side::Sell, 6, true)).await.unwrap(),
        Outcome::Queued
    );
    // 6 pending + 6 more > 10 held: blocked.
    let second = h.submit(env.intent(2, Side::Sell, 6, true)).await.unwrap();
    assert!(blocked(&second), "{second:?}");
    assert_eq!(state(&store, 2).await, "blocked");
    // 6 + 4 fits exactly.
    assert_eq!(
        h.submit(env.intent(3, Side::Sell, 4, true)).await.unwrap(),
        Outcome::Queued
    );
    // An approved-and-sent exit also counts (here: lost answer leaves it `unknown`).
    env.lose.store(true, SeqCst);
    env.advance(2);
    let out = h
        .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(
        matches!(out, Outcome::ExecFailed(ExecError::Unknown)),
        "{out:?}"
    );
    assert_eq!(state(&store, 1).await, "unknown");
    let again = h.submit(env.intent(4, Side::Sell, 1, true)).await.unwrap();
    assert!(blocked(&again), "6 unknown + 4 pending + 1 > 10: {again:?}");
}

#[tokio::test]
async fn an_undecodable_stored_order_makes_pending_exits_unknown_and_blocks_exits() {
    let env = Env::new();
    env.held.0.lock().unwrap().push(long(10));
    let (store, h, _) = boot(&env).await;
    let junk = AuditDraft {
        ts_ist_rfc3339: t0().to_rfc3339(),
        actor: "test".into(),
        kind: "proposed".into(),
        payload: "{}".into(),
    };
    OrderRepo::insert(
        &store,
        OrderRecord {
            intent_id: 99,
            version: 0,
            state: "approved".into(),
            payload: "not json".into(),
        },
        junk,
        chain_hash,
    )
    .await
    .unwrap();
    let out = h.submit(env.intent(1, Side::Sell, 1, true)).await.unwrap();
    assert!(blocked(&out), "{out:?}");
    // Entries do not depend on pending exits.
    assert_eq!(
        h.submit(env.intent(2, Side::Buy, 10, false)).await.unwrap(),
        Outcome::Queued
    );
}

#[tokio::test]
async fn the_cost_model_unknown_blocks_and_a_missing_position_blocks_an_exit() {
    let env = Env::new();
    let (store, mut pipe) = build(&env, Arc::new(UnknownCost)).await;
    pipe.reload().await.unwrap();
    let h = pipe.spawn();
    let out = h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap();
    assert!(blocked(&out), "{out:?}");
    assert_eq!(state(&store, 1).await, "blocked");
    let (_, h2, _) = boot(&env).await;
    let out = h2.submit(env.intent(2, Side::Sell, 1, true)).await.unwrap();
    assert!(blocked(&out), "no live position: {out:?}");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

// ---- kill switch: typed reason, store-fed state ------------------------------------------------

#[tokio::test]
async fn kill_switch_reason_is_stored_and_the_gate_reads_it_from_the_store() {
    let env = Env::new();
    env.held.0.lock().unwrap().push(long(10));
    let (store, h, _) = boot(&env).await;
    h.set_kill_switch(true, OWNER, "manual stop for test")
        .await
        .unwrap();
    let row = store.kill_switch_get().await.unwrap();
    assert!(row.engaged);
    assert_eq!(row.reason, "manual stop for test");
    let mut c = SqliteConnectOptions::new()
        .filename(&env.path)
        .connect()
        .await
        .unwrap();
    let ev: String =
        sqlx::query("SELECT reason FROM kill_switch_events ORDER BY version DESC LIMIT 1")
            .fetch_one(&mut c)
            .await
            .unwrap()
            .get(0);
    assert_eq!(ev, "manual stop for test");
    c.close().await.unwrap();

    // Entries blocked at the gate (the context's kill state is the store's); a genuine exit
    // passes while `kill_allow_reduce_only` is true.
    let entry = h.submit(env.intent(1, Side::Buy, 1, false)).await.unwrap();
    assert!(blocked(&entry), "{entry:?}");
    assert_eq!(
        h.submit(env.intent(2, Side::Sell, 4, true)).await.unwrap(),
        Outcome::Queued
    );
    env.advance(2);
    let out = h
        .decide(IntentId(2), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Placed(_)), "{out:?}");
}

#[tokio::test]
async fn with_kill_allow_reduce_only_false_an_exit_is_blocked_while_killed() {
    let env = Env::new();
    env.held.0.lock().unwrap().push(long(10));
    let risk = RISK.replace(
        "kill_allow_reduce_only = true",
        "kill_allow_reduce_only = false",
    );
    assert_ne!(risk, RISK, "the fixture key must be patched");
    let (store, mut pipe) = build_with(&env, Arc::new(KnownCost), &risk).await;
    pipe.reload().await.unwrap();
    let h = pipe.spawn();
    // Not killed: the exit queues.
    assert_eq!(
        h.submit(env.intent(1, Side::Sell, 4, true)).await.unwrap(),
        Outcome::Queued
    );
    h.set_kill_switch(true, OWNER, "stop").await.unwrap();
    env.advance(2);
    // Killed with the key false: neither a new exit nor the approval of a queued one goes out.
    let new_exit = h.submit(env.intent(2, Side::Sell, 4, true)).await.unwrap();
    assert!(blocked(&new_exit), "{new_exit:?}");
    let out = h
        .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(!matches!(out, Outcome::Placed(_)), "{out:?}");
    assert_eq!(env.placed(), Vec::<u128>::new());
    assert_ne!(state(&store, 1).await, "filled");
}

// ---- AC4: approval-time TTL and fresh-quote re-gate ----------------------------------------

#[tokio::test]
async fn approval_after_the_ttl_expires_and_sends_nothing() {
    let env = Env::new();
    let (store, h, _) = boot(&env).await;
    assert_eq!(
        h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap(),
        Outcome::Queued
    );
    env.advance(301);
    let out = h
        .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert_eq!(out, Outcome::Expired);
    assert_eq!(state(&store, 1).await, "expired");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

#[tokio::test]
async fn approval_regates_on_a_fresh_quote_drift_and_a_stale_quote_block() {
    let env = Env::new();
    let (store, h, _) = boot(&env).await;
    assert_eq!(
        h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap(),
        Outcome::Queued
    );
    env.price("49.70"); // 60 bps from the shown 50, limit 50
    env.advance(2);
    let out = h
        .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(matches!(out, Outcome::QuoteDrifted { .. }), "{out:?}");
    assert_eq!(state(&store, 1).await, "blocked");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

#[tokio::test]
async fn a_quote_older_than_the_limit_blocks_because_quote_at_comes_from_the_quote() {
    let env = Env::new();
    let (store, h, _) = boot(&env).await;
    assert_eq!(
        h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap(),
        Outcome::Queued
    );
    // The feed freezes: the quote's own time lags the clock by 120 s (the price is unchanged).
    *env.quotes.lag_secs.lock().unwrap() = 120;
    env.advance(2);
    let out = h
        .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(
        matches!(&out, Outcome::GateBlocked(m) if m.contains("G15")),
        "{out:?}"
    );
    assert_eq!(state(&store, 1).await, "blocked");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

#[tokio::test]
async fn restart_requeues_with_the_original_time_and_the_ttl_still_applies() {
    let env = Env::new();
    {
        let (_, h, _) = boot(&env).await;
        assert_eq!(
            h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap(),
            Outcome::Queued
        );
        env.advance(200);
        assert_eq!(
            h.submit(env.intent(2, Side::Buy, 10, false)).await.unwrap(),
            Outcome::Queued
        );
    } // crash: pipeline and store dropped
    tokio::task::yield_now().await;
    env.advance(150); // #1 is 350 s old (past TTL), #2 is 150 s old
    let (store, h, rep) = boot(&env).await;
    assert_eq!((rep.requeued, rep.expired), (1, 1), "{rep:?}");
    assert_eq!(state(&store, 1).await, "expired");
    let out = h
        .decide(IntentId(2), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Placed(_)), "{out:?}");
    // 150 s later it would have expired: the original queue time was kept.
    assert_eq!(env.placed(), vec![2]);
}

#[tokio::test]
async fn restart_requeued_intent_expires_at_its_original_ttl_not_a_fresh_one() {
    let env = Env::new();
    {
        let (_, h, _) = boot(&env).await;
        assert_eq!(
            h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap(),
            Outcome::Queued
        );
    }
    env.advance(200);
    let (store, h, rep) = boot(&env).await;
    assert_eq!(rep.requeued, 1);
    env.advance(150); // 350 s since submit: past TTL only if the original time was kept
    let out = h
        .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert_eq!(out, Outcome::Expired);
    assert_eq!(state(&store, 1).await, "expired");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

/// A crash between the approval write and the send leaves an `approved` order.
async fn seed_approved(env: &Env, id: u128, kill: bool) {
    let (store, h, _) = boot(env).await;
    assert_eq!(
        h.submit(env.intent(id, Side::Buy, 10, false))
            .await
            .unwrap(),
        Outcome::Queued
    );
    let rec = OrderRepo::get(&store, id).await.unwrap().unwrap();
    let audit = AuditDraft {
        ts_ist_rfc3339: env.clock.now_ist().to_rfc3339(),
        actor: OWNER.into(),
        kind: "approve".into(),
        payload: "{}".into(),
    };
    ApprovalRepo::decide_and_advance(
        &store,
        id,
        ApprovalDecision::Approve,
        OWNER,
        rec.version,
        "approved".into(),
        "approve".into(),
        audit,
        chain_hash,
    )
    .await
    .unwrap();
    assert_eq!(state(&store, id).await, "approved");
    if kill {
        h.set_kill_switch(true, OWNER, "halt before restart")
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_stored_approve_is_blocked_at_reload_when_the_kill_switch_is_engaged() {
    let env = Env::new();
    seed_approved(&env, 1, true).await;
    env.advance(5);
    let (store, _h, rep) = boot(&env).await;
    assert_eq!((rep.blocked, rep.resumed), (1, 0), "{rep:?}");
    assert_eq!(state(&store, 1).await, "blocked");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

#[tokio::test]
async fn a_stored_approve_is_blocked_at_reload_when_the_cost_is_unknown() {
    let env = Env::new();
    seed_approved(&env, 1, false).await;
    env.advance(5);
    let (store, mut pipe) = build(&env, Arc::new(UnknownCost)).await;
    let rep = pipe.reload().await.unwrap();
    let _h = pipe.spawn();
    assert_eq!((rep.blocked, rep.resumed), (1, 0), "{rep:?}");
    assert_eq!(state(&store, 1).await, "blocked");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

#[tokio::test]
async fn an_unreadable_kill_switch_reads_as_unknown_and_blocks_entries() {
    let env = Env::new();
    let (_store, h, _) = boot(&env).await;
    // Break the kill switch table out of band: the store read now fails.
    let mut c = SqliteConnectOptions::new()
        .filename(&env.path)
        .connect()
        .await
        .unwrap();
    sqlx::query("DROP TABLE kill_switch")
        .execute(&mut c)
        .await
        .unwrap();
    c.close().await.unwrap();
    let out = h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap();
    assert!(blocked(&out), "{out:?}");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

#[tokio::test]
async fn a_stored_approve_after_restart_is_regated_first_and_a_drift_ends_blocked() {
    let env = Env::new();
    seed_approved(&env, 1, false).await;
    env.advance(5);
    env.price("49.70");
    let (store, _h, rep) = boot(&env).await;
    assert_eq!((rep.blocked, rep.resumed), (1, 0), "{rep:?}");
    assert_eq!(state(&store, 1).await, "blocked");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

#[tokio::test]
async fn a_stored_approve_after_restart_past_the_ttl_ends_blocked_and_sends_nothing() {
    let env = Env::new();
    seed_approved(&env, 1, false).await;
    env.advance(301);
    let (store, _h, rep) = boot(&env).await;
    assert_eq!((rep.blocked, rep.resumed), (1, 0), "{rep:?}");
    assert_eq!(state(&store, 1).await, "blocked");
    assert_eq!(env.placed(), Vec::<u128>::new());
}

#[tokio::test]
async fn a_stored_approve_after_restart_resumes_only_when_everything_still_passes() {
    let env = Env::new();
    seed_approved(&env, 1, false).await;
    env.advance(5);
    let (store, _h, rep) = boot(&env).await;
    assert_eq!((rep.resumed, rep.blocked), (1, 0), "{rep:?}");
    assert_eq!(state(&store, 1).await, "filled");
    assert_eq!(env.placed(), vec![1]);
}

#[tokio::test]
async fn a_restart_never_resends_a_sent_or_unknown_order() {
    let env = Env::new();
    env.lose.store(true, SeqCst);
    {
        let (_, h, _) = boot(&env).await;
        assert_eq!(
            h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap(),
            Outcome::Queued
        );
        env.advance(2);
        let out = h
            .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
            .await
            .unwrap();
        assert!(
            matches!(out, Outcome::ExecFailed(ExecError::Unknown)),
            "{out:?}"
        );
    }
    env.lose.store(false, SeqCst);
    env.advance(10);
    let (store, _h, rep) = boot(&env).await;
    assert_eq!(
        (rep.in_flight, rep.resumed, rep.requeued),
        (1, 0, 0),
        "{rep:?}"
    );
    assert_eq!(state(&store, 1).await, "unknown");
    assert_eq!(env.placed(), vec![1], "sent exactly once, never re-sent");
}

#[tokio::test]
async fn a_second_decision_on_a_filled_order_is_a_conflict_and_writes_nothing() {
    // Atomic decide (AC3) on the real store: an approval for an order that is no longer pending
    // is refused and nothing is written.
    let env = Env::new();
    let (store, h, _) = boot(&env).await;
    assert_eq!(
        h.submit(env.intent(1, Side::Buy, 10, false)).await.unwrap(),
        Outcome::Queued
    );
    env.advance(2);
    let out = h
        .decide(IntentId(1), ApprovalDecision::Approve, OWNER)
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Placed(_)));
    let before = AuditRepo::read_all(&store).await.unwrap().len();
    let audit = AuditDraft {
        ts_ist_rfc3339: env.clock.now_ist().to_rfc3339(),
        actor: OWNER.into(),
        kind: "approve".into(),
        payload: "{}".into(),
    };
    let r = ApprovalRepo::decide_and_advance(
        &store,
        1,
        ApprovalDecision::Approve,
        OWNER,
        0,
        "approved".into(),
        "approve".into(),
        audit,
        chain_hash,
    )
    .await;
    assert_eq!(r.err(), Some(RepoError::Conflict));
    assert_eq!(AuditRepo::read_all(&store).await.unwrap().len(), before);
    assert_eq!(state(&store, 1).await, "filled");
}
