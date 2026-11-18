# Internal packages and boundaries

Config packages, UI package, API client package, boundary rules and enforcement, ownership.

## Contents

- Internal packages
- Boundaries and ownership

## Internal packages

Internal packages export TypeScript source directly (Turborepo calls these Just-in-Time packages). Each app's bundler compiles them along with its own code, so there's no build step, no stale `dist/`, and editing a component shows up in every app instantly. Compile a package only if a plain Node process or an npm publish consumes it.

### Shared TypeScript config

**`packages/config-typescript/package.json`**

```json
{ "name": "@acme/config-typescript", "private": true, "files": ["*.json"] }
```

**`packages/config-typescript/base.json`** — the strict options from `fe-repo-foundation`, in one place:

```json
{
  "compilerOptions": {
    "target": "ES2023",
    "lib": ["ES2023", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitOverride": true,
    "noFallthroughCasesInSwitch": true,
    "verbatimModuleSyntax": true,
    "erasableSyntaxOnly": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "skipLibCheck": true,
    "noEmit": true
  }
}
```

Each package or app then extends it and adds only what differs:

```json
// packages/ui/tsconfig.json
{ "extends": "@acme/config-typescript/base.json", "include": ["src"] }
```

### Shared ESLint config

**`packages/config-eslint/react.js`** — the `fe-repo-foundation` rules as a function, so each app passes its own root:

```js
import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';
import jsxA11y from 'eslint-plugin-jsx-a11y';
import { defineConfig } from 'eslint/config';

export function reactConfig({ tsconfigRootDir }) {
  return defineConfig([
    { ignores: ['dist', '.next', 'coverage', 'storybook-static'] },
    js.configs.recommended,
    tseslint.configs.strictTypeChecked,
    {
      languageOptions: { globals: globals.browser, parserOptions: { projectService: true, tsconfigRootDir } },
    },
    {
      files: ['**/*.{ts,tsx}'],
      plugins: { 'react-hooks': reactHooks },
      extends: ['react-hooks/recommended', jsxA11y.flatConfigs.recommended],
      rules: {
        '@typescript-eslint/consistent-type-imports': 'error',
        'no-restricted-syntax': ['error', { selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']", message: 'Use SafeHtml from @acme/ui.' }],
      },
    },
  ]);
}
```

The ESLint plugins are `dependencies` of `@acme/config-eslint` (with `eslint` as a peer), so apps install only the config package.

```js
// apps/dashboard/eslint.config.js
import { defineConfig } from 'eslint/config';
import { reactConfig } from '@acme/config-eslint/react';

export default defineConfig([
  ...reactConfig({ tsconfigRootDir: import.meta.dirname }),
  // app-specific rules (`fe-repo-foundation`'s feature boundaries) go here
]);
```

### The UI package

**`packages/ui/package.json`** — the `exports` map is the package's public API; anything not listed can't be imported:

```json
{
  "name": "@acme/ui",
  "private": true,
  "type": "module",
  "exports": {
    ".": "./src/index.ts",
    "./styles.css": "./src/styles/global.css"
  },
  "sideEffects": ["**/*.css"],
  "scripts": {
    "typecheck": "tsc --noEmit",
    "lint": "eslint .",
    "test": "vitest run",
    "storybook": "storybook dev -p 6006",
    "build-storybook": "storybook build"
  },
  "dependencies": { "dompurify": "^3.0.0" },
  "peerDependencies": { "react": "catalog:", "react-dom": "catalog:" },
  "devDependencies": {
    "@acme/config-eslint": "workspace:*",
    "@acme/config-typescript": "workspace:*",
    "typescript": "catalog:",
    "vitest": "catalog:"
  }
}
```

- React is a **peer** dependency, so every app supplies exactly one copy.
- Storybook lives in this package; it documents the components where they're built.
- Apps import `import { Button } from '@acme/ui'` and load the global styles once: `import '@acme/ui/styles.css'`.

### The API client package

When several apps call the same backend, the API's **shape** moves into `@acme/api-client`: the generated OpenAPI types (`fe-testing`), the Zod schemas that mirror them, and an HTTP client factory. Each app keeps its own business rules (`model/`) and its own TanStack Query keys and options (`api/`).

```ts
// packages/api-client/src/index.ts
export { Order, OrderPage, OrderStatus } from './schemas/orders';
export { createHttpClient, HttpError, type HttpClient } from './http';
```

```ts
// packages/api-client/src/http.ts — `fe-auth-realtime`'s client, with app-specific parts injected
export interface HttpClientOptions {
  baseUrl: () => string; // each app passes its runtime config
  onSessionExpired?: () => void; // each app decides what expiry means
}

export function createHttpClient(options: HttpClientOptions): HttpClient {
  // same request(), CSRF, single-flight refresh and error handling as `fe-auth-realtime`
}
```

## Boundaries and ownership

A monorepo turns into one big tangle unless dependencies only flow one way and every package hides its internals. Most of the enforcement comes free from tools you already have: pnpm refuses undeclared dependencies, `exports` maps hide internals, and lint rules block deep imports.

**Allowed dependencies**

| From ↓ / may import → | `ui` | `api-client` | `config-*` | another app |
| --- | --- | --- | --- | --- |
| **apps/** | Yes | Yes | Yes (dev) | **Never** — extract a package instead |
| **ui** | — | **No** (presentational only) | Yes (dev) | Never |
| **api-client** | No | — | Yes (dev) | Never |
| **config-*** | No | No | Yes | Never |

**How each rule is enforced**

| Rule | Enforced by |
| --- | --- |
| A package can only import what it declares in `package.json` | pnpm's strict, non-flat `node_modules` (undeclared imports fail to resolve) |
| Only the public API is importable | The `exports` map in each package (`@acme/ui/src/button/button` doesn't resolve) |
| No deep or relative cross-package imports | ESLint `no-restricted-imports` banning `@acme/*/src/*` and `../../packages/*` |
| Dependency direction (e.g. `ui` never imports `api-client`) | Review via CODEOWNERS, plus Turborepo's boundaries check with package tags where your version supports it |
| One version of shared dependencies | pnpm catalogs with `catalogMode: strict` |

Config packages expose their files the same way, for example `"exports": { "./react": "./react.js" }` in `@acme/config-eslint`.

**CODEOWNERS additions**

```text
/apps/dashboard/               @acme/product-web
/apps/marketing/               @acme/growth-web
/packages/ui/                  @acme/design-system
/packages/api-client/          @acme/web-platform
/packages/config-*/            @acme/web-platform
/pnpm-workspace.yaml           @acme/web-platform
/turbo.json                    @acme/web-platform
```

**Changing a shared package**

- **Additive changes** (a new prop, a new export) are normal PRs. Affected apps re-run their tests automatically.
- **Breaking changes** update the package **and every consumer in the same PR**. That's the main advantage of a monorepo: no app is ever left on an old version. The affected-tests run proves nothing broke.
- **Large migrations** (renaming a widely used component API) go deprecate-then-remove: add the new API, mark the old one `@deprecated` with a lint rule against new uses, migrate apps with a codemod over a few PRs, then delete it.
