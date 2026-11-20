<!-- Rules for the manager's part of an efficiency review (`just efficiency <feature>` writes the numbers and flags).
     Add at most 15 lines under "Observations (manager)". Use scripts only: launch no agents, read no raw transcripts or diffs.
     Every line:
     - is stack- and repo-neutral (it must apply to any kit); repo details only as an anonymized example in brackets
     - names its evidence (a number from the tables above, or a concrete failed command / repeated step you saw)
     - says its type: kit bug | missing kit feature | default to change | manager or agent habit
     - proposes nothing that weakens owner rules or safety guards (merge checks, branch protection, secrets)
     - is aimed at a big share of spend first (the "Spend by role" table): long developer runs, review rounds and the
       manager usually dwarf cold starts; never propose warm reviewers across PRs (they cost more per review)
     Cover what the numbers can't show: failed or blocked commands, manual workarounds, things read twice,
     waiting, unclear instructions. Then list what worked and should stay (<= 3 lines). -->
