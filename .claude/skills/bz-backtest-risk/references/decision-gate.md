# DecisionGate: rule evaluation, results, overrides, risk.toml keys

Open this when adding or changing a rule, the gate's types, override handling, or `config/risk.toml`. The **policy** (which rules exist for a beginner, which are overridable, default thresholds, readiness bar) is owned by `bz-trading-discipline`; this file is the **mechanism** in `crates/risk`. Execution wiring (approval queue, re-check at approval time, audit log writes) is `bz-execution`.

## 1. Design

- The gate is a **pure function**: `gate.evaluate(&GateContext, &OrderIntent) -> GateVerdict`. No I/O, no wall clock, no globals. Everything it needs arrives in `GateContext` (clock reading, account snapshot, today's realised P&L, open positions, instrument master facts, ban list, events calendar, latest quote with its age, margin result, loaded `RiskConfig`).
- **All rules run** (no short-circuit) so the UI can show every failing check in one Why panel.
- **Fail closed**: missing or stale input (no quote, ban list not refreshed today, margin unavailable for F&O, backtest stats absent) is a failing result, never a pass.
- The same gate code runs in the backtester (simulated context, no overrides) and in paper and live pipelines. A rule that exists only in the live path is a bug.
- Order of rules is stable (a `Vec<Box<dyn Rule>>` built from config) so verdicts snapshot cleanly.

## 2. Types

```rust
pub trait Rule: Send + Sync {
    fn id(&self) -> RuleId;
    fn evaluate(&self, ctx: &GateContext, intent: &OrderIntent) -> RuleResult;
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct RuleId(&'static str);          // stable snake_case ids, see catalogue

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Severity { Hard, Overridable }   // derived from the discipline table's Override column, not set by the rule
// Hard ceiling, No -> Hard.  Soft -> Overridable.  Paper -> Overridable, plus a mode check at override time
// (OverrideError::NotAllowedInMode unless execution_mode is paper).

#[derive(Clone, Debug, serde::Serialize)]
pub enum RuleResult {
    Pass { id: RuleId },
    Fail {
        id: RuleId,
        severity: Severity,
        reason: String,                    // plain English, rupees first, for the Why panel
        observed: Option<Decimal>,
        limit: Option<Decimal>,
    },
}

pub struct GateVerdict { pub results: Vec<RuleResult>, pub evaluated_at: DateTime<Tz>, pub config_hash: String }
impl GateVerdict {
    pub fn passed(&self) -> bool;                       // no Fail at all
    pub fn hard_failures(&self) -> impl Iterator<Item = &RuleResult>;
    pub fn overridable_failures(&self) -> impl Iterator<Item = &RuleResult>;
}
```

`GateVerdict` is the one gate-result name everywhere (approval queue, UI, audit); a `Recommendation` stores a `GateSummary` derived from it, never a second verdict type.

The only way to obtain something the pipeline may send is a type the gate mints:

```rust
pub struct GatedIntent { intent: OrderIntent, verdict: GateVerdict, overrides: Vec<Override> }  // private fields
impl Gate {
    pub fn approve(&self, v: GateVerdict, i: OrderIntent) -> Result<GatedIntent, GateRefusal>;   // only if passed
    pub fn approve_with_override(&self, v: GateVerdict, i: OrderIntent, o: Override)
        -> Result<GatedIntent, OverrideError>;
}
```

`execution` turns a `GatedIntent` plus a user approval (gate re-run with a fresh quote) into an `ApprovedOrder`, the only type `Executor::place` accepts (see `bz-execution`), so a bypass does not compile. No `skip_gate`, no `GatedIntent::new` outside the module, no `Default`, no `From<OrderIntent>`.

## 3. Overrides

```rust
pub struct OverrideReason { pub code: OverrideCode, pub note: String }   // OverrideCode variants: bz-trading-discipline section 3
pub struct Override { pub rule_ids: Vec<RuleId>, pub reason: OverrideReason, pub by: UserId, pub at: DateTime<Tz> }
pub enum OverrideError { HardFailurePresent(RuleId), ReasonTooShort { min: usize }, DoesNotCoverFailures(Vec<RuleId>), NotAllowedInMode, CapReached }
```

- Allowed only when every failing result is `Overridable` and the override names each of them.
- `reason.note` is trimmed free text, at least `override.min_reason_chars` characters (30), and not a duplicate of the previous override's text (cheap guard against pasted reasons); a code without a note is rejected.
- Never allowed in `backtest` mode or in `auto` mode.
- The crate has no I/O: it returns an `OverrideRecord` (verdict, reason, user, timestamp, config hash) that `bz-execution` appends to the hash-chained audit log, and that the journal analytics use to compare override trades against system trades.

## 4. Rule catalogue and risk.toml keys

**Policy source of truth: the G01-G16 table in `bz-trading-discipline`** (rule id, check, `risk.toml` key, default, Override level: Hard ceiling / No / Soft / Paper). It is the only key list: implement exactly that table, do not invent or restate keys, and do not change the override levels here. Use the ids as `RuleId` variants (`G01EvidenceBar`, `G02PerTradeRisk`, ...) so audit entries, the Why panel and the journal all name the same rule.

Implementation-only rules the gate also runs (always **hard**, no config switch):

| id | Fails when | Reads |
|---|---|---|
| `X01 market_open` | outside a session the product allows, or instrument not tradable now | clock, calendar |
| `X02 lot_and_tick` | qty not a multiple of the master lot size, price off tick, or outside the circuit band | instrument master |
| `X03 margin` | F&O margin not fetched from the broker margin API, or insufficient (never hand-estimated) | margin result |

Code-level `NEVER_OVERRIDABLE` = every rule `bz-trading-discipline` marks Hard ceiling or No, plus X01-X03. The loader rejects a `risk.toml` that tries to make one of them overridable or to loosen a Hard ceiling rule past `[hard_limits]`.

Like every config file, `risk.toml` starts with `schema_version`, `verified_on` and `source` (`source = "owner"`; there is no `reviewed_on`). The per-unit cost buffer used in sizing comes from the cost model and slippage settings (`charges-toml.md`, `strategies.toml`), not from a `risk.toml` key.

Loader rules: `deny_unknown_fields`; percentages in (0, 100]; `limits.max_risk_per_trade_pct <= limits.max_daily_loss_pct`; money values as Decimal strings; an invalid file is a startup error and the app blocks all entries (never falls back to built-in defaults).

## 5. Tests

- One table-driven `rstest` per rule: pass, fail, boundary exactly at the limit (limit itself passes or fails as documented), missing input (fails closed).
- Verdict snapshot with `insta` for a multi-failure intent (stable order, plain-English reasons).
- Override tests: hard failure refuses; short note or missing code refuses; override must cover every failing id; `backtest` and `auto` modes refuse; a Paper rule refuses outside paper mode.
- Type-level: a `trybuild` or doc-test `compile_fail` showing `GatedIntent` cannot be built outside the crate.
- Property: for any intent and config, `passed()` implies `per_trade_risk` and `daily_loss` both passed; evaluating twice with the same context gives an identical verdict (determinism).
