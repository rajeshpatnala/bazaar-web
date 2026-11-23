# Owner rules

Standing decisions by the project owner. They apply to every task and every part of the product, not only where
they were first said. A hook adds this list to every agent automatically, so dispatch prompts don't repeat it.

The manager adds a rule whenever the owner corrects something that could happen again ("no pinned scroll",
"every section fits the screen", "amounts always show two decimals"), then checks the whole product for the same
problem in one go. One line per rule, newest last, at most 20; merge or retire old ones rather than growing the list.

<!-- example:
- No pinned/sticky/scroll-jacking effects.
- Every section (heading + cards) fits the visible viewport on desktop.
-->

- Never delete branches (local or remote) after a merge: the merge command must not pass the delete-branch flag; feature branches are kept as history (owner, 2026-11-18).
- Before coding a slice, read `.agents/known-pitfalls.md` and the checklists the reviewers will use: your area's `just docs "<area skill>: done checklist"`, `just docs "code-review: checklist"`, and on risk-guardian paths `just docs "bz-trading-discipline: risk-guardian PR checklist"` (owner, 2026-11-23).
- Before opening a PR, self-review your own diff against those checklists and the slice's failure modes; fix or explain every gap. The handoff states "checklists: <sections>; self-check: <gaps fixed / none>" or the manager sends it back (owner, 2026-11-23).
- Safety code takes proof types (an approved order, a gate verdict), never raw intents or plain safety bools; every write has a tested failure path; refusal tests assert which rule refused (owner, 2026-11-23).
- Plans list per slice: failure modes, and policy decisions answered by risk-guardian / market-strategist / the owner before coding. A slice is not dispatched with either open (owner, 2026-11-23).
- Keep a PR within max_slice_changed_lines (400): when a change passes it, stop, open what is done as its own PR and tell the manager; the rest becomes the next slice (owner, 2026-11-23).
