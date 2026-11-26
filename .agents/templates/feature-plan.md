# <feature-id>: <title>

Size: M | L   Estimate: <tokens>   Status: draft | approved | in progress | done   Owner: tech-lead

<!-- M: <= 40 lines - Goal, Acceptance, Steps (instead of Slices), Feature record, Risks. Delete empty sections.
     L: every section. One plan file per feature, never one per slice. Every step/slice: Verify T0+T1; T2 once at the end. -->

## Goal
<!-- one paragraph -->

## Acceptance criteria
- Given ..., when ..., then ...
<!-- plan review: add every case from `just docs "plan checklist"` that applies, as its own criterion here (e.g. "Given
     the network is offline, when an approval is pending, then Approve stays disabled"). These criteria are the
     developer's requirements and what the final reviewers check. -->

## Out of scope
-

## Design notes
<!-- modules/packages touched, new ports/adapters, API contract sketch, data store and read/write pattern, error cases. Architect appends a Design section when needed. -->

## Steps (M: one developer does them in order, continued between steps)
<!-- each step: scope; failure modes; checklists; reject if (as in the slice table) -->
1. 

## Slices (L only)
| slice | developer type | scope (modules/files owned) | depends on | order | est. lines | live path | failure modes | checklists | reject if |
|---|---|---|---|---|---|---|---|---|---|
| 0 | dev-core | shared contracts and formulas every later slice uses: domain types, ports, DTOs, proof types | - | 1 | | no | | | |
| 1 | dev-data | | 0 | 2 (parallel) | | no | | | |
<!-- est. lines: counted lines (tests at half), between min_slice_lines and max_slice_changed_lines; `just plan` flags misses.
     live path: yes when the slice connects new code to a live or money path (that PR always gets the safety review);
     until then new code stays unused. Branch names carry the slice: feat/<id>-s<slice>-<slug>.
     failure modes: what happens when each write/call fails midway, what is audited, what is bounded, what Unknown does (fails closed).
     checklists: the `just docs` sections the developer reads first and self-reviews against (see .agents/known-pitfalls.md).
     reject if (tech lead): 3-5 concrete reasons a reviewer would reject this slice (1-2 under ~100 lines), from
     known-pitfalls, the checklists and earlier findings in these files; the developer writes those tests first.
     Carry new major findings from a slice's review into the next slice's brief. -->

## Policy decisions (answered before any slice is dispatched)
| question | answer | decided by (risk-guardian / market-strategist / owner) |
|---|---|---|

## Feature record
<!-- draft of docs/features/F<n>-<slug>.md (template: .agents/templates/feature-record.md): Status, Summary, Owner area,
     acceptance criteria, decisions, known limits. Slice 0 commits it; for a change to an existing feature, name the record to update. -->

## Test plan
<!-- link: <feature-id>-tests.md (written by tester) -->

## Security notes
<!-- trust boundaries, sensitive data, authz rules -->

## Owner inputs needed
<!-- every file, rate, calendar or decision the owner must supply, asked once at plan time -->
-

## Risks / open questions
-
