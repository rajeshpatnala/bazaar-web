---
name: git-workflow
description: Git and PR workflow for the agent team - integration and release branches, branch naming, isolated worktrees for parallel developers or in-place mode, checkpoint commits, Conventional Commits, opening PRs against the integration branch, reviews on the PR, the merge-check gate, who may merge (tech lead into the integration branch only), promotions and releases (user-approved), dependency policy. Use whenever creating branches, committing, opening, updating, reviewing or merging PRs.
---

# Git Workflow

## 1. Branch model

| Branch | Purpose | How it changes |
|---|---|---|
| **integration branch** (`just base`, default `develop`) | Where all feature work lands | Feature PRs, merged by the tech lead after `just merge-check` passes |
| **release branch** (default `main`) | What gets released | Only promotion PRs (integration -> release) and release-please PRs, via `just promote <pr>`, which Claude Code asks the user to approve every time |
| work branches | One slice each | Pushed by their author; PR into the integration branch |

Both names live in `.agents/team.md` -> Branches. Nobody pushes to the integration or release branch, and nobody commits on them; the branch guard hook blocks it for every agent.

**Policy - never wipe them:** no agent deletes, renames, force-moves (`branch -f`, `checkout -B`), force-pushes, mirror/prune-pushes or resets the integration or release branch, locally or on GitHub (`gh api` included), and nobody deletes, archives or renames the repository. Merges never delete them (`just merge` only deletes work branches). If a situation seems to require it, stop and ask the user. GitHub branch protection (`just protect-branches`) is the authoritative safeguard.

## 2. Roles

| Who | May | May not |
|---|---|---|
| Developers, cicd | create work branches, commit, push their branch, open/update PRs | merge, push to protected branches, force-push others' branches |
| Testers | push test commits to the work branch they're assigned | merge, push to protected branches |
| Reviewers (code, security) | read the PR diff, write findings files | commit, push, merge |
| Tech lead | `just ship <pr>` (= `just merge` + wait for CI + pull) into the integration branch; open promotion PRs; revert via PR | merge into the release branch, merge without a passing merge-check |
| Manager | read git/PR state, run `just merge-check` to report status | commit code, merge |
| User | approve `just promote` prompts, approve on GitHub | - |

## 3. Branches and worktrees

- Ids: features `F<n>`, bugs `B<n>` (a bug fix is `fix/B<n>-<slug>`; a fix to an unreleased feature during review stays on its `F<n>` branch).
- Names: `feat/<feature-id>-<slug>`, `fix/<feature-id>-<slug>`, `test/<feature-id>-<slug>`, `ci/<feature-id>-<slug>`, `chore/<feature-id>-<slug>`. The feature id links the PR to its review files.
- **Work mode** (`team.md` -> Parallelism, checked by `just preflight`):
  - `worktrees` (default): parallel developers each run in an **isolated git worktree** (`isolation: worktree`). Never edit files in another agent's worktree.
  - `in-place`: no worktrees; **one developer at a time** in the main checkout. `just new-branch` refuses to start on
    a dirty checkout, so the user's own uncommitted work is never carried along. Reviews use
    `just review-pack --diff <base>..<branch>`; merges `just merge --branch <branch>` (needs a recorded gate pass).
- **Checkpoint commits** (`checkpoint_commits | yes`): commit on your own feature branch each time the gate passes, so
  any step can be undone with `git revert <sha>`. Push only your own feature branch.
- Records travel with the code: a feature PR adds or updates `docs/features/F<n>-*.md`; a bug-fix PR commits `docs/bugs/B<n>-*.md` with root cause and regression test. `merge-check` checks both.
- First action: `just new-branch feat/<id>-<slug>` (fetches and branches from the latest integration branch).
- Keep branches short-lived; before updating the PR: `git fetch -q && git rebase origin/$(just base)`.

## 4. Commits

Conventional Commits, small and focused:

```
feat(api): add GET /api/v1/customers/{id}
fix(store): map unique violation to Conflict
test(ingest): property test for cleaning idempotency
refactor(domain): introduce CreditLimit type
chore(deps): bump <dependency> to <version>
```

Breaking change: `feat(api)!: ...` + `BREAKING CHANGE:` footer. Put the id in a `Refs: F12` (or `Refs: B7`) trailer; `just merge` adds it to the merge commit too, which is how `just feature <id>` finds every change. Add any attribution trailers your environment requires.

## 5. Pull requests (always against the integration branch)

```bash
just dev-gate                               # must pass first
git push -u origin HEAD
gh pr create --base "$(just base)" --title "feat(api): ..." --body-file - <<'BODY'
<PR body, see below>
BODY
```

Don't write PR bodies or handoffs as files inside your worktree (they'd get committed). PR body, short:

```markdown
## What
## Why            (feature id, plan)
## How            (layers touched, key decisions)
## Tests          (unit tests added, other levels, gate result)
## Risk/rollout   (config keys added to .env.example, migrations, flags)
```

No GitHub remote or `gh`: keep the branch local and report "PR: local branch <name>"; the tech lead uses the local merge (section 6).

## 6. Reviews and merging (tech lead)

Reviews happen **on the PR, before merging**: code reviewer, security reviewer and tester(s) read the PR diff as soon as the author hands off, and record a verdict in `.agents/reviews/<feature-id>/` (`Verdict: APPROVE` / `PASS` / `SIGN-OFF`).

`just merge-check <pr>` is the gate and the **only** source for saying a PR is green. It verifies:
- the PR targets the integration branch (or is a back-merge from the release branch), is open and not a draft
- the title is a Conventional Commit
- every GitHub CI check on the PR has passed (none running, none failed)
- code review approved and security passed with no open blocker/major/critical/high findings (unless the tech lead recorded reject/defer), and tester sign-off for feat/fix/refactor/perf/test branches

```bash
just merge-check <pr>        # status for anyone (manager reports this, never memory)
just merge-check <pr> --wait # polls CI in the script; no checks reported yet = pending
just ship <pr>               # tech lead: wait for CI, merge-check, squash-merge, delete branch, pull base
# local fallback without GitHub:
just merge --branch feat/F12-orders --title "feat(api): orders endpoint"
```

After merging, refresh the main checkout so the code map updates: `git switch "$(just base)" && git pull --ff-only`. Merge order for parallel work: smallest/most foundational first; remaining branches rebase (their developers resolve conflicts). If the integration branch breaks: revert via a PR (`git revert <sha>` on a `fix/` branch).

## 7. Promotions and releases (user-approved)

1. When the user wants a release, the tech lead opens a promotion PR: `gh pr create --base main --head "$(just base)" --title "chore(release): promote $(just base) to main"`.
2. The manager asks the user; `just promote <pr>` merges it (Claude Code shows the user a permission prompt; a merge commit keeps the feature commits for the changelog).
3. release-please opens a release PR on the release branch (version bump + `CHANGELOG.md`). Same path: user approves -> `just promote <pr>` -> tag `vX.Y.Z`, GitHub release, image.
4. Back-merge the release commit: PR from the release branch into the integration branch, `just merge <pr>` (CI only).

Never hand-edit generated changelog entries; never tag by hand while release-please is active. No GitHub: `just changelog` + annotated tag by the tech lead after the user approves. Production deploys need explicit human approval.

## 8. Dependencies

New dependency in a PR requires: why existing ones aren't enough, maintenance health, license policy, transitive count, and isolation behind our own type if it's an integration. Versions pinned in one place, per `bz-repo-structure`.
