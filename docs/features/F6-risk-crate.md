# F6: Risk crate

Status: building (slices 0 and 1 done: config). Plan: `.agents/plans/F6-risk-crate.md`. Owner area: `crates/risk`, `config/risk.toml`.
Summary: deterministic DecisionGate, sizing, portfolio and behaviour rules, override engine in `crates/risk`; keys in `risk.toml`.
Decisions: owner 2026-11-19 (G08 = 2 losses, until end of session; max 3 positions; G17/G18/G19 approved; G14 cap 25%; No-rows loosenable only within `[hard_limits]` ceilings; sizing equity capped at starting cash); fail closed by type; gate classifies Entry/Exit/StopMove; one override-level table; `GatedIntent` mintable only in `gate.rs`; config hash = sha256 of file text. Skill G-table updated first (own commit) so reviewers check against it.
Known limits: G12, readiness bar, real audit/baseline/journal deferred; in-memory baseline loosens on restart until F10; inputs for G01/G06/G07/G15/X01/X03 reject until F3/F9/F10; every key not owner-decided is `owner-to-confirm, unverified`; file-level `verified_on` (2026-11-18) not bumped.
F10 handoff: snapshot built only from the journal (journal read failure = no snapshot = no order); G10 entry zone from the stored recommendation, not the request; gate re-run and expiry at approval via `evaluated_at` and config hash.
Deferred: RISK-50-1 gate passes one worst-case reference price to size, G02, G10, G11 (slices 5, 11); RISK-50-2 gate derives `overridden` from an accepted override in the same verdict, calls `size` itself, and F9 computes cost for the final qty (slices 10, 11); RISK-50-3 slices 8/9 reject above cash/margin and G16 notional and test a sized order passes, G14 counts a same-symbol position (slices 5, 8, 9); first-15-minute rule; broker order-rate figure and the new hard ceilings (G04b 3, override caps, orders/min) stay Owner actions until confirmed; slice 1 values (`[hard_limits]` G05/G04a/override/orders-per-minute ceilings, `max_orders_per_minute` 10, charges floor 20) are placeholders, unverified, owner to confirm.

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
| `exposure.correlation_window_days` | 60 | 20..=250 | unverified |
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

Slice 1b `[hard_limits]` (unverified, owner to confirm): `max_ban_list_age_hours` 24, `max_quote_age_secs` 5, `max_snapshot_age_secs` 10, `latest_expiry_day_no_new_after_ist` 13:30, `min_confirm_delay_secs` 10, `max_size_factor` 0.5, `min_oos_trades` floor 30 (equals the G01 default). Deferred: symlink and canonical-path check for `events_file` belongs to the events loader (F3/F10).

Ceilings for G05 and G04a sit at their defaults; changing any `[hard_limits]` value needs an ADR and the owner.
Test fixture `fixtures/config/risk.toml` gained the same sections with synthetic values.

## Slice 3 decisions
- X01 (market open), X02 (lot and tick) and X03 (margin) are not in the skill G-table. Their override level is **No** in `RuleId::override_level()`. Owner to confirm.
- G12 (liquidity) has no `RuleId` (outside F6).
- Keys without a baseline entry (quote and snapshot age, size factor, confirm delay, correlation, evidence, event flags) can be loosened within their ceilings or bounds mid-session until F10 persists a baseline for them.
