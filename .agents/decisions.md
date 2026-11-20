# Decisions

One line per decision, newest first. Full records in `docs/adr/`.

| date | decision | by | ADR |
|---|---|---|---|
| 2026-11-18 | Kit 0.2.0 (PR #9) deferred to kit backlog, no kit feature record: SEC-kit-2 (find_review fallback accepts other feature dirs), SEC-kit-4 (dev-git-guard regex bypassable, pre-existing), SEC-kit-13 (info, open), CR-kit-4. Details in `.agents/reviews/kit/` | tech-lead | - |
| 2026-11-18 | F1: Rust 1.100.0 pinned via rustup; risk.toml hard ceilings equal owner defaults (1% / 3%); F1 task budget 3M->4M tokens and tester_review behaviour-change->large; F2 queued for calendar/charges data | owner | - |
| 2026-11-19 | F6 owner decisions: G08 pauses entries for the day after 2 consecutive losses; max open positions 3; G17 (60 min re-entry lockout), G18 (3 new entries/day), G19 (6% weekly loss, Hard) approved; G14 single-position cap stays 25%; all other F6 defaults stay owner-to-confirm/unverified, file-level verified_on not bumped. Plan `.agents/plans/F6-risk-crate.md` | owner | - |
| 2026-11-19 | F6 owner answers (relayed by manager): sizing equity capped at starting cash (`cap_equity_at_starting_cash` cannot be false); No-rows loosenable only within `[hard_limits]` ceilings; `[hard_limits]` ceilings for G14 (25), G05 (30), G04a (3) added (RISK-44-1) | owner | - |
| 2026-11-20 | F7/F8: regime params live in `config/strategies.toml` `[regime]` beside `[strategy.<id>]` (one file, one config hash); `StrategyContext` carries market + stock regime and a `strategies::PortfolioView` (strategies may not depend on `risk`); `DataView` defined in `core` by F8 slice 0; long-only daily swing cash equity first, UNVERIFIED regime defaults accepted, revisit after F9 state frequencies, never tune on P&L; strategies ship `enabled = false`; `config/risk.toml` untouched by F7/F8 | tech-lead (owner decisions relayed 2026-11-20) | - |
- 2026-11-20 owner: G16 max_orders_per_minute = 4 (was 10), max_notional_per_order_inr stays 25000 (paper only); still unverified, no broker/SEBI source (PR 69).

- 2026-11-20 (manager, owner offline): F8 entry_zone_atr_frac = 0.25 accepted as UNVERIFIED default (spec gave no value); strategies ship enabled=false; owner to confirm before enabling. QUAN-74-10 (OpenPosition fields) deferred to F9 slice 0.
- 2026-11-20 F9: `bazaar-backtest` -> `bazaar-regime` edge allowed (engine drives the real Strategy, whose StrategyContext carries RegimeState); same edge scanner already has; no cycle (regime depends on core, indicators). Handbook crate-map row for backtest to add `regime` (tech lead).
- 2026-11-20 F9 s6 (tech lead, owner offline): walk-forward OOS runs use the real gate with G01 evidence from the chosen candidate IS window and walk_forward_pass=Known(true) set by one named constructor (measurement context; verdict computed ex post from pooled OOS). Owner to confirm; see plan F9 "Slice 6 G01 design".
