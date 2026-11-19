# F6: Risk crate

Status: building (slice 0 done: config). Plan: `.agents/plans/F6-risk-crate.md`. Owner area: `crates/risk`, `config/risk.toml`.
Summary: deterministic DecisionGate, sizing, portfolio and behaviour rules, override engine in `crates/risk`; keys in `risk.toml`.
Decisions: owner 2026-11-19 (G08 = 2 losses, until end of session; max 3 positions; G17/G18/G19 approved; G14 cap 25%; No-rows loosenable only within `[hard_limits]` ceilings; sizing equity capped at starting cash); fail closed by type; gate classifies Entry/Exit/StopMove; one override-level table; `GatedIntent` mintable only in `gate.rs`; config hash = sha256 of file text. Skill G-table updated first (own commit) so reviewers check against it.
Known limits: G12, readiness bar, real audit/baseline/journal deferred; in-memory baseline loosens on restart until F10; inputs for G01/G06/G07/G15/X01/X03 reject until F3/F9/F10; every key not owner-decided is `owner-to-confirm, unverified`; file-level `verified_on` (2026-11-18) not bumped.
F10 handoff: snapshot built only from the journal (journal read failure = no snapshot = no order); G10 entry zone from the stored recommendation, not the request; gate re-run and expiry at approval via `evaluated_at` and config hash.
Deferred: first-15-minute rule; broker order-rate figure and the new hard ceilings (G04b 3, override caps, orders/min) stay Owner actions until confirmed; keys for slice 1 (`[options]`, `[override]`, `gate.evidence`, `events`, `data`, G16, charges floor).

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

Not in slice 0 (slice 1): `block_ban_list`, `block_results_days`, `block_policy_days`, `max_quote_age_secs`, `min_oos_trades`, `confirm_delay_secs`, `size_factor`, override caps and `max_orders_per_minute` ranges and ceilings.
Test fixture `fixtures/config/risk.toml` gained the same sections with synthetic values.
