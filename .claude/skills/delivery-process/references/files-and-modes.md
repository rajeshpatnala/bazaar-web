# Files, work modes and approvals

## Work modes (`team.md` -> Parallelism -> work_mode; `just preflight` recommends one)

| Mode | Developers | Reviews and merge |
|---|---|---|
| `worktrees` (default) | parallel, each in its own worktree | PR: `review-pack <pr>`, `ship <pr>`; no gh: `--branch <b>` |
| `in-place` | one at a time in the main checkout, on its own feature branch | `review-pack --diff <base>..<branch>`, `ship --branch <b> --title <t>` |

- Branch missing: ask which branch the team uses (set it in `team.md` -> Branches) or `just setup-branches` with the user's OK.
- Worktrees fail: fix the cause, or `just work-mode in-place` with the user's OK.
- The manager's checkout stays on the integration branch; feature branches belong to agents.

## State files

| File | Owner | Purpose |
|---|---|---|
| `.agents/project.md` | manager | project profile (<= 60 lines); a digest linking the repo's own docs if it has them |
| `.agents/owner-rules.md` | manager | owner's standing rules, injected into every agent by a hook |
| `.agents/team.md` | user/manager | every setting; look one value up with grep, open the file only to change it |
| `.agents/board.md` | manager | every task: id, size, agent, branch/PR, status, blocker |
| `.agents/budget.csv` | hooks | measured usage per agent run (tokens, cold start, processed) |
| `.agents/codemap.md` (area index), `just codemap find <word>` | generated | modules, routes, config keys, features, open bugs, in-flight branches |
| `.agents/plans/<id>.md` | tech lead | ONE plan file per feature (M or L), in the main checkout. Agents in worktrees read it with `just plan <id> [--slice n]` (a slice brief: goal, acceptance, that slice's row) - nothing to commit or copy |
| `.agents/reviews/<id>/<pr-N or branch>/*.md` | reviewers | findings per PR at the paths `review-pack` prints; T2 report at `.agents/reviews/<id>/tests-t2.md` |
| `.agents/reports/<date>-<id>.md` | manager | report and retrospective |
| `docs/features/`, `docs/bugs/` | developers via PR | committed feature and bug records |

Committed: `project.md`, `owner-rules.md`, `team.md`, `decisions.md`, `templates/`, `docs/`. Everything else in
`.agents/` is local; agents in worktrees get **absolute paths** to it.

## Code map, decisions, approvals, hiring

- The codemap refreshes at session start and after merges/pulls/pushes/branch changes; `just ship` refreshes it too.
- Architecture changes update `docs/architecture.md` (architect) and `.agents/decisions.md` (tech lead) in the same feature.
- Ask the user before (team.md -> Approvals): starting a plan (if enabled); building look-and-feel work without a
  picked preview (if enabled); every promotion (always); new dependencies or stores; going over budget (always);
  real data, credentials, production, deleting data (always).
- Hiring: follow `agent-hiring`; new agents are found by their descriptions after the next session start.
