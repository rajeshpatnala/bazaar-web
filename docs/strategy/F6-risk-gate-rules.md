# F6 risk crate: which gate rules and guards to build (market-strategist pre-plan, 2026-11-19)

Not investment advice. Rules limit damage; none of them makes a trade safe. Owner: Rs 1,00,000, 1% risk/trade
(Rs 1,000), 3% daily loss (Rs 3,000), paper mode. Policy source of truth for ids, keys and Override levels is the
G01-G16 table in `bz-trading-discipline`; this spec only picks scope and defaults. It invents no override levels.

## 1. Build now (F6) vs later

| Build in F6 | Why |
|---|---|
| G02 per-trade risk + sizing, G03 daily loss, G04a/b open positions + total open risk | The core capital protection; backtest (F9) and execution (F10) both need them. |
| G08 cool-off, G09 no stop widening, G10 entry zone only, G11 no averaging down | Behaviour guards: cheap to code, they target the beginner's most expensive habits. |
| G13 naked short rejected, G14 position cap, G16 mode / kill switch / order rate, X02 lot+tick | Pure checks on the intent and snapshot. |
| G05 sector + correlation | Pure function on a supplied sector map and return series; no data fetch in F6. The gate also requires every return series to end within 5 calendar days of the snapshot date (constant `MAX_RETURN_LAG_DAYS` in `gate.rs`, not config) and not after it; otherwise G05 rejects as a missing input. |
| G01 evidence bar, G06 ban list, G07 events, G15 freshness, X01 market open, X03 margin | Build the rule against an input struct; **missing input = Reject** (fail closed). Real data wiring lands with F3/F9/F10. Until then they reject, which is the safe default. |
| Override engine (typed reason, 30-char note, caps, size_factor) | Caps must live in `risk`. Audit write goes through an `AuditSink` trait; if it fails, the override fails. F10 supplies the real sink. |

Defer: G12 liquidity (needs real ADV/OI data, F3), paper-to-live readiness bar (F10+), override analytics (journal UI).
All rules run (no short-circuit) so the user sees every reason at once; order of evaluation per the skill.
Every verdict carries the config hash. Guard state (consecutive losses, today's P&L, last stop-out times) comes from
the portfolio snapshot built from the journal, never from process memory, so a restart cannot wipe a lockout.

## 2. Sizing method

Fixed-fractional, as in `bz-backtest-risk` position-sizing: `qty = floor((capital x risk% - charges/slippage buffer) / per-unit risk)`,
lots floored, re-check so rounding never exceeds budget, `BudgetBelowOneLot` = "no trade" (never one lot anyway). Then
`qty = min(risk-sized, notional-cap-sized, cash/margin-sized)`, again floored. Capital is current equity (starting cash plus
realised P&L), so size shrinks after losses and is never scaled up to "win it back".
Worked example (cash equity, no leverage): entry Rs 500, stop Rs 490, buffer Rs 60 total: qty 94, risk about Rs 1,000, notional Rs 47,000.
That breaks G14 (25% = Rs 25,000), so the qty drops to 50 and real risk is about Rs 500 + buffer. Tight stops on a small account
will often be capped by G14, not by G02. That is intended; tell the user ("size reduced to fit your 25% position cap").
Rejected for sizing: Kelly, volatility-targeting, pyramiding, any increase after a loss or a win streak. Overridden trades: size x `override.size_factor`.
Options: bought option risk = premium x qty; spreads = payoff-engine max loss; naked shorts never auto-sized (G13).

## 3. Rules, keys, defaults

Every default below is a proposal. Mark each new key in `risk.toml` as `# source = owner-to-confirm, verified_on = unverified`;
do not bump the file-level `verified_on` (2026-11-18) until the owner confirms. Only the two `[limits]` values and their
`[hard_limits]` are currently owner-verified. Code holds no threshold literals (review blocker).

| Id | Key | Proposed default | Override | Owner confirm |
|---|---|---|---|---|
| G02 | `limits.max_risk_per_trade_pct` | 1 (existing, ceiling 1) | Hard | already set |
| G03 | `limits.max_daily_loss_pct`, `on_daily_loss` | 3 (existing), `lockout_entries` | Hard | already set |
| G04a | `limits.max_open_positions` | 3 (consider 2 while learning) | Soft | yes |
| G04b | `limits.max_total_open_risk_pct` (+ `[hard_limits]` entry) | 3 | Hard | yes |
| G05 | `exposure.max_sector_pct`, `max_correlated_positions`, `correlation_threshold`, `correlation_window_days` (+ `[hard_limits] min_correlation_window_days`, floor 20) | 30, 2, 0.7, 60 | Soft | yes |
| G08 | `behaviour.cooloff_after_losses`, `cooloff_until` | 3 (see Q1), `end_of_session` | No | yes |
| G09 / G11 | `behaviour.allow_stop_widening`, `allow_averaging_down` (+ `[hard_limits]`) | false; loader refuses true | Hard | yes |
| G10 | `behaviour.entry_zone_tolerance_bps` | 0 | No | yes |
| G13 | `options.allow_naked_short` | false | No | yes |
| G14 | `limits.max_position_value_pct` | 25 | Soft | yes |
| G16 | `hard_limits.max_orders_per_minute`, `limits.max_orders_per_day`, `max_notional_per_order_inr` | set from broker/SEBI limits (cite source); 12/day; Rs 25,000 | Hard | yes |
| Override | `override.max_per_day`, `max_per_week`, `size_factor`, `confirm_delay_secs` | 1, 3, 0.5, 10 | n/a | yes |

G01 keys (30 trades, PF 1.3, 0.1 R, walk-forward pass) stay as in the table; F6 only reads them. Broker order-rate numbers must be
looked up and cited (bz-india-markets) when set; do not quote from memory.

### Proposed additions (not in the G-table; need owner OK and a skill-table update by the manager before dev-quant codes them)
| Id | Guard | Key | Default | Override |
|---|---|---|---|---|
| G17 revenge-trade | After a losing exit in an instrument, no new entry in the same instrument for N minutes | `behaviour.reentry_lockout_minutes` | 60 | No |
| G18 overtrading | New entries per day cap (entries only; exits never blocked) | `behaviour.max_new_entries_per_day` | 3 | No |
| G19 weekly loss | Realised+unrealised net loss in the ISO week at the limit locks entries until next Monday | `limits.max_weekly_loss_pct` (+ ceiling) | 6 | Hard |
Later, optional: no new entries in the first 15 minutes of the session (opening volatility); needs owner view.

## 4. Behaviour-guard design notes
- Loss counting is net of charges and uses IST session days via `Clock`; a day with a full-loss trade at 1% plus charges can hit 3% before three
  losses (charges are real). G03 counts realised plus unrealised, so an open losing trade counts before it is closed.
- G03 lockout blocks entries only. Exits, and stop moves toward safety, always stay allowed (never trap the user in a loser).
- Tightening applies instantly; loosening takes effect next session with typed confirmation (contract item 7). F6 defines a
  `LimitsBaseline` trait (in-memory fake now, persisted in F10) so "raise the limit at 2 pm after a loss" is impossible.
- Guard thresholds are policy, not fitted parameters. Do not tune them on backtest P&L. Backtest runs them without overrides
  so F9 shows what the guards cost and save; judge guards on drawdown and worst day, not on return.

## 5. Reject list (do not build)
Martingale or any size increase after a loss; averaging down; widening a stop; pyramiding for now; Kelly or leverage-based sizing; any
"bypass", "force" or "skip gate" flag, env var or CLI switch; mode changed anywhere but `risk.toml`; making cool-off, revenge or daily-loss
overridable; letting the UI supply qty or rupee risk (gate re-derives); reading state from memory instead of the journal; fail-open on missing
data, stale quote or config parse error; f64 money; ML or LLM inside the gate; auto-resetting a lockout at restart or at midnight UTC;
raising a ceiling without ADR plus owner approval; auto-selling naked options "defined by stop".

## 6. Block-reason wording (rupees first, plain English, no shaming)
- G02: "This trade could lose about Rs 1,240 if your stop is hit. Your limit per trade is Rs 1,000. Try a smaller size or a closer stop." One lot over budget: "This setup needs more money than your risk limit allows, so there is no trade here."
- G03: "You are down Rs 3,050 today, past your Rs 3,000 daily limit. No new trades until tomorrow. You can still close positions."
- G04a/b: "You already have 3 open trades. New trades wait until one closes." / "Your open trades already risk Rs 2,600 in total; this one adds Rs 900, above your Rs 3,000 cap."
- G05: "This stock moves much like one you already hold (HDFCBANK). Two similar bets act like one big bet."
- G08: "Three losing trades in a row. Trading is paused until tomorrow so you can step back."
- G09: "Moving your stop further away raises your loss. This is not allowed. You can move it closer to lock in profit."
- G10: "Price has moved past the planned entry range. The setup is missed; wait for a fresh scan. Chasing raises your risk."
- G11: "This trade is below your buy price. Adding more would increase a losing bet, so it is blocked."
- G13: "Selling options without a cap on loss can cost far more than your account holds. Only capped-loss structures are allowed."
- G14: "This position would be Rs 40,000, above your Rs 25,000 single-position cap. Size reduced / blocked."
- G17: "You were just stopped out of this stock. Wait 60 minutes; quick re-entries after a loss usually come from emotion, not a new signal." G18: "You have used today's 3 new trades. Fewer, better trades."
- G01/G06/G07/G15/X03 missing data: "Cannot confirm [item]. When unsure, the app says no." Each also shows what would make it pass.
Every message ends with the rule id for the audit trail. Never "safe", "sure" or "guaranteed".

## 7. Evidence bar for F6 itself (quant-reviewer and risk-guardian)
Property tests: sizing never exceeds budget at the stop; qty is a lot multiple; Decimal only; same inputs give same verdict; every rule
fails closed on missing input; no Hard/No rule becomes overridable; loader refuses ceiling breaches and `true` for the two forbidden flags.
Replay tests: sequences of 3 losses, gap-down through stop, restart mid-lockout, week boundary, Rs 1,000 budget with one lot above budget.

## 8. Open questions for the owner
1. G08 at 3 losses equals your whole 3% daily limit, so it adds little within one day. Use 2 losses (pause for the day) instead, or keep 3 counted across days?
2. Max open positions: 3, or 2 while you learn (total risk then 2%)?
3. Approve G17 (60 min re-entry lockout), G18 (3 new entries/day) and G19 (weekly loss 6%)?
4. G14 at 25% will often shrink your size on tight stops. Keep 25%, or allow up to 33%?
5. Confirm all other proposed defaults above, and who adds the new keys to `risk.toml` (this agent does not edit config).
