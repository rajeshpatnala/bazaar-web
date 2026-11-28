# Handoff: F10 after slice 9c (2026-11-25)

Start the new session with `just status`, then `just next`.

## Where things stand
- develop 408ceb1: F10 slices 0-8, 9a (#174), 9b (#175) merged.
- **PR 176 (slice 9c, audited reset)** is open, branch `feat/bz-f10-s9c-audited-reset`. The developer reports T0 PASS, about 385 added / 69 removed lines (flag for next plan), no questions. **No reviews have run yet.**
- Board F10 row says "next 9c then 9d"; update it to "9c PR 176 in review".

## Next steps (in order)
1. PR 176 reviews: `just envelope 176` (exit 3 expected: money path), then `just review-plan 176` and launch exactly the reviewers named. Risk-guardian, code-reviewer and security-reviewer are required (9c row); each is fresh for this PR.
2. Scoped tester sign-off is likely needed (touches `crates/execution/**` or paper, 20+ production lines). Check `just merge-check 176`.
3. Triage with `just triage <id> --ref pr-176`; fix rounds go to the 9c developer (agent a2b8d0a2393c9e909, check `just warm`) via `just fix-list 176`; max 3 rounds.
4. `just approve 176` is run by the tech lead (agent acf6575799a4a109f, context 112k: check `just warm`). Then `just ship 176` yourself in the background.
5. **Slice 9d** (plan section in `.agents/plans/F10-slice9.md`, which is UNCOMMITTED in the main checkout, so give the developer its absolute path): server wiring, first-open reset with `Initial`, a real ResetGuard (refuses while the breaker is latched or the current IST ISO week nets a loss across all epochs), `paper-reset` command, `lines_since(day)` (deferred from 9c), feature-record closures. About 300 lines. Use a developer fresh or warm per `just warm`.
6. Then a fresh F10 T2 (only when 9d is merged). Include the deferred items:
   - GET /killswitch latch display
   - HTTP latch-refusal test
   - approve with a good quote places exactly once
   - quote drift gives `quote_drifted` with `new_risk`
   - an engaged kill switch gives `guard_blocked`
   - refusal tests assert which rule refused
   - money tests check all fields that move
7. F10 sign-off, close the feature record, `just efficiency F10`, `just milestone`.

## 9c facts reviewers should check
- `new_epoch(opening, audit, chain)` is on the core port. The epoch must be max+1 or it returns `Conflict`; the epoch line, the `paper_reset` audit row and the anchor hook run in one transaction.
- `PaperAccount::reset(&ResetRequest{reason, actor}, &guard)`; `ResetReason` is Initial or OwnerRequested. The fail-closed guard is `RefuseAll`.
- The runtime first-open `Initial` reset does not consult the guard and is allowed only on an empty ledger. Reviewers should confirm that.
- The opening amount comes from config only, and values <= 0 are refused.

## Deferred and owner items
- Deferred nits: CR-174-3, CR-174-4, RG-174-8, CR-175-4, CR-175-5. SEC-175-1 and RISK-175-5 are deferred by name as an owner action.
- Owner actions open:
  - Add `audit-anchor adopt` and `paper-reset` to the HUMAN-ONLY recipe lists (CLAUDE.md hard rules, agent rules, team handbook) and the deny rules. No paper day counts until this is done. Rule-file edits need the owner's approval.
  - NSE 2027 holiday list before 2027-01-01.
  - F13 UI visual sign-off via `just app-dev`.
  - Walk through `config/risk.toml`.
- Housekeeping: run `just worktree-clean` (17+ agent worktrees). Slices 9a and 9b were over the 400-line cap, so plan smaller.
- Standing owner preference: the owner is often offline. Take the recommended option and keep going, and queue owner-only items.
