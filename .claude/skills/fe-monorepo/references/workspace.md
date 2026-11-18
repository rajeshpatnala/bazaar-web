# Workspace setup

When to move, package graph, workspace layout, catalog, root scripts and turbo.json.

## Contents

- When to move, and the package graph
- Workspace setup

## When to move, and the package graph

Move to a monorepo when a second app needs code the first one already has: a design system, API types, configs. With one app, the single-repo structure from `fe-repo-foundation`, `fe-app-architecture`, `fe-auth-realtime`, `fe-design-system`, `fe-testing`, `fe-performance`, `fe-cicd-observability`, `fe-team-workflow` is simpler and just as scalable. Moving later is cheap: `git mv` the existing repo into `apps/dashboard` and extract packages one at a time.

*Diagram: package graph · 2 apps, 2 shared packages, shared configs*

Arrows show allowed imports. Both apps use both shared packages; the packages never import an app or each other in the forbidden direction; and every package builds on the same TypeScript and lint configs.

## Workspace setup

pnpm workspaces link packages together. Catalogs pin each shared dependency to one version. Turborepo runs tasks in dependency order, in parallel, and caches the results. As of [Turborepo 2.10](https://turborepo.dev/blog/2-10) (June 2026), tasks are declared under `tasks` in `turbo.json`.

**Layout**

```text
acme-web/
├─ apps/
│  ├─ dashboard/            # Vite SPA from `fe-repo-foundation`, `fe-app-architecture`, `fe-auth-realtime`, `fe-design-system`, `fe-testing`, `fe-performance`, `fe-cicd-observability`, `fe-team-workflow` (signed-in product)
│  └─ marketing/            # Next.js app (public, server-rendered pages)
├─ packages/
│  ├─ ui/                   # design system from `fe-design-system` + Storybook
│  ├─ api-client/           # generated API types, Zod schemas, http client
│  ├─ config-typescript/    # shared tsconfig presets
│  └─ config-eslint/        # shared ESLint flat configs
├─ package.json  pnpm-workspace.yaml  turbo.json
└─ .github/  .nvmrc  .prettierrc.json
```

**`pnpm-workspace.yaml`**

```yaml
packages:
  - apps/*
  - packages/*

# One version of each shared dependency for the whole repo. Packages reference it as "catalog:".
catalog:
  react: ^19.3.0
  react-dom: ^19.3.0
  typescript: ~6.0.0
  zod: ^4.0.0
  '@tanstack/react-query': ^5.0.0
  vitest: ^4.1.0
  eslint: ^10.0.0

catalogMode: strict # adding a dependency that's in the catalog must use the catalog version
minimumReleaseAge: 1440 # supply-chain cooldown (`fe-repo-foundation`)
```

In each package, write `"react": "catalog:"` instead of a version. Upgrading React then means changing one line, and every app moves together.

**Root `package.json`** — scripts only delegate to Turborepo; the real scripts live in each package:

```json
{
  "name": "acme-web",
  "private": true,
  "packageManager": "pnpm@11.28.5",
  "engines": { "node": ">=24" },
  "scripts": {
    "dev": "turbo run dev",
    "build": "turbo run build",
    "typecheck": "turbo run typecheck",
    "lint": "turbo run lint",
    "test": "turbo run test",
    "verify": "turbo run typecheck lint test build",
    "format": "prettier --write .",
    "format:check": "prettier --check .",
    "prepare": "husky"
  },
  "devDependencies": {
    "turbo": "^2.10.0",
    "prettier": "^3.0.0",
    "husky": "^9.0.0",
    "lint-staged": "latest"
  }
}
```

**`turbo.json`**

```json
{
  "$schema": "https://turborepo.dev/schema.json",
  "ui": "tui",
  "globalDependencies": [".nvmrc", "pnpm-workspace.yaml"],
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "inputs": ["$TURBO_DEFAULT$", "!**/*.test.*", "!**/*.stories.*"],
      "outputs": ["dist/**", ".next/**", "!.next/cache/**"],
      "env": ["VITE_*", "NEXT_PUBLIC_*"]
    },
    "typecheck": {},
    "lint": {},
    "test": { "outputs": ["coverage/**"] },
    "dev": { "cache": false, "persistent": true }
  }
}
```

What matters in this file:

- `dependsOn: ["^build"]` builds a package's dependencies first (relevant only for compiled packages; source-only packages have no build step).
- `inputs` excludes tests and stories, so editing a test never invalidates a production build cache.
- `env` lists every build-time variable the bundle reads. A variable missing from the list means two builds with different values share a cache entry, and one environment ships the other's config.
- Each package defines its own `build`, `typecheck` (`tsc --noEmit`), `lint` and `test` scripts. Turborepo runs only the packages that have them.
