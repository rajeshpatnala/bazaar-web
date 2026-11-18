# Feature records and bugs

**Ids.** Features `F<n>`, bugs `B<n>`, numbered after the highest existing (`just features`, `just bugs --all`).
Branches carry the id: `feat/F12-orders`, `fix/B7-null-total`.

**Feature record** (`docs/features/F<n>-<slug>.md`, template `.agents/templates/feature-record.md`): drafted in the
plan; the first PR commits it (`Status: building`); later PRs add *What was built*; the last sets `Status: done`.
Deferred findings and known limits go here. Later changes add a *History* line. merge-check requires it.

**Bug intake (manager):**
1. `just bug-new "<title>" --feature F<n> --severity <critical|high|medium|low>`; fill what/expected/reproduce from the user's words.
2. critical/high: tell the user at once, fix before feature work. medium/low: board row, ask whether to fix now.
3. One developer on `fix/B<n>-<slug>`: **failing regression test first, then the fix**; the PR commits
   `docs/bugs/B<n>-*.md` with root cause, fix, PR and regression test, and a *History* line in the feature record.
4. Normal reviews; merge-check requires the bug record. Won't-fix/duplicate: commit the record with that status.

**Questions about a feature:** `just feature F<n>` and the codemap first; read only the files it lists.

**Older features without records:** the tech lead lists them (git log, codemap, CHANGELOG) with ids; the user
confirms; one developer writes the records on `docs/F0-feature-records` (code review only).
