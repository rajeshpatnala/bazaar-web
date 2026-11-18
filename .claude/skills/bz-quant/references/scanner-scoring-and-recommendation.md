# 7. Scanner scoring and Recommendation

(From `bz-quant` SKILL.md; open when the task touches this area.)


Pipeline: regime -> universe filter (liquidity, ban list, event calendar) -> run enabled strategies per instrument (rayon) -> merge signals per instrument/side -> score -> size via `risk` -> portfolio limits via `risk` -> rank -> Why text.

Score 0-100 as a weighted sum, weights in `strategies.toml` (default shape, tune with evidence, not taste):

| Component | Idea |
|---|---|
| Confluence | Distinct strategy *families* agreeing (trend, momentum, breakout, volume ...). Two EMA variants count once |
| Regime fit | Strategy `regime_fit` vs current `RegimeState`; wrong regime gives 0 and usually drops the signal |
| Liquidity | Turnover/volume percentile; for options bid-ask spread % and OI; below the floor drops the signal |
| Backtest quality | Profit factor, expectancy in R, max drawdown, walk-forward stability from `backtest` stats on *this instrument*, shrunk toward the strategy-wide stats when trades are few |
| Event penalty | Results day, expiry day for options, ex-dates, RBI/Budget (from `events.toml`); ban-list names are excluded, not penalised |

**This is the only field list for `Recommendation`.** `RecommendationDto` is derived from it as a utoipa DTO in the OpenAPI schema (`bz-web-api`), and the UI types are generated with `openapi-typescript` into `web/src/api/schema.d.ts` (`bz-web-ui`); no other skill restates the fields, and a snapshot test of the OpenAPI document guards drift.

```rust
pub struct Recommendation {
    pub id: Uuid, pub generated_at: DateTime<Tz>, pub data_as_of: DateTime<Tz>,
    pub instrument: InstrumentRef,
    pub action: Action,                        // Buy | Sell | OptionStructure(..) | NoTrade
    pub entry_zone: (Decimal, Decimal),
    pub stop_loss: Decimal,
    pub targets: Vec<Decimal>,
    pub horizon: Horizon,
    pub size: PositionSize,                    // qty or lots, from risk crate
    pub risk_rupees: Decimal,                  // size x stop distance, incl. estimated charges
    pub reward_risk: Decimal,                  // to first target; also per target in `targets_rr`
    pub agreeing_strategies: Vec<StrategyId>,
    pub confidence: u8,                        // 0-100 score, not a probability
    pub regime: RegimeState,
    pub backtest: BacktestStats,               // trades, win_rate, profit_factor, avg_r, max_drawdown (+ period, costs applied)
    pub low_evidence: bool,                    // backtest.trades < min_trades_evidence (default 30, from config)
    pub why: Vec<WhyFactor>,                   // structured; UI/text renderer produces plain English, risk in rupees first
    pub warnings: Vec<Warning>,                // naked short, ratio spread, event nearby, stale data
    pub gate: GateSummary,                     // derived from the DecisionGate's GateVerdict; blocked ones are shown as blocked, not hidden
}

/// Compact, serialisable view of a `GateVerdict` (defined in `bz-backtest-risk` `references/decision-gate.md`).
/// Built only via `GateSummary::from(&GateVerdict)`; it is not a second verdict type and cannot approve anything.
pub struct GateSummary { pub passed: bool, pub hard_failures: Vec<RuleId>, pub overridable_failures: Vec<RuleId>,
                         pub evaluated_at: DateTime<Tz>, pub config_hash: String }
```

Rules: every field from product Hard rule 6 is mandatory (a constructor `Recommendation::new` returns `Err` on a missing one, there is no `Default`). `low_evidence` also caps `confidence` (cap value in config) and forces the badge. If no candidate clears the score floor and the gate, `scanner` returns `ScanResult { recs: vec![], verdict: NoHighQualitySetups, reasons }`. Why text is generated from `facts`/`WhyFactor` by a pure function so it can be snapshot-tested; wording follows `bz-market-strategy`, with the permanent disclaimer owned by the UI (`bz-web-ui`).
