# Decisions

One line per decision, newest first. Full records in `docs/adr/`.

| date | decision | by | ADR |
|---|---|---|---|
| 2026-11-18 | Kit 0.2.0 (PR #9) deferred to kit backlog, no kit feature record: SEC-kit-2 (find_review fallback accepts other feature dirs), SEC-kit-4 (dev-git-guard regex bypassable, pre-existing), SEC-kit-13 (info, open), CR-kit-4. Details in `.agents/reviews/kit/` | tech-lead | - |
| 2026-11-18 | F1: Rust 1.100.0 pinned via rustup; risk.toml hard ceilings equal owner defaults (1% / 3%); F1 task budget 3M->4M tokens and tester_review behaviour-change->large; F2 queued for calendar/charges data | owner | - |
| 2026-11-19 | F6 owner decisions: G08 pauses entries for the day after 2 consecutive losses; max open positions 3; G17 (60 min re-entry lockout), G18 (3 new entries/day), G19 (6% weekly loss, Hard) approved; G14 single-position cap stays 25%; all other F6 defaults stay owner-to-confirm/unverified, file-level verified_on not bumped. Plan `.agents/plans/F6-risk-crate.md` | owner | - |
| 2026-11-19 | F6 owner answers (relayed by manager): sizing equity capped at starting cash (`cap_equity_at_starting_cash` cannot be false); No-rows loosenable only within `[hard_limits]` ceilings; `[hard_limits]` ceilings for G14 (25), G05 (30), G04a (3) added (RISK-44-1) | owner | - |
