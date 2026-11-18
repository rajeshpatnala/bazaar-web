---
name: fe-repo-foundation
description: "Creates and maintains the front-end repo foundation: Node 24/pnpm 11 pinning, package.json scripts, TypeScript project configs, vite.config.ts with React Compiler, Oxlint + ESLint 10 + Prettier, Husky/lint-staged/commitlint, validated env config, the typed HTTP client, Vitest + MSW + Playwright setup, Renovate and README. Use whenever starting a new front-end repo or creating, fixing or reviewing any tooling, lint, git-hook, env, HTTP-client or test-setup file, even if only one of those files is mentioned."
---

# Repo foundation & tooling

Builds the skeleton every other playbook builds on: after this playbook, `pnpm verify` runs every gate CI runs and passes on an empty app. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

1. **Pin Node and pnpm versions** (`.nvmrc`, `engines`, `packageManager`) — every laptop and CI runner builds identically.
2. **One `verify` command runs exactly what CI runs** — "works on my machine" stops being possible.
3. **Strict TypeScript plus `noUncheckedIndexedAccess` and `exactOptionalPropertyTypes`** — missing items and bad API payloads fail at compile time, not in production.
4. **Separate type-check projects for browser and Node code** — mixed globals hide real errors.
5. **Lint in two stages: Oxlint first, ESLint second** — feedback in milliseconds, with type-aware rules still enforced.
6. **Enforce architecture with lint rules, not a wiki page** — import boundaries between layers and features fail CI, so nobody has to remember them.
7. **Formatting is automatic and never discussed in review** — Prettier decides; reviewers look at logic.
8. **Git hooks are for speed, CI is the gate** — hooks can be skipped with `--no-verify`, CI can't.
9. **Conventional Commits, enforced** — readable history and automated changelogs.
10. **Validate environment variables once, at startup** — the app fails immediately on bad config; nothing else reads `import.meta.env`; nothing secret goes in `VITE_` variables.
11. **One HTTP client for the whole app** — central timeouts, cancellation and error shape; it returns `unknown`, so every caller must validate the data.
12. **Tests never touch a real network** — MSW answers every request and unhandled requests fail the test.
13. **E2E tests run against the production build in all major browsers** — retried passes are reported as flaky and fixed, not ignored.
14. **Update dependencies weekly in small groups** — patch and minor updates auto-merge on green CI after a 3-day release-age delay; upgrades never pile up.

## Workflow

1. Toolchain: `.nvmrc`, pnpm 11, `pnpm create vite@latest <app> --template react-ts`, `git init` → `references/config-files.md` (File map, package.json and toolchain).
2. TypeScript project configs → `config-files.md` (TypeScript configs).
3. `vite.config.ts` with the React Compiler preset placed **before** `react()` → `config-files.md` (vite.config.ts).
4. Oxlint, ESLint flat config with layer boundaries, Prettier, EditorConfig → `config-files.md` (Lint and format). Run `pnpm verify`.
5. Husky hooks and commitlint → `config-files.md` (Git hooks).
6. `env.ts`, `vite-env.d.ts`, `http.ts` → `references/env-and-http.md`. Run `pnpm verify`.
7. Vitest, MSW, render helper, first test, Playwright, smoke spec → `references/test-setup.md`. Run `pnpm verify` and `pnpm e2e`.
8. Renovate, `.gitignore`, `.env.example`, README skeleton → `config-files.md` (Repo automation).
9. Prove each gate catches failures: introduce one type error, one `any`, one failing assertion, one bad commit message — each must fail the right command.

## Files this playbook owns (Build mode)

`package.json` (first version), `pnpm-workspace.yaml`, `.nvmrc`, `tsconfig*.json`, `vite.config.ts`, `eslint.config.js`, `.oxlintrc.json`, `.prettierrc.json`, `.prettierignore`, `.editorconfig`, `.husky/*`, `commitlint.config.js`, `src/vite-env.d.ts`, `src/shared/config/env.ts`, `src/shared/lib/http.ts` (first version; `fe-auth-realtime` owns the final one), `vitest.config.ts` and `src/test/*` (first version; `fe-testing` extends), `playwright.config.ts`, `e2e/smoke.spec.ts`, `.github/renovate.json`, `.gitignore`, `.env.example`, `README.md`.

## Done when

Every row of the "Verify the setup" table in `references/test-setup.md` gives the expected result: `pnpm install` creates hooks; `pnpm verify` is green; `pnpm e2e` passes in all browser projects; a non-conventional commit message is rejected; a cross-feature deep import fails `pnpm lint`.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/config-files.md` — File map, package.json, TypeScript configs, vite.config.ts, lint/format, git hooks, Renovate, .gitignore and README.
- `references/env-and-http.md` — Zod-validated `env.ts`, `vite-env.d.ts` and the first `http.ts` with timeouts and `HttpError`.
- `references/test-setup.md` — Vitest config, MSW server and handlers, render helper, first tests, Playwright config, smoke spec, and the verify table.

## Sources

- [React Compiler 1.0 with Vite 8 and plugin-react v6](https://recca0120.github.io/en/2026/04/14/react-compiler-vite-v6/)
- [pnpm 11 release](https://doolpa.com/news/pnpm-11-released-esm-node-22-supply-chain-defaults-april-28-2026)
- [Announcing Vitest 4.0](https://voidzero.dev/posts/announcing-vitest-4)
- [ESLint v10](https://infoq.com/news/2026/04/eslint-10-release)
- [Announcing Oxlint 1.0](https://voidzero.dev/posts/announcing-oxlint-1-stable)
