<!-- agent-kit:start -->
# Agent team: index

You are one agent of an AI delivery team for Bazaar Rust web trading app. The manager (main session) coordinates and is the only one
who launches agents. Your own agent file says your job; read only what your task needs.

## Hard rules (all agents)

1. **Secrets:** never read, print or edit `.env*` (except `.env.example`), keys, `secrets/` or environment dumps; connection status only via `just db-check`.
2. **No real data:** tests use fakes, mock servers or test databases they start themselves; use only the stores and services `.agents/project.md` declares.
3. **Git:** your own branch from `just base`, Conventional Commit titles; merges only through `just ship` once a PR is approved (the manager runs it; developers never merge); the release branch moves only via `just promote` (the user approves); never delete, force-move or reset the integration or release branch.
4. **Quality:** developers write unit tests with every change; each step is verified T0 (gate) + T1 (one quick look), the full matrix once per feature; reviewers never change source.
5. **Scope and questions:** do your task only; ask instead of guessing on contracts, data meaning or security; replies <= 15 lines, details in files.
6. **Read narrowly:** `just codemap find <word>` before searching, diffs before whole files, `just docs "<topic>"` before whole handbooks, `just section <file> "<heading>"` before whole long notes (a hook blocks whole reads of big markdown files). Every tool call re-sends your context.
7. **Records and the kit:** feature and bug records travel with the code; if a kit step fails, stop and report (`just preflight`) - never work around it.

## Never break (Bazaar Rust web trading app)

- Rupee amounts are `rust_decimal::Decimal`, never `f64` (f64 only inside indicator/options math, converted at one boundary).
- Times are IST through the `Clock` trait; no wall-clock reads in domain code.
- Never hardcode lot/tick sizes, expiries, holidays, ban lists or charge rates: instrument master and `config/*.toml` with `source` and `verified_on`.
- No look-ahead: strategies and indicators see only data up to the closed bar.
- Every order goes through the DecisionGate and a user approval; there is no "place order" endpoint; `paper` is the default mode; `live-trading` is never a default feature.
- The server binds `127.0.0.1` only; every route except `/healthz`, `/auth` and the Kite callback needs a session.
- Legal data only (official APIs and exchange archives); never scrape.
- No panics in library code; every outbound call has a timeout.
- The disclaimer stays visible; never imply certainty or SEBI registration.
- Agents never run the HUMAN-ONLY recipes (`run`, `ui-dev`, `set-secret`, `refresh-instruments`, `import-bhavcopy`, `backtest`) or touch the live broker or the owner's `data/`; use fixtures, in-process router tests and temp dirs.

## Where to look

| Need | Where |
|---|---|
| what the project is, stores, integrations | `.agents/project.md` |
| code layout, routes, config keys, features, open bugs, in-flight branches | `.agents/codemap.md` (area index), `just codemap find <word>` |
| a rule or how-to | `just docs "<topic>"` (one section of a handbook) |
| team, files, all commands, hard rules in detail | `just docs "team-handbook: <topic>"` |
| the owner's standing rules | given to you at start (`.agents/owner-rules.md`) |
| a setting or the budget | grep `.agents/team.md`; `just budget` |
| status, a feature's history | `just status`, `just feature F<n>` |
| what each open PR needs next; a fix round's brief | `just next`; `just fix-list <pr>` |
| formats (plan, handoff, findings) | `.agents/templates/` |

Agents in worktrees get absolute paths to `.agents/` files in their prompt.

Domain agents: consult `market-strategist` when planning strategy/recommendation/risk/options/long-term work; `quant-reviewer` (quant, backtest, data, charges) and `risk-guardian` (risk, execution, paper, broker, scanner, risk.toml, order/approval/kill-switch endpoints and UI) are added automatically by `just review-plan` when a PR touches those paths, and merge-check requires their APPROVE.
<!-- agent-kit:end -->
