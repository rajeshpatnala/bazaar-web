# F11: Scanner (run strategies over a fixture universe, score, rank, Why text, Recommendation)

Status: building (step 1 of 4: config and types). Not investment advice.

Summary: a fixture scan yields ranked, explained, sized, gate-summarised recommendations; no route, no UI.
Owner area: `crates/scanner`, `[scanner]` in `config/strategies.toml` (typed in `core::config::files`).
Plan: `.agents/plans/F11-scanner.md`. Spec: `docs/strategy/F11-scanner-defaults.md`.

## Decisions
- Every `[scanner]` value is an UNVERIFIED proposal (weights 0.30/0.25/0.15/0.30, floor Rs 10 crore, `max_results` 5, cap 55, `shrink_k` 30); owner confirms later.
- Loader rejects weights outside 0-1 or not summing to 1, `max_results` outside 1-20, a cap of 100 or above, zero counts.
- No `scanner -> backtest` edge: `EvidenceSource`, `BanList`, `EventCalendar` are scanner-owned ports.
- `scanner.max_ban_list_age_hours` may not exceed `risk.toml` `hard_limits.max_ban_list_age_hours` (checked in `Config::load`).
- `low_evidence` = no evidence, too few trades, costs not applied, or not out-of-sample (unknown is false); shown in the Why.
- `Recommendation::new` rejects risk below qty x worst-fill stop distance and non-monotonic targets.
- Ban list unavailable, undated, or older than `max_ban_list_age_hours` (24) fails closed.
- `Recommendation::new` validates and caps `confidence` when `low_evidence`; no evidence is `backtest() == None` (never placeholder numbers). Id is a `u128` from `IdGen` (no uuid dependency).
- `GateSummary::from_verdict(&verdict, evaluated_at)`: the verdict has no clock, so the caller passes the time.

## Known limits
Evidence is empty until F9 runs (all recs `low_evidence`), no options structures, events calendar is a placeholder, equity long-only.

## Deferred
- Step 2 review (PR 114): ~~CR-114-3~~ closed in slice 4 (D8): `Signal` is long-only by type, so no Buy/Sell conflict can arise (test `a_short_shaped_signal_is_rejected_by_the_signal_contract`); re-deferred: the first slice that adds a sell side must add `Warning::ConflictingSignals` reporting in the runner (named in that slice's plan). RISK-114-7 (D9): the risk crate has no G12 rule and no `min_adv_inr`. Owner approved on 2026-11-24 ("I approve #1"): the scanner's Rs 10 crore median-traded-value floor is the only liquidity check until a G12 plan exists; no G12 check exists today (target: a G12 slice or plan item before `semi_auto` is reachable, risk-guardian plan after F11).
- ~~RISK-114-4 `StaleDecisionDate`~~ done in slice 4 (`run` errors when `decision_date` is older than the latest completed equity session; a signal past `valid_until` is dropped as `SignalExpired`).
- ~~QUAN-118-4 / RISK-118-3 `families_total`~~ done in slice 4 (`ScanOutput.families_total`, distinct families of the enabled strategies, >= 1 else `NoStrategiesEnabled`).
- ~~RISK-102-5 remainder~~ type done in slice 4 (`GateSummary.paper_only`, subset-checked in `Recommendation::new`); filled from the real verdict in slice 6 (already by `from_verdict`). ~~QUAN-102-6 remainder, reward_risk part~~ `Recommendation::new` re-derives `reward_risk` from the levels (`types::reward_risk`); the charges-inclusive `risk_rupees` formula stays with slice 5.
- Rank by capped confidence, absolute log liquidity scale, golden fixtures (strategist): steps 2-3.
- QUAN-102-7: round or reject entry, stop and targets to the instrument-master tick (test the stop 96.0333 case): step 4 (the tick comes from the instrument master the sizing step reads).
- QUAN-102-6 remainder: charges-inclusive `risk_rupees` formula and the `reward_risk`-to-first-target check: step 4 (`risk::size` caller, tested there).
- RISK-102-10: all `[scanner]` values stay UNVERIFIED, owner to confirm (with `docs/strategy/F11-scanner-defaults.md`) and to confirm G12 `min_adv_inr` is not looser than `min_median_traded_value` before step 4.
- ~~RISK-102-8 tests~~ done in step 2 (`universe::tests`: unavailable, undated, 25 h stale, 24 h fresh, case-normalised, future-dated).
- `family_of` (scanner runner) is a hardcoded strategy-label map (F8 table); move it to config or `StrategyMeta` later. `circuit_locked` is a caller-supplied flag until a data source exists.
- SEC-102-3: size caps on ban-list symbols and event lists belong to the F13 adapters (documented on the ports).
- RISK-102-5 remainder: `GateSummary` fields stay public (checked for consistency in `new`); a per-failure rule level so the UI does not offer an override the mode forbids (G01 is Paper level) comes with the UI/step 4.
- Steps 2-4 will be re-cut so each fits `max_slice_changed_lines` (step 1 carried the shared types).

## Slice 4 note (guards + shared contracts)
`result.rs`: `ScanResult` (private fields, verdict derived in `new`, no `Default`), `BlockedIdea`, `BlockReason`; runner guards above; calendar.toml covers 2026 only, so runner tests use dates in Nov 2026.

## Slice 2 note
Universe filters (ban list fail closed, liquidity, circuit lock, history) and the runner (regimes to the closed bar, `DataView::ending_at`, per-family merge, `lookahead_` test) landed in `crates/scanner/src/{universe,runner}.rs`; accepted as one PR over the line budget.
Fix round (PR 114): the runner takes one effective ban-list age (`min` of the scanner key, risk.toml events and hard_limits) and a tri-state `CircuitState` (the caller passes `Unknown` until a data source exists, which drops the name). The market regime uses no India VIX or breadth yet (`Extras::default()`); the output does not flag it. Deferred to steps 3-4: a `StaleDecisionDate` check of `decision_date` against the calendar (RISK-114-4), Sell/conflicting-signal handling (CR-114-3; only Buy is accepted today), the `family_of` move to config or `StrategyMeta`, and RISK-114-7 (owner item).

## Slice 3 note (BZ-302)
`crates/scanner/src/score.rs`: `score`, `rank`, `score_and_rank` over `Candidate`s. Score = 100 x weighted components (confluence = agreeing families / enabled families, regime fit = mean of 1 or `penalty_weight`, liquidity on the absolute log scale, shrunk evidence) minus a proximity-scaled event penalty, one f64 to `u8` boundary. `low_evidence` (primary hit: allowed fit first, then reward:risk) caps the displayed score. Rank = proven above low-evidence (rule R1), then capped score desc, tradingsymbol asc, instrument token asc. Evidence prior is the strategy-wide profit factor only with >= `min_trades_evidence` trades, else neutral 1.0; unusable evidence (in-sample, gross, non-finite, win rate outside 0..=1) is the floor, and avg_r <= 0 gives no evidence credit. Empty calendar: no penalty, `EventsNotChecked`.
Decisions / deferred: `EVENT_WINDOW_DAYS` (5) and the profit-factor-2.0 full-score point are code constants, UNVERIFIED, move to `[scanner]` with the owner confirmation; `families_total` is passed by the caller (step 4); `max_results` truncation happens after the gate in step 4; the "scores within 3 points are similar" display is a UI concern. Golden fixture pins 80/59/33 and the two strategist invariants (low evidence below proven; up:high below up:normal).

Fix round (PR 118): ranking is structural (proven above low-evidence, then capped score desc, symbol, token); "proven" needs >= `min_trades_evidence` trades, out of sample, net of costs, PF > 1 and avg_r > 0; non-finite evidence is unusable and not shown in `Scored.evidence`; confluence is 0 with fewer than 2 enabled families. Deferred (QUAN-118-3, RISK-118-2): `EVENT_WINDOW_DAYS` and `PF_FULL_SCORE` stay code constants; move to `[scanner]` with loader validation (window 1..=30, full-score PF > 1) and owner confirmation. Step 4 acceptance (QUAN-118-4, RISK-118-3): derive `families_total` from enabled strategies' families in config, validate it, never from hits. Also deferred: a contract test for the real `EvidenceSource` (no trades closed after `as_of`) when it lands.
Step 4 acceptance (SEC-118-1): event `note` text is config-sourced and goes into `Warning::EventNearby`; escape it when rendered (UI/Why text). Event kind is shown as a plain label, not the Debug repr.
