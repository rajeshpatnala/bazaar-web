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
- Before coding, read `.agents/known-pitfalls.md` and the checklists named in your slice row (area done checklist, code-review checklist, risk-guardian PR checklist on its paths) (owner, 2026-11-23).
- Before a PR, self-review your diff against those checklists and the slice's failure modes; the handoff says "checklists: <sections>; self-check: <gaps fixed / none>" (owner, 2026-11-23).
- No slice is dispatched while its failure modes or policy decisions (plan template) are open (owner, 2026-11-23).
- Past 400 changed lines, stop: open what is done as its own PR and tell the manager (owner, 2026-11-23).
- At the end-of-feature test run (T2), the tester checks that every refusal test asserts which rule refused and every money test checks all fields that move (known-pitfalls #5) (owner, 2026-11-23).
- Consecutive slices in the same area go to the same developer while its context is under reuse_max_context; a fresh developer gets a handoff listing the earlier slices' major findings (owner, 2026-11-23).
