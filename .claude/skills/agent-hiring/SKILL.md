---
name: agent-hiring
description: Interview-driven hiring of a new agent for the team. Use when the user says "add/hire/create a new agent", "we need someone who ...", "add a role for ...", or asks to change or retire an existing agent's role. The manager interviews the user, drafts a job card, gets approval, and installs the agent - the user never runs commands.
---

# Hiring a new agent

Treat it like hiring a person: understand the role, check the team doesn't already cover it, write a job card, get the user's OK, onboard. Keep the interview short: **at most two rounds of questions**, and skip anything the user already said.

## 1. Understand the role (round 1)

If the request doesn't say what the agent will do, first ask in one plain sentence:
"Tell me about the new team member: what will they work on, and why do we need them?"

Then ask the structured questions in **one** AskUserQuestion call (skip any already answered):

| Question | Options (user can always type their own) |
|---|---|
| What kind of work will they do? | Build code (developer) · Review others' work (reviewer) · Plan / design / decide (lead) · Run tools and report (runner) |
| What may they change? | Source code and tests · Only reports and findings · Only docs and plans · Nothing (read and run only) |
| How hard is the work? (sets effort) | Routine - medium (Recommended) · Complex - high · Very complex - xhigh |
| Which existing skills should they have? (multiSelect) | the 3-4 most relevant skills from `.claude/skills/` for the described role |

Map answers: kind = developer / reviewer / lead / runner. A "build code" answer means source write access; "reports only" means reviewer; "docs and plans" means lead; "nothing" means runner.

## 2. Fill the gaps (round 2, only if needed)

Ask in plain text, in a single message, only what is still unclear:
- **Special knowledge** not covered by existing skills (a library, protocol, domain, company standard)? This becomes a new skill.
- **What does "done" look like** for their work (a PR with tests, a findings file, a report)?
- **Who checks their work?** Default: code + security reviewers for developers; tech lead triage for reviewers.
- **Anything they must never do** beyond the team rules?
- **When should the manager call them** (which kinds of tasks)?

## 3. Check the existing team first

Compare the role with existing agents (`.claude/agents/*.md` descriptions). If an existing agent covers 70%+ of it, propose instead: "extend `<agent>` with a new skill" (cheaper, no new session needed for skill content). Hire only if the user still wants a separate agent.

## 4. Job card (get approval)

Show the user a short card and ask for "OK" or changes (AskUserQuestion with Approve / Change something):

```
New hire: <kebab-case-name>
Role: <one line>
Called for: <task types> (becomes the agent's description, written as "<Role>. Use for <task types>.")
Kind: developer | reviewer | lead | runner     Effort: <level>     Max turns: <n>
Can change: <scope>
Skills: <existing skills> + <new skill name, if any>
Done means: <deliverable>
Checked by: <agents>
Never: <extra restrictions>
Est. cost per launch: ~<tokens> (from similar agents in the budget ledger)
```

Name rules: kebab-case, 2-41 chars, describes the role (`perf-engineer`, `api-docs-reviewer`), no "claude"/"anthropic", not an existing name.

## 5. Onboard (after approval)

1. Write the role instructions to `.agents/hiring/<name>.md` (body only, no frontmatter), with these sections:
   - "You are **<name>** ..." plus the role in two sentences.
   - **Responsibilities**, **Steps** (numbered, concrete, including how they read the plan/codemap and where outputs go).
   - **Done means**, **Return** (short handoff format, <= 25 lines).
   - **Never**: extra restrictions. The script appends the standard team rules for the kind automatically.
2. If a new skill is needed: ask the `tech-lead` to write `.agents/hiring/<name>-skill.md` (body only: standards, procedures, examples; under 150 lines). For non-technical skills you may write it yourself.
3. Install:

```bash
python3 .claude/kit/new_agent.py <name> --kind <kind> --description "<Role>. Use for <task types>." \
  --skills <comma,separated> --effort <level> --max-turns <n> \
  --body-file .agents/hiring/<name>.md [--skill-file .agents/hiring/<name>-skill.md]
```

   The script creates `.claude/agents/<name>.md` (tools, worktree isolation, hooks and write limits set by kind), the skill if given, the `team.md` row, and for developers the unit-test/gate hook.
4. Run `just agents-check`; add a line to `.agents/decisions.md` ("hired <name> for <role>").
5. Tell the user: who was hired, what they'll be called for, and that a **new session** is needed before the new agent can be launched. Delete the files in `.agents/hiring/` afterwards.

## 6. Changing or retiring an agent

- **Change role, description or skills:** interview only about what changes, write the full updated instructions to `.agents/hiring/<name>.md`, show the user a short summary of what changes, and on approval run `python3 .claude/kit/new_agent.py <name> --update [--body-file ...] [--description "..."] [--skills a,b] [--skill-file ...]`. Only the given parts change; kind, tools and hooks stay as they were. A new session is needed.
- **Model, effort, turns:** edit `.agents/team.md` and run `just agents-sync`.
- **Retire:** confirm with the user, then run `python3 .claude/kit/new_agent.py <name> --retire`. This moves the file to `.claude/agents-retired/`, removes the `team.md` row and the hook registration. Retiring is reversible by moving the file back.
