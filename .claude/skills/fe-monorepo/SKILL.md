---
name: fe-monorepo
description: "Scales the front end to several apps and shared packages: when to move to a monorepo, pnpm workspaces with catalogs, Turborepo tasks and caching, internal JIT packages (ui, api-client, config-typescript, config-eslint), package boundaries and ownership, affected-only CI and per-app deploys, and adding a Next.js 16 app for SEO/public pages. Use whenever adding a second app or shared package, editing pnpm-workspace.yaml or turbo.json, sharing code between apps, or building public SSR pages."
---

# Monorepo & multiple apps

Only needed when there is more than one app or a public SSR site. Moves shared code into packages without breaking the single-app conventions. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

**When and what**

1. **Move to a monorepo when a second app needs shared code, not before** — one app in a monorepo is overhead without benefit.
2. **Choose rendering per app, not per company** — a client-rendered SPA (Vite) for signed-in tools, server rendering (Next.js) for public pages that need SEO and fast first loads.
3. **Share packages, not copies** — the design system, API client and configs exist once; copy-paste across apps drifts within weeks.

**Structure**

4. **Two top-level folders: `apps/` (deployable) and `packages/` (shared)** — anyone can see what ships and what is reused.
5. **Dependencies point one way: apps → packages, never the reverse, never app → app** — a package that imports an app can't be reused.
6. **Each package has a narrow public API (`exports` in `package.json`)** — internals can change without breaking consumers.
7. **Internal packages ship TypeScript source, not build output** — no build step to forget, and edits show up instantly in every app.
8. **One version of each shared dependency, declared once in pnpm catalogs** — two Reacts in one repo cause bugs that are miserable to find.
9. **Shared configs are packages too** (TypeScript, ESLint, Prettier) — every app gets the same rules, and changing one rule is one PR.

**Speed and safety**

10. **Run only what a change affects, and cache everything** — Turborepo skips unaffected packages and replays cached results, so CI time tracks the size of the change, not the size of the repo.
11. **Deploy each app independently, only when it changed** — a typo fix in the marketing site never redeploys the dashboard.
12. **Every package has an owning team in CODEOWNERS** — shared code without an owner rots fastest.
13. **Internal packages always move together at `workspace:*`; no internal versioning** — every app uses the latest shared code, and breaking changes are fixed across the repo in the same PR.

## Workflow

1. Confirm the move is justified and sketch the package graph → `references/workspace.md` (When to move).
2. Workspace layout, `pnpm-workspace.yaml` catalog, root scripts, `turbo.json` → `workspace.md` (Workspace setup).
3. Create config packages, `@acme/ui`, `@acme/api-client` (move `http.ts` here) → `references/packages-and-boundaries.md`.
4. Enforce boundaries and update CODEOWNERS → `packages-and-boundaries.md` (Boundaries).
5. Affected CI runs and per-app deploys → `references/ci-and-nextjs.md` (CI and deploys); send workflow changes to `fe-cicd-observability`.
6. If public pages are needed, add the Next.js app → `ci-and-nextjs.md`.
7. Run the checklist for every new package or app.

## Files this playbook owns (Build mode)

`pnpm-workspace.yaml` (monorepo version), root `package.json` delegation, `turbo.json`, `packages/**`, `apps/marketing/**`, monorepo additions to CODEOWNERS (via `fe-team-workflow`).

## Done when

`pnpm turbo run build test lint` passes from the root and a second run is fully cached; PR CI runs only affected packages; apps import packages only through their `exports`; React and shared deps resolve to one version via the catalog; the Next.js app builds standalone and its server components fetch with revalidation.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/workspace.md` — When to move, package graph, workspace layout, catalog, root scripts and turbo.json.
- `references/packages-and-boundaries.md` — Config packages, UI package, API client package, boundary rules and enforcement, ownership.
- `references/ci-and-nextjs.md` — Affected runs, remote cache, per-app deploys, Next.js 16 app setup and the new package/app checklist.

## Sources

- [Turborepo 2.10 release — turborepo.dev](https://turborepo.dev/blog/2-10)
- [Turborepo field notes: tasks key, Just-in-Time packages, --affected — DEV Community](https://dev.to/ahmed_mahmoud360/turborepo-in-a-real-monorepo-field-notes-on-cache-misses-nextpublic-poisoning-and-the-tasks-key-4ojn)
- [Next.js 16: what's new and how to upgrade — Lucky Media](https://www.luckymedia.dev/blog/nextjs-16-an-early-look-and-release-date)
- [pnpm catalogs — pnpm.io](https://pnpm.io/11.x/catalogs)
