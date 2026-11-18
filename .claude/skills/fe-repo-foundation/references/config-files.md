# Config files

File map, package.json, TypeScript configs, vite.config.ts, lint/format, git hooks, Renovate, .gitignore and README.

## Contents

- File map
- package.json and toolchain
- TypeScript configs
- vite.config.ts
- Lint and format
- Git hooks and commit rules
- Repo automation and hygiene

## File map

This playbook gives the complete contents of every config and setup file a new repo needs; copy them in order and `pnpm verify` passes on the first run. Later docs in the series import from these files.

```text
web-app/
├─ .github/renovate.json        # dependency updates
├─ .husky/pre-commit, commit-msg # local quality gates
├─ e2e/                          # Playwright specs
├─ src/
│  ├─ shared/config/env.ts       # validated environment variables
│  ├─ shared/lib/http.ts         # typed fetch client
│  └─ test/                      # setup.ts, server.ts, handlers.ts, render.tsx
├─ .editorconfig  .gitignore  .nvmrc  .prettierrc.json  .prettierignore
├─ commitlint.config.js  eslint.config.js  .oxlintrc.json
├─ package.json  pnpm-workspace.yaml
├─ tsconfig.json  tsconfig.app.json  tsconfig.node.json
├─ vite.config.ts  vitest.config.ts  playwright.config.ts
└─ README.md
```

## package.json and toolchain

Pin Node and pnpm so every laptop and CI runner use identical versions; one `verify` script runs every gate in the same order CI does.

**`.nvmrc`**

```text
24
```

**`package.json`**

```json
{
  "name": "web-app",
  "private": true,
  "type": "module",
  "packageManager": "pnpm@11.28.5",
  "engines": { "node": ">=24" },
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "typecheck": "tsc -b",
    "lint": "oxlint --deny-warnings && eslint . --max-warnings=0",
    "lint:fix": "oxlint --fix && eslint . --fix",
    "format": "prettier --write .",
    "format:check": "prettier --check .",
    "test": "vitest",
    "test:ci": "vitest run --coverage",
    "e2e": "playwright test",
    "verify": "pnpm typecheck && pnpm lint && pnpm format:check && pnpm test:ci && pnpm build",
    "prepare": "husky"
  },
  "dependencies": {
    "@tanstack/react-query": "^5.0.0",
    "react": "^19.3.0",
    "react-dom": "^19.3.0",
    "react-hook-form": "^7.0.0",
    "zod": "^4.0.0"
  },
  "devDependencies": {
    "@axe-core/playwright": "latest",
    "@commitlint/cli": "latest",
    "@commitlint/config-conventional": "latest",
    "@eslint/js": "^10.0.0",
    "@playwright/test": "latest",
    "@rolldown/plugin-babel": "latest",
    "@testing-library/jest-dom": "latest",
    "@testing-library/react": "latest",
    "@testing-library/user-event": "latest",
    "@types/react": "^19.0.0",
    "@types/node": "^24.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^6.0.0",
    "@vitest/coverage-v8": "^4.1.0",
    "babel-plugin-react-compiler": "^1.0.0",
    "eslint": "^10.0.0",
    "eslint-plugin-jsx-a11y": "latest",
    "eslint-plugin-oxlint": "latest",
    "eslint-plugin-react-hooks": "latest",
    "globals": "latest",
    "husky": "^9.0.0",
    "jsdom": "latest",
    "lint-staged": "latest",
    "msw": "^2.0.0",
    "oxlint": "^1.0.0",
    "prettier": "^3.0.0",
    "typescript": "~6.0.0",
    "typescript-eslint": "latest",
    "vite": "^8.0.0",
    "vitest": "^4.1.0"
  },
  "lint-staged": {
    "*.{ts,tsx}": ["oxlint --fix", "eslint --fix --max-warnings=0", "prettier --write"],
    "*.{js,json,css,md,yml,yaml}": ["prettier --write"]
  }
}
```

Install with `pnpm add <pkg>` rather than editing versions by hand, so the lockfile records exact versions; `latest` above just means "whatever is current when you install". `typescript` is pinned with `~` because typescript-eslint must support the exact minor.

**`pnpm-workspace.yaml`** — pnpm 11 reads its settings from here, even in a single-package repo:

```yaml
# Refuse versions published less than 24 h ago (supply-chain protection).
minimumReleaseAge: 1440
```

pnpm 11 also blocks dependency install scripts by default; when a package genuinely needs one, pnpm prints the setting to add — review it in the PR like any other code.

## TypeScript configs

Two projects: browser code (`src/`) and Node code (config files, E2E). They need different globals, so mixing them hides real errors. `tsc -b` checks both; no file emits JavaScript — Vite does the compiling.

**`tsconfig.json`** — the root only references the two projects:

```json
{
  "files": [],
  "references": [{ "path": "./tsconfig.app.json" }, { "path": "./tsconfig.node.json" }]
}
```

**`tsconfig.app.json`** — application code:

```json
{
  "compilerOptions": {
    "tsBuildInfoFile": "./node_modules/.tmp/tsconfig.app.tsbuildinfo",
    "target": "ES2023",
    "lib": ["ES2023", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "types": ["vite/client", "vitest/globals"],
    "paths": { "@/*": ["./src/*"] },

    "strict": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitOverride": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,

    "verbatimModuleSyntax": true,
    "erasableSyntaxOnly": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["src"]
}
```

What the non-obvious flags buy you:

| Flag | Catches |
| --- | --- |
| `noUncheckedIndexedAccess` | `arr[0]` and `record[key]` are typed `T \| undefined`, so missing items can't crash at runtime |
| `exactOptionalPropertyTypes` | `{ name?: string }` no longer accepts an explicit `undefined`; prevents "set vs. unset" bugs in API payloads |
| `verbatimModuleSyntax` | Forces `import type` for types, so type-only imports never end up in the bundle |
| `erasableSyntaxOnly` | Bans `enum`, `namespace` and parameter properties — code stays strippable by any fast transpiler |

**`tsconfig.node.json`** — config files and Playwright tests:

```json
{
  "compilerOptions": {
    "tsBuildInfoFile": "./node_modules/.tmp/tsconfig.node.tsbuildinfo",
    "target": "ES2023",
    "lib": ["ES2023"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "types": ["node"],
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "verbatimModuleSyntax": true,
    "isolatedModules": true,
    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["vite.config.ts", "vitest.config.ts", "playwright.config.ts", "e2e"]
}
```

## vite.config.ts

Vite 8's React plugin (v6) uses Oxc instead of Babel, so the React Compiler now runs through a separate Babel plugin placed **before** `react()` — the [old `react({ babel })` form no longer works](https://recca0120.github.io/en/2026/04/14/react-compiler-vite-v6/).

```ts
import { fileURLToPath, URL } from 'node:url';
import { defineConfig } from 'vite';
import react, { reactCompilerPreset } from '@vitejs/plugin-react';
import { babel } from '@rolldown/plugin-babel';

export default defineConfig({
  plugins: [
    // Order matters: the compiler must see the original source before react() transforms it.
    babel({
      include: /\/src\/.*\.[jt]sx?$/,
      exclude: /\.(stories|test)\.[jt]sx?$/,
      babelConfig: reactCompilerPreset(),
    }),
    react(),
  ],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    port: 5173,
    strictPort: true,
    // Same-origin API calls in dev: no CORS setup, cookies just work.
    proxy: { '/api': { target: 'http://localhost:8080', changeOrigin: true } },
  },
  build: {
    sourcemap: 'hidden', // maps are generated for Sentry upload but not referenced by the bundle
    chunkSizeWarningLimit: 300, // KB; warns when a chunk should be split
  },
});
```

Notes:

- The build target is left at Vite's default, Baseline "widely available" browsers, which matches the CSS support policy in `fe-design-system`.
- Roll the compiler out safely: fix every `react-hooks` lint error first (its recommended preset includes the compiler's rules), because the compiler silently skips components that break the Rules of React.
- Opt a single component out with a `"use no memo"` directive while you fix it; never leave one there permanently.

## Lint and format

Oxlint runs first because it is 50–100x faster and catches most mistakes in milliseconds; ESLint then runs only what Oxlint can't — type-aware rules, React hooks and architecture boundaries. `eslint-plugin-oxlint` switches off the ESLint rules Oxlint already covers, so nothing runs twice.

**`.oxlintrc.json`**

```json
{
  "$schema": "./node_modules/oxlint/configuration_schema.json",
  "plugins": ["typescript", "react", "jsx-a11y", "import", "unicorn", "vitest"],
  "categories": { "correctness": "error", "suspicious": "warn" },
  "rules": {
    "no-console": ["error", { "allow": ["warn", "error"] }],
    "import/no-cycle": "error"
  },
  "ignorePatterns": ["dist", "coverage", "playwright-report"]
}
```

**`eslint.config.js`**

```js
import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';
import jsxA11y from 'eslint-plugin-jsx-a11y';
import oxlint from 'eslint-plugin-oxlint';
import { defineConfig } from 'eslint/config';

// Layer rules: app → pages → features → shared. Never import upward or into another feature's internals.
const deepFeatureImport = { group: ['@/features/*/*'], message: 'Import features via their index.ts only.' };
const upward = (...layers) =>
  layers.map((l) => ({ group: [`@/${l}`, `@/${l}/*`], message: `This layer must not import from ${l}/.` }));

export default defineConfig([
  { ignores: ['dist', 'coverage', 'playwright-report', 'test-results'] },
  js.configs.recommended,
  tseslint.configs.strictTypeChecked,
  tseslint.configs.stylisticTypeChecked,
  {
    languageOptions: {
      globals: globals.browser,
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    extends: ['react-hooks/recommended', jsxA11y.flatConfigs.recommended],
    rules: {
      '@typescript-eslint/consistent-type-imports': 'error',
      '@typescript-eslint/no-misused-promises': ['error', { checksVoidReturn: { attributes: false } }],
      '@typescript-eslint/restrict-template-expressions': ['error', { allowNumber: true }],
      'no-restricted-syntax': [
        'error',
        { selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']", message: 'Use shared/ui/SafeHtml (sanitized).' },
      ],
      'no-restricted-imports': ['error', { patterns: [deepFeatureImport] }],
    },
  },
  { files: ['src/features/**'], rules: { 'no-restricted-imports': ['error', { patterns: [deepFeatureImport, ...upward('pages', 'app')] }] } },
  { files: ['src/shared/**'], rules: { 'no-restricted-imports': ['error', { patterns: upward('features', 'pages', 'app') }] } },
  { files: ['src/shared/ui/safe-html.tsx'], rules: { 'no-restricted-syntax': 'off' } },
  { files: ['**/*.test.{ts,tsx}', 'src/test/**', 'e2e/**'], rules: { '@typescript-eslint/no-non-null-assertion': 'off' } },
  { files: ['**/*.{js,mjs}'], extends: [tseslint.configs.disableTypeChecked], languageOptions: { globals: globals.node } },
  ...oxlint.buildFromOxlintConfigFile('./.oxlintrc.json'), // keep last
]);
```

What this enforces beyond defaults: no floating promises (unhandled async errors), no `any` leaks, `import type` for types, no unsanitized HTML, and the layer boundaries from `fe-standards` — a PR that imports `features/checkout/components/x` from `search/` fails CI.

**`.prettierrc.json`** — choose once, never discuss again:

```json
{ "singleQuote": true, "trailingComma": "all", "printWidth": 100 }
```

**`.prettierignore`**

```text
dist
coverage
playwright-report
test-results
pnpm-lock.yaml
```

**`.editorconfig`** — consistent whitespace in any editor:

```ini
root = true

[*]
charset = utf-8
end_of_line = lf
indent_style = space
indent_size = 2
insert_final_newline = true
trim_trailing_whitespace = true
```

Commit `.vscode/extensions.json` recommending the Oxlint, ESLint and Prettier extensions, and `.vscode/settings.json` with format-on-save, so editors match CI.

## Git hooks and commit rules

Hooks give each engineer feedback in seconds; CI repeats every check because hooks can be skipped with `--no-verify`.

`pnpm install` runs the `prepare` script, which installs Husky. Then create two hook files (Husky 9 hooks are plain shell, no shebang needed):

**`.husky/pre-commit`** — lints and formats only the staged files:

```sh
pnpm exec lint-staged
```

**`.husky/commit-msg`** — rejects messages that aren't Conventional Commits:

```sh
pnpm exec commitlint --edit "$1"
```

**`commitlint.config.js`**

```js
export default {
  extends: ['@commitlint/config-conventional'],
  rules: {
    'scope-empty': [2, 'never'], // every commit names the area: feat(checkout): …
    'subject-case': [2, 'always', 'lower-case'],
    'header-max-length': [2, 'always', 72],
  },
};
```

Type-checking is deliberately left out of the pre-commit hook: `tsc -b` checks the whole project and would make every commit slow. It runs in CI and in the editor instead.

With squash-merge, the PR title becomes the commit on the integration branch (`develop`) and later the changelog entry; add a CI check (for example `amannn/action-semantic-pull-request`) so PR titles follow the same format.

## Repo automation and hygiene

Renovate keeps dependencies current in small weekly batches; patch and minor updates merge themselves when CI is green, so upgrades never pile up into a risky migration.

**`.github/renovate.json`**

```json
{
  "$schema": "https://docs.renovatebot.com/renovate-schema.json",
  "extends": ["config:recommended", ":semanticCommits", "helpers:pinGitHubActionDigests"],
  "schedule": ["before 6am on monday"],
  "minimumReleaseAge": "3 days",
  "prConcurrentLimit": 10,
  "lockFileMaintenance": { "enabled": true, "schedule": ["before 6am on monday"] },
  "packageRules": [
    { "matchUpdateTypes": ["minor", "patch"], "matchCurrentVersion": "!/^0/", "automerge": true },
    { "groupName": "react", "matchPackageNames": ["react", "react-dom", "@types/react", "@types/react-dom"] },
    { "groupName": "vite and vitest", "matchPackageNames": ["vite", "vitest", "@vitest/**", "@vitejs/**"] },
    { "groupName": "lint and format", "matchPackageNames": ["eslint", "@eslint/**", "eslint-plugin-**", "typescript-eslint", "oxlint", "prettier"] },
    { "groupName": "testing", "matchPackageNames": ["@playwright/test", "@testing-library/**", "msw", "@axe-core/**"] },
    { "matchPackageNames": ["typescript"], "automerge": false }
  ]
}
```

Why these settings: grouping turns ~40 PRs into ~5; the 3-day release age avoids freshly compromised or broken versions; GitHub Actions are pinned to commit SHAs; TypeScript never auto-merges because typescript-eslint must support each minor. Automerge only works if branch protection requires CI to pass.

**`.gitignore`**

```text
node_modules/
dist/
coverage/
playwright-report/
test-results/
*.tsbuildinfo
.env*.local
.DS_Store
.vscode/*
!.vscode/extensions.json
!.vscode/settings.json
```

**`README.md`** skeleton — every section answers a question a new engineer asks on day one:

```markdown
# web-app

One-sentence description of the product and who uses it.

## Quick start
nvm use && npm i -g pnpm@11
pnpm install
cp .env.example .env.local
pnpm dev            # http://localhost:5173

## Scripts
| Command | What it does |
| --- | --- |
| pnpm verify | Everything CI runs: types, lint, format, tests, build |
| pnpm test | Unit/component tests in watch mode |
| pnpm e2e | Playwright against a local production build |

## Architecture
Link to the front-end playbook and docs/adr/.

## Contributing
Branch naming, commit format, PR rules: see CONTRIBUTING.md.

## Owners and help
Team channel, on-call rotation, CODEOWNERS.
```
