# KIT-DEVBRIEF: better first drafts from developer instructions (owner approved all 5, 2026-11-23)

Branch: `fix/kit-dev-brief` from `just base` (#143 is merged). Template edits (owner approved), first commit:
- `.agents/templates/feature-plan.md`: slice table gets a last column `reject if` (header, separator, both example rows);
  comment block gets, before "Carry new major findings": "reject if (tech lead): 3-5 concrete reasons a reviewer would
  reject this slice (1-2 under ~100 lines), from known-pitfalls, the checklists and earlier findings in these files; the
  developer writes those tests first."; under "## Steps (M ...)" add the comment
  `<!-- each step: scope; failure modes; checklists; reject if (as in the slice table) -->`.
- `.agents/templates/handoff.md`: before "Notes:" add
  `Failure modes: <mode> -> <how handled> -> <test name>; one per plan failure mode and reject-if line (gap = say so)` and
  `Checklists: <sections>; self-check: <gaps fixed / none>; same pattern searched in whole diff: yes`. One PR. Approx. 150-250 lines. Area: kit and agent files.

1. **Recent findings in the slice brief** (`kit.py plan <id> --slice <n>`): after the brief, print "Reviewers caught
   in these files before:" with the last 5-8 blocker/high/major findings (id, one-line title) from `.agents/reviews/`
   whose file path matches the slice's scope column (path prefix match). None -> print nothing. Bounded output; a
   missing or garbled findings file is skipped with a one-line note, never a crash. Tests: match, no match, cap, garbled file.
2. **Reject-if lines** (the tech lead writes them; the plan template already has the `reject if` column):
   - `plan --slice` prints the slice's `reject if` cell under the heading "Write these tests first:".
   - `tech-lead.md`, planning step: "For each slice, fill failure modes, checklists and 3-5 `reject if` lines (1-2 under
     ~100 lines) from known-pitfalls, the checklists and `just plan`'s earlier findings."
   - `plan` flags an L slice row with an empty `reject if` column the same way it flags other missing cells.
3. **Fix the pattern, not the instance** (`kit.py fix-list`): the brief ends with "For each finding, search your whole
   diff for the same pattern and fix every occurrence; list where in the handoff." Test: line is present.
4. **Handoff table**: in all 7 developer agent files' Handoff block, add the two lines from `.agents/templates/handoff.md`
   (`Failure modes: <mode> -> <how handled> -> <test name>; ...` and `Checklists: <sections>; self-check: ...; same
   pattern searched in whole diff: yes`).
5. **Steps in the agent files** (dev-core, dev-api, dev-backtest, dev-data, dev-execution, dev-quant, dev-ui):
   - after step 1 add: "Read `.agents/known-pitfalls.md` and the checklists in your slice row; write the tests for the
     slice's `reject if` lines and failure modes first."
   - before the PR step add: "Self-review the diff against those checklists and the failure modes; fill the handoff's
     failure-mode table; any gap is fixed or named."
   - keep each file's wording style; `just agents-sync` must leave the edits intact (it only rewrites front matter
     and the handbook block - check).

Acceptance: `just plan BZ-511 --slice 2` shows the findings and reject-if sections; `just fix-list <any merged pr>`
ends with the pattern line; kit tests pass; `just agents-sync` is a no-op on the step text.
6. **Top 10 per area** (added, owner approved): plan --slice prints .agents/checklists/<developer-type>.md (cap 15 lines); files written by the tech lead (KIT-TOP10).
