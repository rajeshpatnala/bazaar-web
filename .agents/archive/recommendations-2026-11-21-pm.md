# Manager recommendations, 2026-11-21 (evening)

Written for the owner and the next manager session. Supersedes the open items in `recommendations-2026-11-21.md` (the morning file). Each item says what to do and why.

## State now
- Merged this session: BZ-403, 502, 202 (PR 119), 302 (PR 118), 102 (PR 120). Roadmap rows are marked DONE.
- In flight: PR 121 (BZ-402) in a fix round (4 findings, then re-review and ship); BZ-106 (system DuckDB) in development.
- Parked, not started: BZ-203 (store order repos, worktree `agent-a85ea92f451974c4d`) and BZ-404 (F3 contracts, worktree `agent-abd11fa51b60cb174`). Both sit on the old base 4a05b20 and need a rebase onto `develop`.
- Limit: at most 2 builders at once (Mac RAM). Do not create new agents when the owner says wait; resume existing ones only.

## Do first (next session)
1. `just status`, then finish PR 121 (fix round -> re-review by the same two reviewers -> tech lead `verify-fixes` + `approve` -> `just ship 121`).
2. Review and ship BZ-106 when it lands. Owner must have `brew install duckdb` done (it is, on this Mac). Check CI passes with the new DuckDB install step; the developer may not be able to verify the Linux CI step locally.
3. Then BZ-203 and BZ-404 (rebase first, low-RAM rules).
4. After BZ-106 merges, delete stale worktrees with `just worktree-clean` (30+ old ones, many hold their own DuckDB build; large disk use).

## Owner decisions still open
- **BZ-290:** daily-loss breaker always engages the kill switch. Recommend: yes.
- **BZ-390:** scanner defaults and G12 `min_adv_inr`. Recommend: accept for now.
- **BZ-491:** Nifty 50 + Bank Nifty list into `config/watchlists.toml`.
- **BZ-490:** Kite keys via `just set-secret kite_api_key` / `kite_api_secret` (Keychain).
- **BZ-990:** NSE 2027 holidays; **BZ-590/591** as listed in the roadmap.
- **UI:** move BZ-503 (Ideas screen) up so there is a real screen sooner? Owner has only seen the page shell. Recommend: yes.
- **BZ-104/105:** does backtest evidence belong in M1? Open.

## Build speed and RAM
- Decision (owner, 2026-11-21): keep DuckDB, link the system library instead of compiling it (BZ-106). Dockerfile install step later. Crate `1.20001.0` matches DuckDB 2.0.1 (Homebrew); a version drift can break the build, so document the version.
- Shared build cache: recommend one shared `CARGO_TARGET_DIR` (e.g. `~/.cache/bazaar-target`) in developer prompts, or sccache (installed, not enabled; the owner must edit `~/.cargo/config.toml` and `~/.zshrc`). Never `cargo clean` in a worktree.
- A different machine, or Windows, pays one slow first build. Windows is not scoped: secrets use the macOS Keychain, scripts assume Unix. Use WSL2 if it ever comes up and ask the tech lead to scope it first.
- CI is slow when the cache is cold. Measure per-job timings with `gh` before changing the workflow; BZ-106's prebuilt library should cut the cold-run cost.

## Process
- **Cut slices smaller.** PR 121 was 471 lines (cap 400); over-cap PRs cost extra review rounds. Ask the tech lead to plan slices under the cap.
- Developers must run the full `just dev-gate` once before handing back. BZ-402 skipped it to save RAM; CI caught nothing yet, but it is the rule.
- Reviewers for small PRs should be told not to run cargo (done this session; worked, no RAM spike).
- The kit has no "triage decide" command; the tech lead edits the decision cells directly. Worth a kit note.

## Housekeeping
- Add the SEC-118-1 deferral (to BZ-519) to the F11 record's Deferred list before `just promote`.
- Walk `just deferred` at BZ-205, 511, 519.
- F15 board row is over 200 characters.
- T2 tester passes still pending for F9, F10, F11, F13.
