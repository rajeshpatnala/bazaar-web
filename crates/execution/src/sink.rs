//! `ChainAuditSink`: the real `risk::AuditSink`, writing override records to the hash chain.
//!
//! `AuditSink::write` is sync, the repo is async. A writer task spawned on the runtime owns the
//! repo; `write` hands it the draft and blocks on a reply with a bounded `recv_timeout`. Any
//! failure or timeout is an `AuditError` (fail closed: nothing is minted).
//!
//! The caller must run on a multi-thread runtime or call the gate from `spawn_blocking`: on a
//! blocked current-thread runtime the writer cannot run, so `write` fails by timeout (it never
//! hangs). The job queue is bounded (`try_send`); a full queue is an `AuditError`. A write that
//! timed out is cancelled if still queued; one already being appended may still land later as an
//! extra row. It is never reported as ok.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::time::Duration;

use bazaar_core::ports::{AuditDraft, AuditRepo, RepoError};
use bazaar_risk::audit::{AuditError, AuditRecord, AuditSink};
use bazaar_risk::override_::ReasonType;
use tokio::runtime::Handle;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Sender, channel};

use crate::audit::chain_hash;

struct Job {
    draft: AuditDraft,
    reply: std::sync::mpsc::SyncSender<Result<(), RepoError>>,
    /// Set by `write` on timeout; the writer then skips the append.
    cancel: Arc<AtomicBool>,
}

/// Stable names for the hash-chained payload; no wildcard, so a new variant must be named here.
const fn reason_name(r: ReasonType) -> &'static str {
    match r {
        ReasonType::NewInformation => "new_information",
        ReasonType::PlanChanged => "plan_changed",
        ReasonType::PaperExperiment => "paper_experiment",
        ReasonType::Other => "other",
    }
}

/// Sync sink over an async `AuditRepo` (see the module doc for the runtime rule).
#[derive(Debug)]
pub struct ChainAuditSink {
    tx: Sender<Job>,
    timeout: Duration,
}

impl ChainAuditSink {
    /// Spawn the writer on `handle`. `timeout` bounds every write; `queue` (>= 1) bounds the
    /// jobs waiting for the writer.
    pub fn new<R: AuditRepo + 'static>(
        repo: Arc<R>,
        handle: &Handle,
        timeout: Duration,
        queue: usize,
    ) -> Self {
        let (tx, mut rx) = channel::<Job>(queue.max(1));
        handle.spawn(async move {
            while let Some(job) = rx.recv().await {
                if job.cancel.load(Ordering::SeqCst) {
                    continue;
                }
                let r = repo.append(job.draft, chain_hash).await.map(|_| ());
                let _ = job.reply.send(r);
            }
        });
        Self { tx, timeout }
    }

    fn draft(record: &AuditRecord) -> AuditDraft {
        let rules: Vec<_> = record
            .rules()
            .iter()
            .map(|r| {
                serde_json::json!({
                    "rule": r.rule().to_string(),
                    "observed": r.observed(),
                    "limit": r.limit(),
                })
            })
            .collect();
        // serde_json's default map is sorted and `to_string` has no whitespace: canonical.
        let payload = serde_json::json!({
            "config_hash": record.config_hash().as_str(),
            "intent_digest": record.intent_digest().as_str(),
            "mode": record.mode().as_str(),
            "note": record.note(),
            "reason": reason_name(record.reason()),
            "rule_ids": record.rule_ids().iter().map(ToString::to_string).collect::<Vec<_>>(),
            "rules": rules,
        });
        AuditDraft {
            ts_ist_rfc3339: record.at().to_rfc3339(),
            actor: "user".to_owned(),
            kind: "override".to_owned(),
            payload: payload.to_string(),
        }
    }
}

impl AuditSink for ChainAuditSink {
    fn write(&self, record: &AuditRecord) -> Result<(), AuditError> {
        let (reply, wait) = sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        self.tx
            .try_send(Job {
                draft: Self::draft(record),
                reply,
                cancel: cancel.clone(),
            })
            .map_err(|e| match e {
                TrySendError::Full(_) => AuditError::new("audit queue full"),
                TrySendError::Closed(_) => AuditError::new("audit writer is gone"),
            })?;
        let out = wait.recv_timeout(self.timeout);
        if out.is_err() {
            cancel.store(true, Ordering::SeqCst);
        }
        match out {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(AuditError::new(format!(
                "append failed: {}",
                e.safe_reason()
            ))),
            Err(RecvTimeoutError::Timeout) => Err(AuditError::new("audit write timed out")),
            Err(RecvTimeoutError::Disconnected) => Err(AuditError::new("audit writer dropped")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::verify_chain;
    use crate::killswitch::KillSwitch;
    use crate::killswitch::fakes::{At, MemRepo, Shared};
    use bazaar_core::mode::ExecutionMode;
    use bazaar_core::money::Qty;
    use bazaar_risk::config::{Baseline, GateConfig, RuleCtx};
    use bazaar_risk::override_::{
        IntentDigest, OverrideChallenge, OverrideRequest, ReasonType, evaluate,
    };
    use bazaar_risk::snapshot::PortfolioSnapshot;
    use bazaar_risk::types::{OrderIntent, Side, Tri};
    use bazaar_risk::verdict::{GateVerdict, RuleId, RuleResult};
    use chrono::Duration as Cd;
    use rust_decimal::Decimal;
    use std::sync::atomic::Ordering::SeqCst;

    const TEXT: &str = include_str!("../../../fixtures/config/risk.toml");

    fn intent() -> OrderIntent {
        OrderIntent {
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: Qty(10),
            price: Decimal::from(100),
            stop: Some(Decimal::from(95)),
            new_stop: None,
            lot_size: 1,
            tick_size: Decimal::new(5, 2),
        }
    }

    struct Early(chrono::DateTime<chrono_tz::Tz>);
    impl bazaar_core::clock::Clock for Early {
        fn now_ist(&self) -> chrono::DateTime<chrono_tz::Tz> {
            self.0
        }
    }

    fn record() -> AuditRecord {
        use bazaar_core::clock::Clock;
        let cfg = GateConfig::from_toml_str(TEXT).unwrap();
        let now = At.now_ist();
        let ctx = RuleCtx {
            limits: cfg.effective(&Baseline::none()),
            mode: ExecutionMode::Paper,
            config_hash: cfg.hash().clone(),
            now,
        };
        let results: Vec<RuleResult> = RuleId::EVALUATION_ORDER
            .iter()
            .map(|&r| {
                if r == RuleId::G14 {
                    RuleResult::fail(r, "o", "l", "Too much.")
                } else {
                    RuleResult::pass(r, "o", "l")
                }
            })
            .collect();
        let v = GateVerdict::new(ctx.config_hash.clone(), &results)
            .unwrap()
            .bound_to(IntentDigest::of(&intent()));
        let wait = Cd::seconds(i64::from(ctx.limits.override_confirm_delay_secs));
        let ch = OverrideChallenge::open(&Early(now - wait), &intent(), &v).unwrap();
        let req = OverrideRequest {
            challenge: ch,
            reason: ReasonType::PlanChanged,
            note: "Earnings read changed my plan after the close.".into(),
        };
        let mut s = PortfolioSnapshot::unknown();
        s.overrides_today = Tri::Known(0);
        s.overrides_week = Tri::Known(0);
        let a = evaluate(req, &intent(), &v, &s, &ctx, &At).unwrap();
        AuditRecord::new(&a, ExecutionMode::Paper, &results, &At).unwrap()
    }

    fn sink(r: &Arc<MemRepo>, ms: u64) -> ChainAuditSink {
        ChainAuditSink::new(r.clone(), &Handle::current(), Duration::from_millis(ms), 8)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ack_path_writes_a_canonical_override_row_and_the_chain_verifies() {
        let r = Arc::new(MemRepo::default());
        let s = sink(&r, 2000);
        let rec = record();
        s.write(&rec).unwrap();
        s.write(&rec).unwrap();
        let ks = KillSwitch::new(
            Shared(r.clone()),
            Arc::new(At),
            "RELEASE KILL SWITCH".into(),
            true,
        )
        .unwrap();
        ks.engage("user", "x").await.unwrap();
        ks.release("RELEASE KILL SWITCH", "user").await.unwrap();
        s.write(&rec).unwrap();
        let rows = r.read_all().await.unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].draft.actor, "user");
        assert_eq!(rows[0].draft.kind, "override");
        assert!(rows[0].draft.ts_ist_rfc3339.ends_with("+05:30"));
        assert!(!rows[0].draft.payload.contains(": "));
        assert!(
            rows[0]
                .draft
                .payload
                .contains("\"reason\":\"plan_changed\"")
        );
        assert!(rows[0].draft.payload.starts_with("{\"config_hash\""));
        assert_eq!(verify_chain(&rows), Ok(()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn erroring_repo_and_corrupt_hash_are_audit_errors() {
        let r = Arc::new(MemRepo::default());
        let s = sink(&r, 2000);
        r.fail_write.store(true, SeqCst);
        assert!(s.write(&record()).is_err());
        r.fail_write.store(false, SeqCst);
        r.bad_hash.store(true, SeqCst);
        assert!(s.write(&record()).is_err());
        // verify_chain catches the wrong hash the fake stored.
        let rows = r.read_all().await.unwrap();
        assert!(verify_chain(&rows).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slow_repo_times_out_with_an_error() {
        let r = Arc::new(MemRepo::default());
        r.delay_ms.store(1000, SeqCst);
        let s = sink(&r, 50);
        let e = s.write(&record()).unwrap_err();
        assert!(e.detail().contains("timed out"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn writer_gone_is_an_error() {
        let s = ChainAuditSink {
            tx: channel(1).0,
            timeout: Duration::from_millis(50),
        };
        // receiver dropped at once: send fails.
        assert!(s.write(&record()).is_err());
    }

    #[tokio::test]
    async fn current_thread_runtime_fails_by_timeout_not_hang() {
        let r = Arc::new(MemRepo::default());
        let s = sink(&r, 100);
        let rec = record();
        let e = s.write(&rec).unwrap_err();
        assert!(e.detail().contains("timed out"));
    }

    #[test]
    fn reason_names_are_pinned() {
        assert_eq!(reason_name(ReasonType::NewInformation), "new_information");
        assert_eq!(reason_name(ReasonType::PlanChanged), "plan_changed");
        assert_eq!(reason_name(ReasonType::PaperExperiment), "paper_experiment");
        assert_eq!(reason_name(ReasonType::Other), "other");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn timed_out_in_flight_write_lands_late_but_queued_one_is_cancelled() {
        let r = Arc::new(MemRepo::default());
        r.delay_ms.store(300, SeqCst);
        let s = sink(&r, 30);
        let rec = record();
        assert!(s.write(&rec).is_err()); // in flight: will land late
        std::thread::sleep(Duration::from_millis(30));
        assert!(s.write(&rec).is_err()); // queued behind it: cancelled
        tokio::time::sleep(Duration::from_millis(900)).await;
        let rows = r.read_all().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(verify_chain(&rows), Ok(()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn full_queue_is_an_audit_error() {
        let r = Arc::new(MemRepo::default());
        r.delay_ms.store(500, SeqCst);
        let s = ChainAuditSink::new(r.clone(), &Handle::current(), Duration::from_millis(20), 1);
        let rec = record();
        assert!(s.write(&rec).is_err());
        std::thread::sleep(Duration::from_millis(50)); // writer now holds job 1
        assert!(s.write(&rec).is_err()); // occupies the one slot
        let e = s.write(&rec).unwrap_err();
        assert!(e.detail().contains("queue full"));
    }
}
