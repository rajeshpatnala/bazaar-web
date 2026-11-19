# F6: Risk crate

Status: building (slices 0, 1, 1b, 2, 3 done; slice 6 behaviour guards and baseline in review). Plan: `.agents/plans/F6-risk-crate.md`. Owner area: `crates/risk`, `config/risk.toml`.
Summary: deterministic DecisionGate, sizing, portfolio and behaviour rules, override engine in `crates/risk`; keys in `risk.toml`.
Decisions: owner 2026-11-19 (G08 = 2 losses, until end of session; max 3 positions; G17/G18/G19 approved; G14 cap 25%; No-rows loosenable only within `[hard_limits]` ceilings; sizing equity capped at starting cash); fail closed by type; gate classifies Entry/Exit/StopMove; one override-level table; `GatedIntent` mintable only in `gate.rs`; config hash = sha256 of file text. Skill G-table updated first (own commit) so reviewers check against it.
Known limits: G12, readiness bar, real audit/baseline/journal deferred; in-memory baseline loosens on restart until F10; inputs for G01/G06/G07/G15/X01/X03 reject until F3/F9/F10; every key not owner-decided is `owner-to-confirm, unverified`; file-level `verified_on` (2026-11-18) not bumped.
F10 handoff: snapshot built only from the journal (journal read failure = no snapshot = no order); G10 entry zone from the stored recommendation, not the request; gate re-run and expiry at approval via `evaluated_at` and config hash.
Deferred (slice 9, slice 11 / F10): `user_approval` bound to the intent digest; `live_trading_compiled` from build info (`cfg!`), not request data; auto-mode unlock inputs from the journal/evidence store; gate computes `notional_price = max(intent, live)` once for `size` and `g16`, and never calls `size` when `is_sizable` is false (option entries rejected until options sizing exists); F10 builds `orders_today`, `orders_last_minute`, `kill_switch` into the snapshot. Owner to confirm: kill switch lets reduce-only exits through (default behaviour per skill).
Deferred: RISK-52-6 G05 return-series freshness (series end date within N sessions of `as_of`) belongs to the slice 11/F10 input contract; RISK-52-7 loader floor for `correlation_window_days` (with `[hard_limits] min_correlation_window_days`) is a core config change for the config owner (board item); RISK-49-2 (same as RISK-50-1; G11 already uses the worst of live and intent price); RISK-51-11 (slice 11 gate carry-over: build `OptionKind` only via `from_instrument`; never call `size` when `is_sizable` is false; compute `notional_price = max(intent, live)` once for `size` and `g16`; `live_trading_compiled` from build info and `user_approval` bound to the intent digest; tests for each); RISK-50-1 gate passes one worst-case reference price to size, G02, G10, G11 (slices 5, 11); RISK-50-2 gate derives `overridden` from an accepted override in the same verdict, calls `size` itself, and F9 computes cost for the final qty (slices 10, 11); RISK-50-3 slices 8/9 reject above cash/margin and G16 notional and test a sized order passes, G14 counts a same-symbol position (slices 5, 8, 9); first-15-minute rule; broker order-rate figure and the new hard ceilings (G04b 3, override caps, orders/min) stay Owner actions until confirmed; slice 1 values (`[hard_limits]` G05/G04a/override/orders-per-minute ceilings, `max_orders_per_minute` 10, charges floor 20) are placeholders, unverified, owner to confirm.
Deferred (slice 10 review): the override count update belongs to F10 (journal) and single use to slice 11b: the gate must bind the verdict with `GateVerdict::bound_to(IntentDigest::of(intent))`, write and acknowledge the audit record, and bump the count before the next `evaluate` (11b test: a sink that bumps the count makes a second override on the same intent fail; serialise evaluate, audit write and count update; RISK-55-4, SEC-55-3, CR-55-3). Rules that fail on a missing input must use `RuleResult::input_missing` (G05 does; slice 8 G07 and the rest must), or the override engine would treat them as real breaches (RISK-55-1).
Deferred: RISK-52-6 G05 return-series freshness (series end date within N sessions of `as_of`) belongs to the slice 11/F10 input contract; RISK-52-7 done in slice 1c (`[hard_limits] min_correlation_window_days` = 20, unverified, owner to confirm; loader refuses `exposure.correlation_window_days` below it); RISK-49-2 (same as RISK-50-1; G11 already uses the worst of live and intent price); RISK-51-11 (slice 11 gate carry-over: build `OptionKind` only via `from_instrument`; never call `size` when `is_sizable` is false; compute `notional_price = max(intent, live)` once for `size` and `g16`; `live_trading_compiled` from build info and `user_approval` bound to the intent digest; tests for each); RISK-50-1 gate passes one worst-case reference price to size, G02, G10, G11 (slices 5, 11); RISK-50-2 gate derives `overridden` from an accepted override in the same verdict, calls `size` itself, and F9 computes cost for the final qty (slices 10, 11); RISK-50-3 slices 8/9 reject above cash/margin and G16 notional and test a sized order passes, G14 counts a same-symbol position (slices 5, 8, 9); first-15-minute rule; broker order-rate figure and the new hard ceilings (G04b 3, override caps, orders/min) stay Owner actions until confirmed; slice 1 values (`[hard_limits]` G05/G04a/override/orders-per-minute ceilings, `max_orders_per_minute` 10, charges floor 20) are placeholders, unverified, owner to confirm.

## Slice 0 config keys: loader bounds (inclusive; load error outside)

| Key | Default | Bound | Status |
|---|---|---|---|
| `limits.max_open_positions` (G04a) | 3 | 1..=20 | owner 2026-11-19 |
| `limits.max_total_open_risk_pct` (G04b) | 3 | (0,100], <= ceiling | unverified |
| `limits.max_weekly_loss_pct` (G19) | 6 | (0,100], <= ceiling, >= daily limit | owner 2026-11-19 |
| `limits.max_position_value_pct` (G14) | 25 | (0,100] | owner 2026-11-19 |
| `limits.on_daily_loss` (G03) | lockout_entries | enum, one value | existing policy |
| `hard_limits.max_total_open_risk_pct` / `max_weekly_loss_pct` | 3 / 6 | (0,100] | unverified / owner |
| `exposure.max_sector_pct` (G05) | 30 | (0,100] | unverified |
| `exposure.max_correlated_positions` | 2 | 1..=10 | unverified |
| `exposure.correlation_threshold` | 0.7 | (0,1] | unverified |
| `exposure.correlation_window_days` | 60 | 20..=250, >= `hard_limits.min_correlation_window_days` | unverified |
| `behaviour.cooloff_after_losses` (G08) | 2 | 1..=10, <= `hard_limits.max_cooloff_after_losses` (2) | owner 2026-11-19 |
| `behaviour.cooloff_until` | end_of_session | enum, one value | unverified |
| `behaviour.allow_stop_widening` (G09), `allow_averaging_down` (G11) | false | `true` refused | unverified |
| `behaviour.entry_zone_tolerance_bps` (G10) | 0 | 0..=50, <= `hard_limits.max_entry_zone_tolerance_bps` (0) | unverified |
| `behaviour.reentry_lockout_minutes` (G17) | 60 | 1..=480, >= `hard_limits.min_reentry_lockout_minutes` (60) | owner 2026-11-19 |
| `behaviour.max_new_entries_per_day` (G18) | 3 | 1..=20, <= `hard_limits.max_new_entries_per_day` (3) | owner 2026-11-19 |
| `hard_limits.max_cooloff_after_losses`, `max_new_entries_per_day`, `min_reentry_lockout_minutes`, `max_entry_zone_tolerance_bps` | 2, 3, 60, 0 | same ranges as the key they bound | ceilings equal defaults; changing one needs an ADR and the owner |

## Slice 1 config keys (inclusive bounds; load error outside; all unverified unless noted)

| Key | Default | Bound |
|---|---|---|
| `hard_limits.max_position_value_pct` (G14 ceiling) | 25 (owner) | (0,100]; limit <= ceiling |
| `hard_limits.max_sector_pct` (G05), `max_open_positions` (G04a) | 30, 3 | (0,100], 1..=20; limit <= ceiling |
| `limits.max_orders_per_minute` / `hard_limits.max_orders_per_minute` (G16) | 10 / 10 | 1..=600; no cited broker source: owner action |
| `hard_limits.max_override_per_day` / `max_override_per_week` | 1 / 3 | 1..=10 / 1..=50 |
| `override.max_per_day` / `max_per_week` | 1 / 3 | same ranges, <= ceiling, week >= day |
| `override.size_factor` | 0.5 | 0 < x <= 1 |
| `override.confirm_delay_secs` | 10 | 1..=300 |
| `gate.evidence.min_oos_trades` (G01) | 30 | 1..=100000, >= `hard_limits.min_oos_trades` |
| `hard_limits.max_ban_list_age_hours` / `max_quote_age_secs` / `max_snapshot_age_secs` | 24 / 5 / 10 | 1..=48 / 1..=60 / 1..=300; limit <= ceiling |
| `hard_limits.latest_expiry_day_no_new_after_ist` | 13:30 | 09:15..=15:30 (also bounds `events.expiry_day_no_new_after_ist`); limit <= ceiling |
| `hard_limits.min_correlation_window_days` (slice 1c) | 20 | 20..=250 | unverified |
| `hard_limits.min_confirm_delay_secs` / `max_size_factor` / `min_oos_trades` | 10 / 0.5 / 30 | 1..=300 / (0,1] / 1..=100000; limit >= floor, <= ceiling |
| `gate.evidence.min_profit_factor`, `min_expectancy_r` | 1.3, 0.1 | >= 1, > 0; `require_walk_forward_pass = false` refused |
| `events.block_ban_list`, `block_results_days`, `block_policy_days` | true | `false` refused |
| `events.ban_list_max_age_hours` | 24 | 1..=48 |
| `events.expiry_day_no_new_after_ist`, `events_file` | "13:30", events.toml | `HH:MM`; relative path, no `..` |
| `data.max_quote_age_secs` (G15) | 5 | 1..=60 |
| `data.max_snapshot_age_secs` (RG-F6-15, own key) | 10 | 1..=300 |
| `options.allow_naked_short` (G13) | false | `true` refused |
| `limits.max_orders_per_day` / `max_notional_per_order_inr` (G16) + ceilings | 12 / 25000 | 1..=1000 / > 0; limit <= ceiling |
| `limits.min_charges_slippage_buffer` | 60 (rupees) | > 0; placeholder, derive from charges.toml with the F9 cost model |
| `limits.cap_equity_at_starting_cash` | true | `false` refused (owner Q2) |

Slice 1b `[hard_limits]` (unverified, owner to confirm): `max_ban_list_age_hours` 24, `max_quote_age_secs` 5, `max_snapshot_age_secs` 10, `latest_expiry_day_no_new_after_ist` 13:30, `min_confirm_delay_secs` 10, `max_size_factor` 0.5, `min_oos_trades` floor 30 (equals the G01 default), `min_correlation_window_days` floor 20 (added in 1c). A higher floor such as 40 is the owner's call (20 matches the loader minimum on purpose). Deferred: symlink and canonical-path check for `events_file` belongs to the events loader (F3/F10).

Ceilings for G05 and G04a sit at their defaults; changing any `[hard_limits]` value needs an ADR and the owner.
Test fixture `fixtures/config/risk.toml` gained the same sections with synthetic values.

## Slice 3 decisions
- X01 (market open), X02 (lot and tick) and X03 (margin) are not in the skill G-table. Their override level is **No** in `RuleId::override_level()`. Owner to confirm.
- G12 (liquidity) has no `RuleId` (outside F6).
- Keys without a baseline entry (quote and snapshot age, size factor, confirm delay, correlation, evidence, event flags) can be loosened within their ceilings or bounds mid-session until F10 persists a baseline for them.

## Slice 6: behaviour guards and baseline
- `rules/behaviour.rs`: G08, G09, G10, G11, G17, G18 as pure functions over `RuleCtx`, the classification and the snapshot. Entry-only rules pass for exits and stop moves; unknown state rejects entries; G09 rejects an unknown stored stop or unknown positions.
- G10 contract for F10: the entry zone (`EntryZone`) comes from the stored recommendation and the live price is a separate input; both `Tri`. Price must sit inside the zone widened by `entry_zone_tolerance_bps` on both sides (below the zone also rejects: conservative choice, owner to confirm).
- G11 needs `Position.avg_cost` (new `Tri` in the snapshot). More than one open row for a symbol, or an unknown average cost, rejects an add.
- G17: a losing-exit time later than now rejects (clock or journal fault). `PortfolioSnapshot::last_losing_exits` is the last losing exit per symbol, net of charges, however closed (stop, target or manual); F10 journal must supply it so (RISK-49-3).
- G11 takes the live price (`Tri<Decimal>`, unknown rejects an add) and judges the worse of live and `intent.price` against average cost (RISK-49-1).
- `GateConfig::rule_ctx` is `pub(crate)`; `baseline::rule_ctx` is the only public path (RISK-49-4). Slice 11 test: failing store gives Reject and no `GatedIntent`.
- Slice 5 contract (RISK-49-5): G02 sizes risk on the combined position (an add with a wider own stop raises aggregate risk, G09 sees only the new stop) or rejects; pyramiding in profit stays out of scope.
- `baseline.rs`: `LimitsBaseline` trait, `rule_ctx` (read error returns `Err`: the gate must Reject, never fall back to the file value), `FakeBaseline`, `MemoryBaseline` (tightens only; loosening next session with typed confirmation and persistence are F10). The `EffectiveLimits` digest in the verdict (RISK-48-4) stays with slice 11.

## Slice 7 (G05 sector and correlation, Soft)
- Equity from `sizing_equity(snapshot, limits)` and marks from `snapshot.marks` (no caller figures); sector cap compared unrounded; entry valued at `ref_price` = `max(intent.price, live price)` computed once by the gate and passed to `evaluate`.
- Correlation is side-aware: a held position counts when `s * cov > 0` (s = +1 same side, -1 opposite) and `cov^2 >= t^2 var_a var_b`. Positively correlated opposite-side pairs are hedges and are not counted. Owner to confirm: hedges are not counted.
- Returns too small to compare (squares round to 0) reject; a window below 2 rejects with its own message.

## Slice 8: input rules (G01, G06, G07, G15, X01, X03)
`rules/inputs.rs`; every input is `Tri` and Unknown rejects until F3/F9/F10 supply it. G06 needs a non-empty list fetched on the current IST date and under `ban_list_max_age_hours` old, trims names, and is always enforced (no off switch); G07 is always enforced and compares the expiry cutoff in IST. `DataInputs.inputs_as_of` is checked by G15 against `max_snapshot_age_secs`, so session, events, evidence and margin figures cannot be reused stale. `MarginInputs.qty` must equal the entry quantity (X03). G15 has no bar-age check (no config key). G01 Paper level is enforced by the override engine (slice 10). F3 must return Unknown, not an empty list, on a failed ban-list fetch. Slice size 739+ lines, owner-approved as one PR.
## Slice 5 (portfolio rules: G02, G04a, G04b, G14)
- `rules/portfolio.rs`: pure fns taking the gate's one `ref_price`. G02 = new risk + max(charges buffer, floor) + held risk of a same-side holding (stored stop) vs `max_risk_per_trade_pct`; G04b = new risk + risk of surviving positions; G14 = new qty at ref plus same-side holding at its mark. Stops in profit count as 0 risk; a flip drops the closed position. Unknown stop, risk price, mark, equity or buffer rejects entries. Tests are inline (need `pub(crate)` rule_ctx), not `tests/portfolio.rs`.
- Fix round (PR 56): G02/G04b take `risk_ref` (Buy `max(intent, live)`, Sell `min(intent, live)`), G14 keeps `ref_price = max`; the gate (slice 11) must compute both. Opposite holding leaves only when `exit_qty > 0`; G14 rejects mark <= 0 or duplicate marks; symbols compare trimmed, case-insensitive.
- Owner to confirm (CR-56-3): G02 adds the new order's charges allowance to the held risk of a same-side add.
- Symbols must reach the gate in canonical instrument-master form (RISK-56-5); gate test in slice 11.
- Deferred: slice 11/11b pass the charges buffer for the FINAL qty into G02 with a cost-grows-with-qty test (RISK-56-3, QR-56-5; below-floor is raised to the floor, `size` rejects it); slice 12 replays boundaries against shipped `config/risk.toml` (RISK-56-4); F10 validates marks at snapshot build.
