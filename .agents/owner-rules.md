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
