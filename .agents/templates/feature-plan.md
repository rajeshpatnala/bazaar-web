# <feature-id>: <title>

Size: M | L   Estimate: <tokens>   Status: draft | approved | in progress | done   Owner: tech-lead

<!-- M: <= 40 lines - Goal, Acceptance, Steps (instead of Slices), Feature record, Risks. Delete empty sections.
     L: every section. One plan file per feature, never one per slice. Every step/slice: Verify T0+T1; T2 once at the end. -->

## Goal
<!-- one paragraph -->

## Acceptance criteria
- Given ..., when ..., then ...

## Out of scope
-

## Design notes
<!-- modules/packages touched, new ports/adapters, API contract sketch, data store and read/write pattern, error cases. Architect appends a Design section when needed. -->

## Steps (M: one developer does them in order, continued between steps)
1. 

## Slices (L only)
| slice | developer type | scope (modules/files owned) | depends on | order |
|---|---|---|---|---|
| 0 | dev-core | shared contracts: domain types, ports, DTOs | - | 1 |
| 1 | dev-data | | 0 | 2 (parallel) |

## Feature record
<!-- draft of docs/features/F<n>-<slug>.md (template: .agents/templates/feature-record.md): Status, Summary, Owner area,
     acceptance criteria, decisions, known limits. Slice 0 commits it; for a change to an existing feature, name the record to update. -->

## Test plan
<!-- link: <feature-id>-tests.md (written by tester) -->

## Security notes
<!-- trust boundaries, sensitive data, authz rules -->

## Risks / open questions
-
