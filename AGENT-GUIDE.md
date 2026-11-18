# Who does what, and how to ask

## The short version

**You only talk to the manager.** Open Claude Code in your repo and type what you want in plain words. The manager decides which agents to use, launches them, checks their work and reports back. You never have to remember agent names.

> "Add a page that lists invoices with filters"

The manager asks the tech lead to plan it, launches the right developers, has the work reviewed, and tells you when it's ready to merge.

## How the manager handles a request (and keeps cost down)

- **It sizes the work first** and tells you in one line before spending anything, e.g. *"M, one developer, ~250k tokens;
  plan next?"*. Small things get one developer and no plan; only big, multi-area work is split across parallel developers.
- **Look-and-feel requests get previews first.** If you ask for something to look or feel different ("make the homepage
  more premium", "nicer dashboard") the manager shows you up to three cheap previews and asks which you like.
  Nothing is built until you pick. You never have to describe a style in words; pointing is enough.
- **Your corrections become rules.** Say "cards must fit the screen" or "no pinned scrolling" once: it goes into
  `.agents/owner-rules.md`, every agent gets it automatically, and the whole product is checked for the same problem,
  not just the place you named.
- **Checks are tiered.** Each piece of work is checked quickly as it's built; the full test matrix runs once per feature.
- **Every report says where to look**: which page, route or command shows the change, so you can see it yourself.
- **Usage is measured, not guessed**: `just budget` and `just budget-report <id>` show real numbers per agent and for the manager.

## Asking for a specific agent

If you want a particular agent, name it in your request. The manager still coordinates it (budget, reviews, merge rules).

> "Have **dev-ui** do this: ..."
> "Ask the **security-reviewer** to look at PR 12"
> "Get the **architect** to design ..."

`just agents` lists every agent installed in this repo, with its model, effort and role, and which one the main session runs as.

## Who does what

### Leads

| Agent | Ask it for | Example request |
|---|---|---|
| manager | Anything; it's who you talk to | "What's the status of the board?" |
| tech-lead | Plans, splitting work, technical questions, decisions on review findings, merging into the integration branch | "Plan the invoices feature before anyone starts" |
| architect | Designs, choosing a data store or library, ADRs, non-functional requirements | "Design how we'll add multi-tenant support" |

### Developers (several of each can run at once)

| Agent | Main skill | Typical work |
|---|---|---|
| dev-ui | `fe-design-system` | tokens, components, Storybook, CSS |
| dev-auth | `fe-auth-realtime` | auth, sessions, guards, real-time |
| dev-perf | `fe-performance` | budgets, RUM, bundle, images, fonts |
| dev-a11y | `fe-a11y-i18n` | a11y, i18n, focus, translations |
| dev-platform | `fe-cicd-observability` | runtime config, flags, Sentry, deploy scripts |
| dev-app | `fe-app-architecture` | features, pages, data, forms, foundation, general |
| cicd | `ci-cd` | CI workflows, Docker, release automation, Dependabot |

Example requests:

> "Add a **dev-ui** task: ..."
> "Fix the failing build in CI" (goes to **cicd**)
> "Refactor the error handling" (goes to **dev-app**, the general developer)

### Reviewers and testers

| Agent | Ask it for | Example request |
|---|---|---|
| code-reviewer | Review a PR or branch | "Review PR 14" |
| security-reviewer | Security review of a PR or design | "Security-review the login changes" |
| tester | Test plans, missing tests, checking a feature works | "Write a test plan for invoices" / "Check PR 14 against its acceptance criteria" |
| build-runner | Running checks and reporting only the errors | "Run the gate on feat/F12-invoices" |

You don't need to ask for reviews: every PR gets a code review automatically, and a security review and tester when it's large or touches sensitive files (`just review-plan <pr>` shows which).

### Auditor

| Agent | Ask it for | Example request |
|---|---|---|
| fe-auditor | A full or scoped audit of the front end: security, payments, architecture, quality, accessibility, performance, testing, playbook gaps. Writes a report to `docs/audits/` | "Audit the whole front end" / "Audit the checkout flow for security" |

Fixes from an audit go through the normal flow: the tech lead triages the findings and developers fix them.

### Prototyper (look-and-feel requests)

| Agent | Ask it for | Example request |
|---|---|---|
| prototyper | Up to three clickable demo pages of different directions, before anything is built. You open `previews/<id>/index.html`, pick one, and say what to keep or drop | "Make the homepage feel more premium" / "Show me a few options for the product cards" |

You don't need to name it: the manager uses it automatically for "make it look/feel ..." requests.

## Common requests

| You want to... | Say something like |
|---|---|
| Build a feature | "Add CSV export to the reports page" |
| Report or fix a bug | "Users get a 500 when the cart is empty" (the manager files it as B<n>, asks how urgent, and fixes it with a regression test) |
| Ask about a feature | "What does F7 do?" / "Does invoices support refunds?" / "Which release shipped search?" |
| List features or bugs | "List our features" / "What bugs are open?" (`just features`, `just bugs`) |
| See progress | "What's in progress?" / "Show the board" |
| Check a PR | "Can PR 14 merge?" (the manager runs `just merge-check 14`) |
| Release | "Release what's on develop" (you approve the promotion to `main`) |
| Hire a new kind of agent | "I want to hire a new agent" (the manager interviews you) |
| Change a setting | "Give reviewers 25 turns" (or edit `.agents/team.md`; see `AGENT-SETTINGS.md`) |
| Check token use | "How much budget have we used today?" |

## How features and bugs are tracked

Every feature gets an id (`F1`, `F2`, ...) and a short record in `docs/features/`, committed with its code: what it does, where it lives,
decisions, known limits, and review findings deferred for later. Every bug gets an id (`B1`, ...); the fix PR commits `docs/bugs/B<n>-*.md`
with the root cause and the test that now guards it. Because these are in git, they survive new sessions, new laptops and new team members.

- `just features` - every feature with its status and open bugs
- `just feature F7` - one feature: its record, the PRs and files that built it, which release shipped it, its bugs
- `just bugs` - open bugs, most severe first
- The merge check refuses a feature PR without its record and a bug fix without its regression test (switch off in `.agents/team.md` -> Review policy).

Started using the kit on a project that already had features? Say "backfill the feature records" once.

## Running one agent on its own (advanced)

To work with a single agent without the manager, start Claude Code with it as the main session:

```bash
claude --agent dev-ui
```

Its safety hooks (secrets, branch guards, the developer gate) still apply, but the manager's coordination doesn't: no budget tracking, no automatic reviews, no board updates. Use it for quick one-off work; for anything that should be reviewed and merged, go through the manager.

## Where to read more

- `AGENT-KIT.md` - what the kit is, install and **upgrading**, safety, branches and releases, feature and bug records.
- `AGENT-SETTINGS.md` - every setting, with examples.
- `CLAUDE.md` - the rules every agent follows.
