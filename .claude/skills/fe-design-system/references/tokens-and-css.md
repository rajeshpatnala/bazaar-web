# Tokens and global CSS

What gets built, token tiers, `tokens.css`, theming and cascade-layered global CSS.

## Contents

- What this playbook builds
- Token tiers
- tokens.css
- Global CSS

## What this playbook builds

A small, production-ready design system in `src/shared/ui`: tokens, global CSS, and the core components the earlier docs referenced (`Button`, `Field`, `Dialog`, `ConfirmDialog`, `Icon`, `SafeHtml`), documented and tested in Storybook. It uses only native CSS (custom properties, cascade layers, nesting, container queries), so there is zero runtime styling cost.

**Principles**

- **Components use semantic tokens only** (`--color-action-primary-bg`), never raw values or primitives. Restyling or adding a theme means editing tokens, not components.
- **Native HTML first.** `<dialog>`, `popover` and `<button>` bring focus handling, keyboard support and accessibility for free.
- **Variants are data attributes** (`data-variant="danger"`), which keeps CSS flat and readable in DevTools.
- **One component, one folder:** `button.tsx`, `button.module.css`, `button.stories.tsx`.

**Packages**

```bash
pnpm add dompurify
pnpm add -D stylelint stylelint-config-standard stylelint-config-css-modules stylelint-declaration-strict-value
pnpm dlx storybook@latest init   # detects React + Vite
```

**Files**

```text
src/shared/
├─ styles/
│  ├─ global.css        # layer order + imports (imported once in main.tsx)
│  ├─ reset.css  tokens.css  base.css  utilities.css
├─ lib/cx.ts            # className joiner
└─ ui/
   ├─ button/           # button.tsx, button.module.css, button.stories.tsx
   ├─ field/            # field.tsx, input.tsx, field.module.css
   ├─ dialog/           # dialog.tsx, confirm-dialog.tsx, dialog.module.css
   ├─ icon/             # icon.tsx, icons.svg (sprite)
   ├─ safe-html.tsx
   └─ index.ts          # public exports
```

## Token tiers

*Diagram: token tiers · primitive, semantic, component, rendered UI*

Each tier only references the one to its left. A rebrand edits primitives, a new theme edits semantic tokens, and a new button variant edits that button's private variables. None of these changes touches the others.

## tokens.css

Primitives define the raw palette and scales. Semantic tokens give each value a job ("primary action background") and pick light or dark values with `light-dark()`. Components read only semantic tokens. Dark mode then needs no extra CSS: setting `color-scheme` flips every `light-dark()` at once.

**`src/shared/styles/tokens.css`**

```css
@layer tokens {
  :root {
    /* ---------- Primitives: never used directly by components ---------- */
    --gray-0: oklch(100% 0 0);
    --gray-50: oklch(98.5% 0.002 260);
    --gray-100: oklch(96% 0.004 260);
    --gray-200: oklch(92% 0.006 260);
    --gray-400: oklch(70% 0.01 260);
    --gray-600: oklch(48% 0.014 260);
    --gray-800: oklch(28% 0.014 260);
    --gray-900: oklch(20% 0.012 260);
    --gray-950: oklch(15% 0.01 260);
    --blue-400: oklch(70% 0.15 255);
    --blue-500: oklch(60% 0.19 255);
    --blue-600: oklch(52% 0.2 255);
    --red-400: oklch(70% 0.17 25);
    --red-600: oklch(53% 0.21 25);
    --green-600: oklch(55% 0.15 150);
    --amber-500: oklch(76% 0.16 75);

    /* ---------- Semantic colours: light value, dark value ---------- */
    color-scheme: light dark; /* follows the OS unless data-theme overrides it */
    --color-bg: light-dark(var(--gray-0), var(--gray-950));
    --color-surface: light-dark(var(--gray-50), var(--gray-900));
    --color-surface-hover: light-dark(var(--gray-100), var(--gray-800));
    --color-text: light-dark(var(--gray-900), var(--gray-50));
    --color-text-muted: light-dark(var(--gray-600), var(--gray-400));
    --color-border: light-dark(var(--gray-200), var(--gray-800));
    --color-action-primary-bg: light-dark(var(--blue-600), var(--blue-400));
    --color-action-primary-bg-hover: light-dark(var(--blue-500), var(--blue-500));
    --color-action-primary-fg: light-dark(var(--gray-0), var(--gray-950));
    --color-danger-bg: light-dark(var(--red-600), var(--red-400));
    --color-danger-bg-hover: light-dark(var(--red-400), var(--red-600));
    --color-danger-fg: light-dark(var(--gray-0), var(--gray-950));
    --color-danger-text: light-dark(var(--red-600), var(--red-400));
    --color-success-text: light-dark(var(--green-600), var(--green-600));
    --color-warning-bg: light-dark(var(--amber-500), var(--amber-500));
    --color-focus: var(--blue-500);
    --color-backdrop: oklch(0% 0 0 / 0.5);

    /* ---------- Typography ---------- */
    --font-family-sans: system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif;
    --font-family-mono: ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
    --font-size-xs: 0.75rem;
    --font-size-sm: 0.875rem;
    --font-size-md: 1rem;
    --font-size-lg: 1.125rem;
    --font-size-xl: clamp(1.25rem, 1.1rem + 0.6vw, 1.5rem);
    --font-size-2xl: clamp(1.5rem, 1.2rem + 1.2vw, 2rem);
    --font-size-3xl: clamp(1.875rem, 1.4rem + 2vw, 2.75rem);
    --font-weight-regular: 400;
    --font-weight-medium: 500;
    --font-weight-bold: 650;
    --line-height-tight: 1.2;
    --line-height-body: 1.5;

    /* ---------- Space (4 px grid) ---------- */
    --space-1: 0.25rem;
    --space-2: 0.5rem;
    --space-3: 0.75rem;
    --space-4: 1rem;
    --space-6: 1.5rem;
    --space-8: 2rem;
    --space-12: 3rem;
    --space-16: 4rem;

    /* ---------- Shape, depth, motion ---------- */
    --radius-sm: 0.25rem;
    --radius-md: 0.5rem;
    --radius-lg: 0.75rem;
    --radius-full: 999px;
    --shadow-sm: 0 1px 2px oklch(0% 0 0 / 0.08);
    --shadow-lg: 0 12px 32px oklch(0% 0 0 / 0.18);
    --duration-fast: 120ms;
    --duration-normal: 200ms;
    --ease-standard: cubic-bezier(0.2, 0, 0, 1);

    /* ---------- Controls and focus ---------- */
    --control-height-sm: 2rem; /* 32 px, still above the WCAG 2.2 24 px minimum */
    --control-height-md: 2.5rem;
    --control-height-lg: 3rem;
    --focus-ring: 2px solid var(--color-focus);

    /* ---------- Layout ---------- */
    --content-max: 72rem;
    --z-sticky: 10; /* dialogs and popovers use the top layer and need no z-index */
  }

  :root[data-theme='light'] {
    color-scheme: light;
  }
  :root[data-theme='dark'] {
    color-scheme: dark;
  }
}
```

Check every text/background pair for contrast (4.5:1 for body text, 3:1 for large text and UI). The OKLCH lightness channel makes this predictable: keep about 50 lightness points between text and background.

Custom properties can't be used inside `@media` conditions, so breakpoints are a documented convention rather than tokens: `40rem` (small), `64rem` (medium), `80rem` (large). Prefer container queries for components.

**Theme switching** — the user's choice (`system` / `light` / `dark`) lives in the `fe-app-architecture` UI store; applying it is one attribute:

```ts
// src/shared/lib/theme.ts
export type ThemePreference = 'system' | 'light' | 'dark';

export function applyTheme(preference: ThemePreference): void {
  if (preference === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = preference;
}
```

To avoid a flash of the wrong theme on load, apply the saved preference before first paint with a tiny inline script in `index.html`. Allow it in your Content Security Policy by its SHA-256 hash.

```html
<script>
  try {
    const t = JSON.parse(localStorage.getItem('ui-prefs') ?? '{}').state?.theme;
    if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
  } catch {}
</script>
```

## Global CSS

Cascade layers fix the order of precedence up front: reset < tokens < base < components < utilities. A utility class always wins over a component style, and component styles always beat the base, whatever the selector specificity or file import order. Specificity fights and `!important` disappear.

**`src/shared/styles/global.css`** — imported once in `main.tsx`:

```css
/* Declaring the order first is what makes layers predictable. */
@layer reset, tokens, base, components, utilities;

@import './reset.css';
@import './tokens.css';
@import './base.css';
@import './utilities.css';
```

**`reset.css`** — modern and minimal:

```css
@layer reset {
  *, *::before, *::after { box-sizing: border-box; }
  * { margin: 0; }
  html { text-size-adjust: none; -webkit-text-size-adjust: none; }
  body { min-block-size: 100dvh; -webkit-font-smoothing: antialiased; }
  img, picture, video, canvas, svg { display: block; max-inline-size: 100%; }
  input, button, textarea, select { font: inherit; color: inherit; }
  p, h1, h2, h3, h4, h5, h6 { overflow-wrap: break-word; }
  :target { scroll-margin-block: 5ex; }

  @media (prefers-reduced-motion: reduce) {
    /* stylelint-disable-next-line declaration-no-important -- the one sanctioned use */
    *, *::before, *::after { animation-duration: 0.01ms !important; transition-duration: 0.01ms !important; scroll-behavior: auto !important; }
  }
}
```

**`base.css`** — element defaults, all from tokens:

```css
@layer base {
  body {
    background: var(--color-bg);
    color: var(--color-text);
    font-family: var(--font-family-sans);
    font-size: var(--font-size-md);
    line-height: var(--line-height-body);
  }
  h1, h2, h3 { line-height: var(--line-height-tight); text-wrap: balance; }
  h1 { font-size: var(--font-size-3xl); }
  h2 { font-size: var(--font-size-2xl); }
  h3 { font-size: var(--font-size-xl); }
  p { text-wrap: pretty; }
  a { color: var(--color-action-primary-bg); text-underline-offset: 0.15em; }
  :focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  code, kbd, pre { font-family: var(--font-family-mono); }
}
```

**`utilities.css`** — few and named for what they do:

```css
@layer utilities {
  .visually-hidden {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .skip-link {
    position: absolute;
    inset-inline-start: var(--space-2);
    inset-block-start: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background: var(--color-bg);
    transform: translateY(-200%);
    &:focus { transform: none; }
  }
  .stack { display: flex; flex-direction: column; gap: var(--stack-gap, var(--space-4)); }
  .cluster { display: flex; flex-wrap: wrap; align-items: center; gap: var(--cluster-gap, var(--space-2)); }
  .container { inline-size: min(100% - 2 * var(--space-4), var(--content-max)); margin-inline: auto; }
}
```

`.stack`, `.cluster` and `.container` cover most page layouts. Adjust spacing per use with the variable (`style={{ '--stack-gap': 'var(--space-8)' }}`) instead of creating near-duplicate classes.
