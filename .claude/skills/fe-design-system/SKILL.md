---
name: fe-design-system
description: "Builds and governs the front-end design system: oklch design tokens with light/dark theming, cascade-layered global CSS, CSS Modules conventions, accessible core components (Button, Field, Input, Dialog, ConfirmDialog, Icon, SafeHtml), Storybook with a11y and interaction tests, Stylelint token enforcement and Style Dictionary. Use whenever writing CSS, adding or changing a shared UI component, theming, tokens, Storybook stories, or reviewing styling and component APIs."
---

# Design system & CSS

Creates `src/shared/ui` and `src/shared/styles`: the components and tokens every feature must use instead of ad-hoc markup and raw values. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

1. **Organize tokens in three tiers and let components use only semantic tokens** — a rebrand, a new theme or a new variant each changes one tier.
2. **Implement dark mode with `color-scheme` and `light-dark()`** — themes are one attribute, and the saved theme is applied before first paint, so there's no flash.
3. **Declare cascade layers once** (reset < tokens < base < components < utilities) — no specificity fights, no `!important`.
4. **Native HTML first** — `<dialog>`, `popover` and `<button>` bring focus handling, keyboard support and accessibility you'd otherwise write and maintain yourself.
5. **Components extend native props, spread the rest, and accept `ref` and `className`** — any `aria-*`, `data-*` or event works without new code.
6. **Give every optional prop a safe default** (`type="button"`) — components never surprise their callers.
7. **Express variants as `data-*` attributes plus private `--_` variables** — flat CSS that is readable in DevTools.
8. **Show state with real attributes** (`aria-invalid`, `aria-busy`) — assistive technology sees what the CSS sees. Loading buttons stay focusable.
9. **Let one `Field` component wire label, hint and error** — every form is accessible by default.
10. **Lay out with container queries, logical properties and `rem`** — components adapt to where they're placed, right-to-left works, and text scales.
11. **Meet accessibility baselines in the tokens themselves** — 4.5:1 text contrast, 24 px minimum targets, reduced motion respected globally.
12. **Keep shared UI free of business logic** — it never fetches data or imports from features.
13. **Document every component in Storybook with all its states** — accessibility violations and broken interactions fail CI.
14. **Enforce tokens with Stylelint and route design-system changes to its owners** — consistency doesn't depend on reviewers spotting a stray hex code.

## Workflow

1. Read "What this playbook builds" and "Token tiers" → `references/tokens-and-css.md`.
2. `tokens.css`, theme switching with the no-flash script, `global.css` layers and utilities → `tokens-and-css.md`.
3. Apply the component API conventions, add `cx.ts` and `react-css.d.ts` → `references/components.md` (Component API conventions).
4. Build Button, Field/Input, Dialog/ConfirmDialog, Icon, SafeHtml and `ui/index.ts` → `components.md` (Core components).
5. Storybook config, theme toolbar, stories with `play` tests → `references/storybook-and-enforcement.md`.
6. Stylelint `declaration-strict-value`, optional Style Dictionary pipeline → `storybook-and-enforcement.md` (Enforcement).
7. Run the new-component checklist for every component; run `pnpm verify` and Storybook tests.

## Files this playbook owns (Build mode)

`src/shared/styles/**`, `src/shared/ui/**` (except UI-state components owned by `fe-app-architecture`), `src/shared/lib/cx.ts`, `src/shared/lib/theme.ts`, `src/react-css.d.ts`, `.storybook/**`, `*.stories.tsx`, `.stylelintrc.json`, `style-dictionary.config.js`, the inline theme script in `index.html` (coordinate with `fe-performance`).

## Done when

No raw colours, spacing or font sizes outside `tokens.css` (Stylelint passes); light, dark and forced-colours modes render correctly; every component is keyboard operable with a visible focus ring and passes Storybook a11y checks at error level; Dialog traps focus natively and returns it on close; `dangerouslySetInnerHTML` only appears inside `SafeHtml`.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/tokens-and-css.md` — What gets built, token tiers, `tokens.css`, theming and cascade-layered global CSS.
- `references/components.md` — API conventions table, `cx`, CSS custom-property typing, and full source for the core components.
- `references/storybook-and-enforcement.md` — Storybook setup and stories, Stylelint, Style Dictionary and the new-component checklist.

## Sources

- [Baseline monthly digest, May 2026 — web.dev](https://web.dev/blog/baseline-digest-may-2026) (support status of modern CSS features)
