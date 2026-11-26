# Kit efficiency review - updated 2026-11-19 13:29 IST (F1 s9, F2 s0-s4, F4, F6 s0-s9 partial: ~26 PRs)

Written for the kit maintainers: stack-neutral. Repo details appear only as one-line anonymized examples.
Sources: `just budget-report`, `just budget-starts`, `just start-report`, `just efficiency`, board, review and triage files, what the manager saw.
Not changed: owner rules, merge checks, branch protection, secrets handling. Launched no agents for this review.
F6 numbers are for a feature still in flight (9 PRs, 4 unmerged); counts of manager wake-ups are estimates.

## 1. Evidence

| # | number | reading | confidence |
|---|---|---|---|
| 1 | Manager: 233k context, 54.6M processed, cost-eq 7.2M today; F6 agents: 12 runs, 1.23M tokens, cost-eq 9.0M | manager is ~44% of spend; context sits at the ceiling | high |
| 2 | Cold starts 136k of 1.23M (11%; target < 25%) | starts are not the waste | high |
| 3 | The two most-reused agents cost the most: risk reviewer 1 run, 214k tokens, 7.35M processed, cost-eq 1.70M (~11 reviews, ~150k each); dev-core 2 runs, cost-eq 2.05M. A fresh single review: 34k tokens, cost-eq 79k | warm reuse trades start cost for context growth; past ~5 turns it loses | high |
| 4 | Envelope used 0 times in 29 agent reviews (F6 PRs 161 to 814 counted lines vs a 150 cap; 7 of 9 PRs over the 400-line slice cap) | built feature unused; both caps are ignored | high |
| 5 | Triage script decided 0 of the review findings by default on 9 PRs; every PR went to the tech lead (3.4M processed, 113k tokens) | the default rules do not cover domain-reviewer findings (severity "unknown") | high |
| 6 | Review yield: of ~36 reviews on 9 PRs the capital-protection reviewer found all 3 blockers, the code reviewer 1; security reviewer 0 above "low" on 5 PRs of pure config/types; tester 0 blockers | uneven value per reviewer and PR kind | medium (small sample) |
| 7 | ~80 manager wake-ups today were reviewer or developer hand-backs; each shows twice (message plus completion notice), each re-sends ~230k context | largest avoidable manager cost | medium (not measured per step) |
| 8 | Merge-check stopped `ship` 3 times on a missing or malformed review file (tester file lost in its worktree twice; "Verdict:" missing once) | one reviewer round trip each | high |
| 9 | Owner reported ~95% of their real limit while the kit budget said 25% of its day cap | the kit budget does not track the owner's real allowance | medium (owner's definition unknown) |

## 2. Ranked recommendations

| # | change | type | evidence | est. saving | effort | risk |
|---|---|---|---|---|---|---|
| 1 | Barrier wait for reviews: one background script that waits until every required review file for a PR exists with a parsed verdict, then wakes the manager once. Also collapse the duplicate hand-back notice | missing feature | #7 | 0.5-1M cost-eq per 10 PRs | M | low |
| 2 | Reset or replace a reused reviewer or developer after N turns or ~120k context (carry the last findings file, not the history); reuse only inside one fix/re-review cycle | default to change | #3: ~150k vs ~80k per review | 30-50% of review cost | S | low (review pack carries context) |
| 3 | Budget must track the owner's real allowance: let the owner set the cap and the "used" figure (or read it), include manager cost, warn at 70/85/95% | kit bug | #9 | avoids a hard stop mid-feature | M | low |
| 4 | Triage defaults for domain reviewers: map their severity words to the same scale and decide nits, info and "deferred to slice N" automatically; only blockers and majors go to the tech lead | missing feature | #5 | ~100k per PR | M | low |
| 5 | Reviewer set by diff content, not path: security only where I/O, parsing, auth or dependencies change; config/types-only diffs skip it; keep the capital-protection reviewer on risk code | default to change | #6 | ~35k per skipped review | M | medium (conservative classifier) |
| 6 | Envelope and size caps count production lines only and are a per-stack setting (tests, docs, lockfiles excluded); stop asking "push as is?" when over the cap, just flag it | default to change | #4 | up to 80k per eligible PR; one turn per overflow | S | low |
| 7 | Review files written outside the main checkout must be copied by the writer recipe, or the recipe must write to the shared path; add a linter for the verdict line | kit bug | #8 | ~30-50k per occurrence | S | low |
| 8 | A recipe to commit the manager's own records (plan, decisions, board, strategy specs) through a docs PR; branch guard blocks direct commits so they stay uncommitted and block `sync-base` | missing feature | local base stayed behind the remote all session | ~20k per sync problem | S | low |
| 9 | Fix recipe quoting: `milestone` broke on semicolons, colons and parentheses; `budget-check` needs its argument form in the usage text | kit bug | 3 failed calls | small | S | low |
| 10 | Put prior blocker patterns into the next slice's brief (e.g. "a stop change with quantity > 0", "a verdict built from one passing rule") so the same defect is not found twice | habit | same defect class reached two PRs | one fix round (~60k) per repeat | S | low |
| 11 | Merge-check output lists what is missing and its fix, including ordering ("feature record must be on base") | missing feature | earlier features | ~20k | S | low |
| 12 | Name each reused agent per task in notifications ("Code review PR 44" label kept appearing for PRs 45-52) | kit bug | notifications and report rows | readability | S | low |
| 13 | Book usage per agent task, not one global current task | kit bug | rows from parallel features mixed | accuracy | S | low |

## 3. What worked and should stay

- Scripted triage for config-only findings and `ship` by script; no agent launched to merge.
- Warm developer continuation for the next slice and fix rounds (start 13k paid once per area).
- One planner run for a whole feature, with plan revisions on the same agent.
- The capital-protection reviewer reviewing the plan before any code: 8 blockers fixed at plan cost, not code cost.
- Fixed review pack, output path and finding ids; re-reviews update statuses in place.
- Merge checks that really blocked: missing tester sign-off, missing verdict, missing feature record.
- Deferral decisions written to the plan with finding ids and a named slice, so later lanes pick them up.
- Owner questions batched with a recommended answer first; decisions recorded in the decisions file.

## 4. Open questions

- What does one manager wake-up cost? A per-step manager counter would settle #1 and #7.
- Is there a turn count where a reused reviewer is clearly worse than a fresh one? Needs same-size re-reviews vs fresh reviews.
- How often does a security reviewer find anything on config/types-only PRs across features? Tally findings per reviewer per PR kind.
- What envelope cap fits pure-logic PRs with no I/O (around 400)? Needs the distribution of production lines per PR.
- Does the owner's real limit count cache reads and the manager's resends the way `cost-eq` does? Needed for #3.
- Would larger slices cut review cost, since 5 of 9 PRs needed a fix round regardless of size?

## 5. Earlier updates (kept short)

- F4 (21 runs, start share 16%, 18 reviews by agents, 0 by envelope): confirmed warm re-reviews worked; repeat findings across PRs cost one fix round each (now #10); `milestone` quoting (#9); `sync-base` blocked by untracked notes (#8).
- Confirmed this round: #1 (barrier wait), #2 (reviewer reuse cost), #3 (envelope never used), #5 (verdict format).
