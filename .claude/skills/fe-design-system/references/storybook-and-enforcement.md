# Storybook, enforcement and checklist

Storybook setup and stories, Stylelint, Style Dictionary and the new-component checklist.

## Contents

- Storybook
- Enforcement
- New-component checklist

## Storybook

Storybook is the design system's living documentation and its test bench. Every story is rendered in a real browser and checked for accessibility violations and interaction bugs in CI, so the catalogue can't drift from the code.

**Setup**

```bash
pnpm dlx storybook@latest init                     # React + Vite detected automatically
pnpm dlx storybook@latest add @storybook/addon-a11y
pnpm dlx storybook@latest add @storybook/addon-vitest  # runs every story as a Vitest browser test
```

The Vitest addon adds a `storybook` project to `vitest.config.ts`, so `pnpm test:ci` (and therefore `pnpm verify`) runs all stories, including their accessibility checks and `play` functions.

**`.storybook/main.ts`**

```ts
import type { StorybookConfig } from '@storybook/react-vite';

const config: StorybookConfig = {
  framework: '@storybook/react-vite',
  stories: ['../src/**/*.stories.tsx'],
  addons: ['@storybook/addon-docs', '@storybook/addon-a11y', '@storybook/addon-vitest'],
};

export default config;
```

**`.storybook/preview.tsx`** — global styles, a theme toolbar, and accessibility violations treated as failures:

```tsx
import type { Preview } from '@storybook/react-vite';
import '../src/shared/styles/global.css';

const preview: Preview = {
  parameters: {
    layout: 'centered',
    a11y: { test: 'error' }, // an axe violation fails the story's test
  },
  globalTypes: {
    theme: {
      description: 'Colour theme',
      toolbar: { icon: 'mirror', items: ['light', 'dark'], dynamicTitle: true },
    },
  },
  initialGlobals: { theme: 'light' },
  decorators: [
    (Story, { globals }) => {
      document.documentElement.dataset.theme = String(globals.theme);
      return <Story />;
    },
  ],
};

export default preview;
```

**`ui/button/button.stories.tsx`** — one story per meaningful state, with a `play` function to test behaviour:

```tsx
import type { Meta, StoryObj } from '@storybook/react-vite';
import { expect, fn } from 'storybook/test';
import { Button } from './button';

const meta = {
  component: Button,
  args: { children: 'Save changes', onClick: fn() },
  argTypes: {
    variant: { control: 'inline-radio', options: ['primary', 'secondary', 'ghost', 'danger'] },
    size: { control: 'inline-radio', options: ['sm', 'md', 'lg'] },
  },
} satisfies Meta<typeof Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Primary: Story = {};
export const Secondary: Story = { args: { variant: 'secondary' } };
export const Danger: Story = { args: { variant: 'danger', children: 'Delete order' } };

export const Loading: Story = {
  args: { loading: true },
  play: async ({ args, canvas, userEvent }) => {
    const button = canvas.getByRole('button', { name: 'Save changes' });
    await userEvent.click(button);
    await expect(args.onClick).not.toHaveBeenCalled(); // repeat clicks are ignored
    await expect(button).toHaveAttribute('aria-busy', 'true');
  },
};
```

**Story conventions**

- Stories sit next to the component and use the component's real props — no wrappers that hide its API.
- Cover every variant, every size, and the hard states: loading, disabled, error, long text, right-to-left (`dir="rtl"`), and dark theme.
- Interaction behaviour lives in `play` functions; unit tests stay for pure logic.
- Publish Storybook from `develop` (any static host). Designers review components there, not in screenshots.
- For visual regression, use Vitest's `toMatchScreenshot` (`fe-testing`) or a hosted service such as Chromatic on the same stories.

## Enforcement

A design system only stays consistent if the rules are machine-checked. Stylelint rejects raw colours, spacing and radii in component CSS, so every value comes from a token. A PR that adds `#3b82f6` or `13px` fails CI with a message pointing to the token to use.

**`.stylelintrc.json`**

```json
{
  "extends": ["stylelint-config-standard", "stylelint-config-css-modules"],
  "plugins": ["stylelint-declaration-strict-value"],
  "rules": {
    "scale-unlimited/declaration-strict-value": [
      ["/color$/", "background", "fill", "stroke", "border-radius", "box-shadow", "z-index", "/^(margin|padding|gap)/", "font-size"],
      {
        "ignoreValues": ["0", "auto", "inherit", "currentColor", "transparent", "none", "1em"],
        "disableFix": true
      }
    ],
    "declaration-no-important": true,
    "selector-max-id": 0,
    "max-nesting-depth": 3,
    "selector-class-pattern": ["^[a-z][a-zA-Z0-9]*$", { "message": "Use camelCase class names (styles.orderCard)" }]
  },
  "overrides": [{ "files": ["src/shared/styles/tokens.css"], "rules": { "scale-unlimited/declaration-strict-value": null } }]
}
```

Add `"lint:css": "stylelint \"src/**/*.css\""` to `package.json`, include it in `verify`, and add `"*.css": ["stylelint --fix", "prettier --write"]` to `lint-staged`. Also add `".storybook"` to the `include` array of `tsconfig.app.json`, so story config is type-checked and linted like app code.

**Design tokens from Figma (when a designer maintains Figma variables)**

The primitives in `tokens.css` can be generated from Figma, so code and design never drift. Designers export variables in the W3C Design Tokens (DTCG) JSON format, and Style Dictionary turns them into CSS. Semantic tokens stay hand-written in `tokens.css`, because they encode decisions (which grey is "muted text") that engineers review.

```bash
pnpm add -D style-dictionary
```

```js
// style-dictionary.config.js
export default {
  source: ['design-tokens/**/*.json'], // DTCG export from Figma variables, committed to the repo
  platforms: {
    css: {
      transformGroup: 'css',
      buildPath: 'src/shared/styles/generated/',
      files: [{ destination: 'primitives.css', format: 'css/variables', options: { outputReferences: true } }],
    },
  },
};
```

Add `"tokens": "style-dictionary build --config style-dictionary.config.js"`, add `@import './generated/primitives.css' layer(tokens);` to `global.css` just before the `tokens.css` import, and delete the hand-written primitives. Token changes then arrive as a reviewable PR that touches JSON and generated CSS only.

**Ownership** — CODEOWNERS routes `src/shared/ui/`, `src/shared/styles/` and `design-tokens/` to the design-system owners, so a token rename can't land without a review from the people who understand its blast radius.

## New-component checklist

Copy this into the PR description when adding to `src/shared/ui`.

- [ ] Not already covered by an existing component or a native element (`<dialog>`, `popover`, `<details>`).
- [ ] Extends the native element's props, spreads the rest, accepts `ref` and `className`.
- [ ] Variants and sizes are string unions rendered as `data-*` attributes; every optional prop has a safe default.
- [ ] Styles in a CSS module inside `@layer components`, semantic tokens only (Stylelint passes).
- [ ] Keyboard: reachable with Tab, operable with Enter/Space/Escape/arrows as the matching native widget would be; visible focus.
- [ ] Screen reader: correct role and name, state exposed via `aria-*`, errors and hints linked with `aria-describedby`.
- [ ] Works in dark theme, at 200% zoom, in right-to-left, and with long or translated text.
- [ ] Pointer target at least 24×24 CSS px; motion respects `prefers-reduced-motion`.
- [ ] Stories for every variant and hard state; a `play` function for interactive behaviour; zero axe violations.
- [ ] Exported from `ui/index.ts`; a design-system CODEOWNER approved.
