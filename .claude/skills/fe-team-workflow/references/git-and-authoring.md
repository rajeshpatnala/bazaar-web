# Git and authoring a PR

PR lifecycle, git config and daily flow, branch/commit rules, stacked PRs, PR size, description example and self-review.

## Contents

- PR lifecycle
- Git
- Authoring a PR

## PR lifecycle

*Diagram: PR lifecycle · branch to production with time targets*

The slowest step is almost always waiting for a human. That's why the rules focus on small PRs, a one-business-day review limit, prompt re-reviews, and automating everything after approval.

## Git

Short-lived branches off one **integration branch** (`develop` by default; your team may name another): every branch lives one or two days, every PR targets the integration branch, and its history is one squashed commit per PR. The **release branch** (`main`) only moves through a promotion PR from the integration branch (and release PRs), each approved by a human. Both branches are protected and are never deleted, renamed, force-pushed or reset. The settings below make the common operations (rebasing, resolving repeated conflicts, stacked branches) painless.

**Recommended git config** — every engineer runs these once:

```bash
git config --global pull.rebase true           # pull rebases your commits instead of creating merge commits
git config --global rebase.autoStash true      # uncommitted work is stashed and restored around a rebase
git config --global rebase.updateRefs true     # rebasing the bottom of a stack moves every branch above it
git config --global rerere.enabled true        # git remembers how you resolved a conflict and reapplies it
git config --global push.autoSetupRemote true  # first push of a new branch just works
git config --global fetch.prune true           # deleted remote branches disappear locally
```

**Daily flow**

```bash
git switch develop && git pull
git switch -c feat/WEB-142-apple-pay        # type/TICKET-short-description
# …work, commit in small logical steps…
git fetch && git rebase origin/develop       # stay current; resolve conflicts while they're small
git push --force-with-lease                  # safe force-push: refuses if someone else pushed
gh pr create --draft --fill --base develop   # draft until CI is green and you've self-reviewed
```

**Branch and commit conventions**

| Item | Convention | Example |
| --- | --- | --- |
| Branch | `type/TICKET-short-description` | `fix/WEB-201-cart-total-rounding` |
| Commit / PR title | Conventional Commits: `type(scope): imperative summary` | `feat(checkout): add Apple Pay as a payment method` |
| Types | `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `ci`, `build` | — |
| Breaking change | `!` after the type, plus a `BREAKING CHANGE:` footer | `feat(api-client)!: drop v1 endpoints` |
| Body | Why the change was needed, not what the diff shows | "Totals were rounded per line, causing 1-cent mismatches with invoices." |

**Rules**

- **Rebase your own branch; never rewrite shared history.** `develop`, `main` and anyone else's branch are append-only.
- **Never delete, rename, force-move, force-push or reset `develop` or `main`**, locally or on GitHub. If it ever seems necessary, stop and ask.
- **Nothing merges before review.** A PR merges only when its reviews are approved and every CI check is green, checked on the PR itself (`gh pr checks <n>`), never from memory.
- **`main` moves only by promotion**: a PR from `develop` to `main`, approved by a human, then release automation tags it.
- **Use `--force-with-lease`, never `--force`**, so you can't overwrite a teammate's pushed commits.
- **Commits inside a PR may be messy** ("wip", "address review"); squash-merge turns the PR title and description into the one commit that matters.
- **Delete work branches after merge** (enable "Automatically delete head branches" in repository settings). Never `develop` or `main`: a promotion PR's head is `develop`, so merge those without deleting the branch.

**Stacked PRs** — when a feature is too big for one PR, build it as a chain of small dependent PRs instead of one giant one:

```text
develop ← feat/orders-1-model ← feat/orders-2-api ← feat/orders-3-ui (behind flag)
```

Each PR targets the branch below it and is reviewed independently. With `rebase.updateRefs`, rebasing the top branch onto `develop` updates the whole chain. Merge bottom-up; after each merge, retarget the next PR to `develop`. Tools such as Graphite or `gh-stack` automate this, but plain git works.

## Authoring a PR

The author's job is to make the reviewer's job easy. A small PR with a clear description, a preview link and a self-reviewed diff gets a careful review in minutes. A 2,000-line PR gets a skim and an approval nobody believes in.

**Size guide** (changed lines excluding generated files, lockfiles and snapshots)

| Size | Typical review | Guidance |
| --- | --- | --- |
| < 100 | Minutes, thorough | Ideal |
| 100–400 | Under an hour, thorough | Normal |
| 400–1,000 | Reviewers skim; bugs slip through | Split unless it's mechanical (rename, codemod) |
| > 1,000 | Rarely reviewed properly | Split; if truly impossible, walk the reviewer through it live |

Add a CI label or bot comment that flags PRs over 400 lines, so splitting becomes a habit rather than a debate.

**How to split a large change**

| Strategy | Example |
| --- | --- |
| Preparatory refactor first, then the feature | PR 1: extract `PriceSummary` from `CheckoutForm` (no behaviour change). PR 2: add discount codes to it |
| Layer by layer (stacked) | PR 1: schemas and model rules with unit tests. PR 2: API and query options. PR 3: UI behind a flag |
| Vertical slices behind a flag | PR 1: bulk-select orders. PR 2: bulk cancel. PR 3: bulk export — each usable on its own, flag off |
| Mechanical changes alone | One PR only renames `customer` to `account` across 80 files; reviewers verify it's purely mechanical |

**Description** — the PR template in `.github/pull_request_template.md`, filled in like this:

```markdown
## What and why
Adds bulk cancel on the Orders page. Support agents cancel ~40 orders a day one at a time (WEB-311).
Behind `orders.bulk-cancel` (off).

## How to test
Preview: https://pr-482.preview.acme.dev/orders — select 3 pending orders → Cancel selected.
Shipped orders are skipped and listed in the result toast.

## Screenshots
Before / after, light and dark. 20-second screen recording of the keyboard flow.

## Notes for reviewers
Start with `features/orders/api/orders-api.ts` (the new mutation); the UI is mostly layout.
I chose one batch request over N requests: see the trade-off in the code comment.
```

**Self-review before requesting review** — open your own diff in the PR view and check:

- [ ] Every changed line is intentional; no debug code, stray `console.log`, commented-out code or unrelated reformatting.
- [ ] The PR does one thing, and its title would make a good line in the changelog.
- [ ] Tests cover the new behaviour and the failure paths; CI is green.
- [ ] UI changes: tested with keyboard only, at 200% zoom, and in dark mode; screenshots attached.
- [ ] Anything surprising has a code comment explaining why, or a note for reviewers.

**Responding to review**

- Reply to every comment: a commit link ("Fixed in a1b2c3"), a reason ("Kept it because…"), or a question.
- Push fixes as new commits during review, so reviewers see what changed since their last pass. Squash-merge cleans it up.
- Resolve a thread yourself only for `nit:` and `suggestion:` comments you applied; let the reviewer resolve `issue:` threads.
- Re-request review when you're done, so the reviewer knows it's their turn.
- Disagree openly and kindly. The author owns the code, but both people are aiming for the healthiest codebase.
