# 4. Order pipeline (identical in every mode)

(From `bz-architecture` SKILL.md; open when the task touches this area.)


```
Signal -> Recommendation -> OrderIntent -> DecisionGate -> Approval -> Executor -> Fill -> Position + Journal + Audit
 (strategies)  (scanner)     (execution)     (risk)      (user click)  (Paper|Live)
```

| Stage | Contract |
|---|---|
| Signal | strategy output from past data only; no sizing, no money |
| Recommendation | fields owned by `bz-quant` (`RecommendationDto`); includes a "low evidence" flag under about 30 trades |
| OrderIntent | what the user may approve: concrete price, qty, product, validity, rupee risk, `intent_id` (the broker idempotency tag is derived from it, see `bz-execution` `references/live-phase.md`) |
| DecisionGate | rules-only checklist from `config/risk.toml`; returns a `GateVerdict` (`bz-backtest-risk` `references/decision-gate.md`); no ML, no discretion; override needs typed reason and is audited |
| Approval | `paper` and `semi_auto`: user click per order. `auto`: policy engine, locked (section 5). Gate runs again at approval time with fresh prices; approvals expire after `approval_ttl_secs` |
| Executor | one trait, two impls; the pipeline cannot tell them apart |
| Fill | updates position, journal entry, audit record in one SQLite transaction |

Design points:
- A **single-writer pipeline task** (tokio task owning an `mpsc` command queue) serialises intents, gate checks, kill-switch reads and sends. Daily-loss and position-count checks race otherwise. HTTP handlers send commands to it and await the reply; with two tabs open, two approvals for one intent resolve as first wins, the second gets a conflict.
- Kill switch and daily-loss lockout are persisted flags read by the pipeline before every `place`; they are server state, not a UI nicety.
- Every state change appends to the hash-chained audit log before the side effect it describes (write-ahead).
- Each stage is a plain function over values, so a property test can feed random intents through the pipeline with a `RecordingExecutor` and assert nothing was placed after a `Block`, after kill switch, or without approval.
