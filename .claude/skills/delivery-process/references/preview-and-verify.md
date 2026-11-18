# Previews and verification tiers

## Preview-first lane

Taste can't be settled with text questions, and a wrong guess in the real code costs ~5-10x a preview.
1. Ask at most one question that can be answered by pointing ("any product or site you like the feel of?").
   Never ask the owner to choose between style adjectives.
2. Launch `prototyper` to make <= 3 cheap previews of clearly different directions in `previews/<id>/` (cap 150k tokens). What a preview is here:
   - a screen as a clickable page in `previews/<task-id>/` (static HTML or `just app-dev` with the MSW mock API and fake
     candles/recommendations), or a sample recommendation card + Why panel rendered on fake data.
   - for strategy ideas: a one-page backtest report on fixture data (equity curve, drawdown, trade list, stats) so the
     owner can judge by looking; made outside the product, no new dependencies, never real account data.
3. Show them and ask: "which do you keep, which do you drop, more or less of what?"
4. Turn the pick into owner rules and acceptance criteria, then size and build it normally.
Skip it only when the owner says "just build it" or the look is already fixed by a design they gave you.

## Verification tiers

| Tier | When | Who | What |
|---|---|---|---|
| **T0** | every step/slice | the developer (gate hook enforces it) | `just dev-gate`: format, lint, types, unit tests, build |
| **T1** | every step that changes what a user or caller sees | the developer | one quick look at the changed thing only, stated in the handoff |
| **T2** | **once per feature**, after the last slice | one tester | the full matrix for the feature |

- **T1 (slice):** one quick look at what changed: quant/backtest/data/execution -> run the changed unit once on a
  fixture (one targeted test or one `just cli-fixture` command) and compare to the reference or expected value; API ->
  one in-process router test (`oneshot`) for the changed route; UI -> one screenshot of the changed screen in
  `just app-dev` with the MSW mock API. No suites, no matrices.
- **T2 (feature):** acceptance criteria end to end on fixtures; full gate + `just check`; `just lookahead` and
  property suites; `just golden` (backtest stats diff vs committed baseline, explained if changed); API auth, CSRF and
  session negative cases on changed routes; UI Vitest on the MSW mock API (Playwright runs in CI; read the PR checks); paper-trading flow from
  recommendation to approval to fill with charges debited; quant-reviewer / risk-guardian findings closed; the full
  "not verified" list.

Slices never run T2. A slice handoff says "T0+T1 done; T2 pending". The T2 report is the only place for the long
"not verified" list. Don't re-run checks a previous tier already passed on unchanged code.
