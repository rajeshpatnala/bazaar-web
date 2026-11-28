# Manager recommendations, 2026-11-21

Written for the owner and the next manager session. Each item says what to do and why.

## Do first (next session)
1. `just ship 104`. Both reviewers approved commit 03cffeb. Re-run `just merge-check 104` first.
2. Review PR 105 (F10 slice 2: rate limiter, daily-loss breaker, guarded executor). It touches the safety path, so it needs risk-guardian, code, security and tests reviews. Launch them in one message.
3. Review PR 102 (F11 step 1). Needs code, security, risk-guardian and quant.
4. Write F9 slice 7b3c (golden tests AC 1, 6, 7 plus the final F9 record), then T2 for F9 and F10.

## Process
- **Cut slices smaller.** PRs 99, 100, 102 and 105 all went over the 400-line cap, and over-cap PRs cost extra review rounds. Ask the tech lead to plan slices of about 300 lines. PR 105 can split at `guarded.rs` (about 235 lines) if you want it smaller.
- **Close findings before pushing hand-backs.** Several rounds were spent only on reviewers updating a Commit line or marking an info finding closed. Tell each reviewer in the first prompt: "after a comments-only fix push, update the Commit line and close findings in the same pass."
- **Use `git merge origin/develop` instead of a rebase** when a PR conflicts. A force-push instruction was blocked by auto mode.
- **Give developers the findings path.** Dev-data and dev-core could not see the review files from their worktrees and worked from my message. Put the absolute path to the findings file in every fix-round prompt.
- **Reviewers see `develop`, not the branch.** They can report stale line numbers. Ask them to read with `git show <branch>:<file>`.

## Product and owner decisions
- **F11 defaults** (weights 0.30/0.25/0.15/0.30, Rs 10 cr liquidity floor, max_results 5, whether an empty events calendar blocks entirely). Confirm before F11 step 2.
- **Approve `duckdb`** (bundled + parquet) so F12 slices 2-3 can start. Slices 0-1 need no new dependency.
- **Review `risk.toml`** and the F10 defaults before slice 3 onward.
- **Add NSE 2027 holidays** to `config/calendar.toml` before year end.
- **Run `just ui-dev`** and sign off the F13 look (light premium preview B), plus the idea copy.
- **Disclaimer.** The new wording is live. The owner may want a legal read, since it now says "not SEBI registered".

## Technical follow-ups already recorded as deferred
- F10: cap `actor`/`reason` at 500 chars (SEC-100-4) and run the gate via `spawn_blocking` (SEC-100-5, CR-100-2), both in slice 3. The breaker latch is in-process and is lost on restart; decide whether to persist it.
- F11 step 4: enforce `min_trades_evidence` at or above the gate's `min_oos_trades`.
- F12 slices 1-2: bind the symbol as a SQL parameter and allowlist it before it goes into a Parquet path (SEC-104-1).
- F13: one slice must add an error boundary and the route-level render-error test that CR-101-7/8 deferred.
- F9: `walk_forward_gated` does not recheck the depository charge, because it is stateful; this is documented, not fixed (QUAN-99-7).

## Budget
- Several fix rounds cost more than the work they fixed. Next session, prefer the tool-free `just envelope` review for small PRs and resume warm reviewers instead of starting new ones.
- Run `just budget-check` before each batch.
