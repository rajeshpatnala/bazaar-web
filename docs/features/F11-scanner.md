# F11: Scanner (run strategies over a fixture universe, score, rank, Why text, Recommendation)

Status: building (step 1 of 4: config and types). Not investment advice.

Summary: a fixture scan yields ranked, explained, sized, gate-summarised recommendations; no route, no UI.
Owner area: `crates/scanner`, `[scanner]` in `config/strategies.toml` (typed in `core::config::files`).
Plan: `.agents/plans/F11-scanner.md`. Spec: `docs/strategy/F11-scanner-defaults.md`.

## Decisions
- Every `[scanner]` value is an UNVERIFIED proposal (weights 0.30/0.25/0.15/0.30, floor Rs 10 crore, `max_results` 5, cap 55, `shrink_k` 30); owner confirms later.
- Loader rejects weights outside 0-1 or not summing to 1, `max_results` outside 1-20, a cap above 100, zero counts.
- No `scanner -> backtest` edge: `EvidenceSource`, `BanList`, `EventCalendar` are scanner-owned ports.
- Ban list unavailable, undated, or older than `max_ban_list_age_hours` (24) fails closed.
- `Recommendation::new` validates and caps `confidence` when `low_evidence`; no evidence is the neutral prior (PF 1.0, expectancy 0, 0 trades). Id is a `u128` from `IdGen` (no uuid dependency).
- `GateSummary::from_verdict(&verdict, evaluated_at)`: the verdict has no clock, so the caller passes the time.

## Known limits
Evidence is empty until F9 runs (all recs `low_evidence`), no options structures, events calendar is a placeholder, equity long-only.

## Deferred
- `min_trades_evidence` must not go below the gate's `min_oos_trades` (strategist): enforce when step 4 reads risk.toml.
- Rank by capped confidence, absolute log liquidity scale, golden fixtures (strategist): steps 2-3.
