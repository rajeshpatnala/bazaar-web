# Budget, reports and retrospectives

## Budget (measured automatically)

- `just budget-task <id>` when work on a task starts: hooks measure every agent's usage from its transcript and book
  it there; the manager's own session is measured too. Don't `budget-add` measured runs; `budget-add ... --estimate`
  only for a run the hooks could not see.
- Each run has `tokens` (context at its last step; limits use this), `start` (its cold start: what it loaded before
  doing anything) and `work` (the rest). `just budget-starts` shows the typical start per agent type: that is the
  overhead every fresh launch pays, and the reason to continue agents instead of relaunching them.
- Before a batch: `just budget-check <id> <agent>:<n> ...`. OK -> go. WARN -> go and tell the user. OVER LIMIT -> ask
  the user (raise, fewer agents, lower effort, smaller scope). `just budget` flags OVER AGENT CAP runs.
- Cheapest levers, in order: fewer cold starts (reuse, scripted triage, `ship`), right size, previews before taste
  work, T2 once, one `review-pack` per review, nothing restated in prompts.

## Report to the user (fixed format, status line first)

```
## <id> - <done | blocked | needs approval>
Delivered: PRs/branches, what changed (3-6 bullets)
Where to look: <page/route/command + what to do, one line each>
Quality: T0/T1 per slice, T2 <passed|n/a>; findings fixed/deferred counts
Release: in release PR vX.Y.Z | released vX.Y.Z | not yet
Budget: agents <measured> (cold starts <x>) + manager <measured>; task <used>/<limit>
Needs from you: approvals, .env key names to fill, questions
```

## Retrospective (L work, or any task over budget or with rework)

Append 5 lines from `.agents/templates/retro.md`: tokens by phase, cold starts vs work, rework, what the owner
corrected, what to change next time. Without this nothing can be measured or improved.
