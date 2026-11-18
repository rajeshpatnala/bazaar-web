---
name: fe-cicd-observability
description: "Builds the front-end delivery pipeline and production visibility: GitHub Actions CI with merge queue, previews and sharded E2E, repository rulesets, build-once-promote deploys with runtime config.json, canary releases and rollback, version checks and chunk-error recovery, OpenFeature feature flags with owners and expiry, Sentry error and route tracing with source maps, SLO alerts, runbooks and the release checklist. Use whenever editing CI workflows, deploy scripts, environment config, feature flags, error reporting, monitoring or alerts, or planning a release."
---

# CI/CD, deployment & observability

Gets every merged change to users safely and makes production failures visible. Owns the final CI workflow, `main.tsx` startup order and `reportError`. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

**Continuous integration**

1. **Every PR runs the same automated gates, and they are required to merge** — quality doesn't depend on who reviewed it or how busy they were.
2. **Use a merge queue** — it tests each PR combined with everything ahead of it, so two individually green PRs can't break the integration branch (`develop`) together.
3. **Keep CI fast and give each stage a budget** — slow pipelines push engineers to batch changes, which makes every release riskier.
4. **Lock down the pipeline** — least-privilege tokens, actions pinned to commit SHAs, cloud access through short-lived OIDC credentials, no long-lived secrets.

**Delivery**

5. **Build once, promote the same artifact** — what passed tests on staging is byte-for-byte what reaches production. Environment differences live in runtime config, not in separate builds.
6. **Deploy atomically and keep old assets** — upload hashed assets first and switch `index.html` last; never delete the previous release's files, so users with an open tab keep working.
7. **Separate deploy from release** — code ships dark behind feature flags, and turning a feature on is a config change, not a deploy.
8. **Roll out gradually and watch** — canary to a small share of traffic, compare error rates and Web Vitals against the previous release, then widen.
9. **Rollback is one command, rehearsed** — repoint to the previous release in seconds. Fix forward only when rollback is impossible.
10. **Tell open tabs when a new version is out** — offer a reload instead of letting stale code talk to a newer API.

**Observability**

11. **Every error, trace and metric carries the release id and route** — a regression points straight at the change that caused it.
12. **Upload source maps to the error tracker, never to the public** — readable stack traces without exposing source code.
13. **Alert on user impact, not on noise** — SLO-based alerts (crash-free sessions, Web Vitals, key-flow success) routed to the owning team, each with a runbook.
14. **Never send personal data to monitoring tools** — scrub it in the browser before anything leaves the device.
15. **Feature flags have owners and expiry dates** — a flag left in the code for months becomes untested, permanent complexity.

## Workflow

1. Understand build-once/promote → `references/ci.md` (Build once).
2. `ci.yml` jobs (pr-title, verify, visual, preview, e2e-smoke, e2e-full) and the ruleset → `ci.md`. Merge in steps sent by `fe-performance`, `fe-testing` and `fe-monorepo`.
3. Runtime config (`config/<env>.json`, `runtime-config.ts`), startup order in `main.tsx`, `deploy.yml`, `deploy.sh`, version check → `references/deployment.md`.
4. Flag registry, OpenFeature provider, `useFlag`, flag rules → `references/flags-and-observability.md` (Feature flags).
5. Sentry init with scrubbing, route tracing, source-map upload, `reportError`, dashboards, SLOs, runbook → `flags-and-observability.md` (Observability).
6. Release checklist → `flags-and-observability.md`.

## Files this playbook owns (Build mode)

`.github/workflows/*.yml`, `scripts/deploy*.sh`, `config/*.json`, `src/shared/config/runtime-config.ts`, `src/app/main.tsx` startup order (with `fe-app-architecture`), `src/shared/lib/errors.ts#reportError` (final), `src/app/observability.ts`, `src/app/version-check.ts`, `src/shared/config/flags.ts`, `src/app/flags.ts`, `docs/runbooks/*`.

## Done when

PRs run verify, visual, preview and smoke E2E; the merge queue runs the full sharded suite; the same build artifact deploys to staging and production with only `config.json` differing; a canary can be promoted or rolled back in minutes; old chunks stay available after deploy; every flag has an owner and remove-by date; errors reach Sentry with readable stack traces and no query strings or PII.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/ci.md` — Build-once model, the full `ci.yml` and the repository ruleset.
- `references/deployment.md` — Runtime config, startup order, deploy workflow and script, promotion, version checks and chunk-error recovery.
- `references/flags-and-observability.md` — Flag registry and provider, Sentry setup, dashboards, SLO alerts, runbook template and release checklist.
