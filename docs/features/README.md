# Feature records

One file per feature, `F<n>-<slug>.md`, created by the feature's first PR from
`.agents/templates/feature-record.md` and kept current by later PRs. This folder is how anyone (people or agents)
learns what the product does, where each feature lives, what was decided, and what is known to be missing.

- `just features` lists them with status and open bugs.
- `just feature F7` shows one: the record, the PRs and files that touched it, which release shipped it, its bugs.
- `merge-check` blocks a feature PR whose record is missing (team.md -> Review policy -> require_feature_record).
