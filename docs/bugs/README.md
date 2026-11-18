# Bug records

One file per fixed (or closed) bug, `B<n>-<slug>.md`, from `.agents/templates/bug-report.md`.
Bugs start as local drafts in `.agents/bugs/` (`just bug-new "<title>" --feature F7 --severity high`);
the fix PR on branch `fix/B<n>-<slug>` commits the record here with the root cause and the regression test that
now guards it. `merge-check` requires that (team.md -> Review policy -> require_bug_record).

- `just bugs` lists open bugs (drafts and committed); `just bugs --all` includes fixed ones.
- `just feature F7` lists the bugs of one feature.
