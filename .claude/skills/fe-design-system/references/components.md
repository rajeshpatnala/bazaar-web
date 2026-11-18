# Component conventions and core components

API conventions table, `cx`, CSS custom-property typing, and full source for the core components.

## Contents

- Component API conventions
- Core components

## Component API conventions

Every shared component follows the same contract, so engineers can use one they've never seen without reading its source.

| Rule | Convention | Why |
| --- | --- | --- |
| Native props | Extend the element's props: `ComponentPropsWithRef<'button'>`, and spread the rest onto the element | `aria-*`, `data-*`, `onKeyDown` and `ref` all work without extra code |
| Refs | Plain `ref` prop (React 19); no `forwardRef` | Simpler types, works with React Hook Form's `register` |
| Variants | `variant` and `size` string unions, rendered as `data-variant` / `data-size` | Flat CSS, visible in DevTools, no class-name juggling |
| Defaults | Every optional prop has a safe default (`type="button"`, `variant="primary"`) | A `<Button>` inside a form never submits it by accident |
| States | Use real attributes: `disabled`, `aria-invalid`, `aria-busy`, `aria-expanded` | Assistive tech sees what the CSS sees |
| Styling hooks | Accept `className` and merge it last | Pages can adjust layout (margins, width) without forking |
| Children | `children` for content; named slots (`footer`, `icon`) only for distinct regions | Composition over configuration props |
| Naming | Event props `onX`, booleans as adjectives (`loading`, not `isLoading`, to match `disabled`) | Matches native HTML vocabulary |
| No business logic | Shared UI never fetches data or imports from `features/` | Enforced by the `fe-repo-foundation` lint boundaries |

**`src/shared/lib/cx.ts`**

```ts
export function cx(...classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(' ');
}
```

**Allow CSS custom properties in `style`** — create `src/react-css.d.ts`. Keep it a separate file: adding an `import` to `vite-env.d.ts` would turn it into a module and silently break its global `ImportMetaEnv` types.

```ts
import 'react';

declare module 'react' {
  interface CSSProperties {
    [key: `--${string}`]: string | number | undefined;
  }
}
```

## Core components

These six components cover what `fe-app-architecture` and `fe-auth-realtime` referenced. Each is complete, accessible and themed through tokens only.

### Button

A loading button stays focusable and keeps its label width. It uses `aria-disabled` instead of `disabled`, because a disabled button drops keyboard focus mid-action and screen readers lose their place.

**`ui/button/button.tsx`**

```tsx
import type { ComponentPropsWithRef, MouseEvent, ReactNode } from 'react';
import { cx } from '@/shared/lib/cx';
import styles from './button.module.css';

export interface ButtonProps extends ComponentPropsWithRef<'button'> {
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
  size?: 'sm' | 'md' | 'lg';
  loading?: boolean;
  icon?: ReactNode;
}

export function Button({
  variant = 'primary',
  size = 'md',
  loading = false,
  icon,
  type = 'button',
  className,
  children,
  onClick,
  ...rest
}: ButtonProps) {
  function handleClick(event: MouseEvent<HTMLButtonElement>) {
    if (loading) {
      event.preventDefault(); // ignore repeat clicks while working
      return;
    }
    onClick?.(event);
  }

  return (
    <button
      {...rest}
      type={type}
      className={cx(styles.button, className)}
      data-variant={variant}
      data-size={size}
      aria-busy={loading || undefined}
      aria-disabled={loading || undefined}
      onClick={handleClick}
    >
      {loading ? <span className={styles.spinner} aria-hidden="true" /> : icon}
      {children}
    </button>
  );
}
```

**`ui/button/button.module.css`** — private `--_` variables let each variant restyle the button by changing values, not rules:

```css
@layer components {
  .button {
    --_bg: var(--color-action-primary-bg);
    --_bg-hover: var(--color-action-primary-bg-hover);
    --_fg: var(--color-action-primary-fg);
    --_border: transparent;

    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-block-size: var(--control-height-md);
    padding-inline: var(--space-4);
    border: 1px solid var(--_border);
    border-radius: var(--radius-md);
    background: var(--_bg);
    color: var(--_fg);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);

    &:hover:not([aria-disabled='true'], :disabled) { background: var(--_bg-hover); }
    &:disabled, &[aria-disabled='true'] { opacity: 0.55; cursor: not-allowed; }

    &[data-variant='secondary'] {
      --_bg: transparent;
      --_bg-hover: var(--color-surface-hover);
      --_fg: var(--color-text);
      --_border: var(--color-border);
    }
    &[data-variant='ghost'] {
      --_bg: transparent;
      --_bg-hover: var(--color-surface-hover);
      --_fg: var(--color-text);
    }
    &[data-variant='danger'] {
      --_bg: var(--color-danger-bg);
      --_bg-hover: var(--color-danger-bg-hover);
      --_fg: var(--color-danger-fg);
    }
    &[data-size='sm'] { min-block-size: var(--control-height-sm); padding-inline: var(--space-3); }
    &[data-size='lg'] { min-block-size: var(--control-height-lg); padding-inline: var(--space-6); font-size: var(--font-size-md); }
  }

  .spinner {
    inline-size: 1em;
    block-size: 1em;
    border: 2px solid currentColor;
    border-inline-end-color: transparent;
    border-radius: var(--radius-full);
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { rotate: 1turn; }
  }
}
```

### Field and Input

`Field` owns the label, hint and error, and generates the ids that connect them. The render prop hands those ids to whatever control sits inside, so it works for `Input`, `select`, `textarea` or a custom control alike.

**`ui/field/field.tsx`**

```tsx
import { useId, type ReactNode } from 'react';
import styles from './field.module.css';

export interface FieldControlProps {
  id: string;
  'aria-describedby': string | undefined;
  'aria-invalid': boolean;
}

interface FieldProps {
  label: string;
  hint?: string | undefined;
  error?: string | undefined; // pass errors.x?.message straight from React Hook Form
  children: (control: FieldControlProps) => ReactNode;
}

export function Field({ label, hint, error, children }: FieldProps) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(' ') || undefined;

  return (
    <div className={styles.field} data-invalid={error ? true : undefined}>
      <label htmlFor={id} className={styles.label}>{label}</label>
      {hint && <p id={hintId} className={styles.hint}>{hint}</p>}
      {children({ id, 'aria-describedby': describedBy, 'aria-invalid': Boolean(error) })}
      {error && <p id={errorId} className={styles.error}>{error}</p>}
    </div>
  );
}
```

**`ui/field/input.tsx`**

```tsx
import type { ComponentPropsWithRef } from 'react';
import { cx } from '@/shared/lib/cx';
import styles from './field.module.css';

export function Input({ className, ...rest }: ComponentPropsWithRef<'input'>) {
  return <input {...rest} className={cx(styles.input, className)} />;
}
```

**`ui/field/field.module.css`**

```css
@layer components {
  .field { display: grid; gap: var(--space-1); }
  .label { font-size: var(--font-size-sm); font-weight: var(--font-weight-medium); }
  .hint { font-size: var(--font-size-sm); color: var(--color-text-muted); }
  .error { font-size: var(--font-size-sm); color: var(--color-danger-text); }
  .input {
    min-block-size: var(--control-height-md);
    padding-inline: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-bg);
    &[aria-invalid='true'] { border-color: var(--color-danger-text); }
  }
}
```

**Usage with React Hook Form** — this replaces the repeated markup in `fe-app-architecture`'s form:

```tsx
<Field label="Customer name" error={errors.customerName?.message}>
  {(control) => <Input {...control} autoComplete="name" {...register('customerName')} />}
</Field>
```

### Dialog and ConfirmDialog

Native `<dialog>` opened with `showModal()` provides a focus trap, Escape to close, an inert background, top-layer stacking (no z-index) and focus returning to the trigger on close. None of that has to be written by hand.

**`ui/dialog/dialog.tsx`**

```tsx
import { useEffect, useId, useRef, type ReactNode } from 'react';
import styles from './dialog.module.css';

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  footer?: ReactNode;
}

export function Dialog({ open, onClose, title, children, footer }: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  return (
    // Escape is handled natively; the click handler only closes on a backdrop click.
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-noninteractive-element-interactions
    <dialog
      ref={ref}
      className={styles.dialog}
      aria-labelledby={titleId}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose(); // the backdrop counts as the dialog itself
      }}
    >
      <div className={styles.panel}>
        <h2 id={titleId} className={styles.title}>{title}</h2>
        <div className={styles.body}>{children}</div>
        {footer && <div className={styles.footer}>{footer}</div>}
      </div>
    </dialog>
  );
}
```

**`ui/dialog/dialog.module.css`** — the inner panel carries the padding, so only a true backdrop click hits the `<dialog>` element:

```css
@layer components {
  .dialog {
    inline-size: min(100% - 2 * var(--space-4), 32rem);
    padding: 0;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-bg);
    color: var(--color-text);
    box-shadow: var(--shadow-lg);
    &::backdrop { background: var(--color-backdrop); }
  }
  .panel { display: grid; gap: var(--space-4); padding: var(--space-6); }
  .title { font-size: var(--font-size-xl); }
  .footer { display: flex; justify-content: flex-end; gap: var(--space-2); }
}
```

**`ui/dialog/confirm-dialog.tsx`** — for destructive actions such as cancelling an order:

```tsx
import { Button } from '../button/button';
import { Dialog } from './dialog';

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  pending?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({ open, title, description, confirmLabel, pending = false, onConfirm, onCancel }: ConfirmDialogProps) {
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      title={title}
      footer={
        <>
          {/* Focus starts on the safe choice, so a stray Enter never destroys anything. */}
          <Button variant="secondary" onClick={onCancel} autoFocus>Keep it</Button>
          <Button variant="danger" onClick={onConfirm} loading={pending}>{confirmLabel}</Button>
        </>
      }
    >
      <p>{description}</p>
    </Dialog>
  );
}
```

Add `'jsx-a11y/no-autofocus': ['error', { ignoreNonDOM: true }]` to the ESLint rules, so `autoFocus` on design-system components (used deliberately in dialogs) is allowed while raw DOM elements stay banned.

### Icon

One SVG sprite and one component. Icons inherit `currentColor` and scale with text. They are hidden from screen readers unless given a `label`.

**`ui/icon/icon.tsx`**

```tsx
import spriteUrl from './icons.svg'; // <svg><symbol id="check" viewBox="0 0 24 24">…</symbol>…</svg>

export type IconName = 'check' | 'close' | 'chevron-down' | 'search' | 'alert';

interface IconProps {
  name: IconName;
  label?: string; // set only when the icon carries meaning on its own
  size?: string;
}

export function Icon({ name, label, size = '1.25em' }: IconProps) {
  return (
    <svg width={size} height={size} fill="currentColor" aria-hidden={label ? undefined : true} role={label ? 'img' : undefined} aria-label={label}>
      <use href={`${spriteUrl}#${name}`} />
    </svg>
  );
}
```

### SafeHtml

The only place allowed to render HTML strings; the `fe-repo-foundation` lint rule bans `dangerouslySetInnerHTML` everywhere else.

**`ui/safe-html.tsx`**

```tsx
import DOMPurify from 'dompurify';

export function SafeHtml({ html, className }: { html: string; className?: string }) {
  const clean = DOMPurify.sanitize(html, { USE_PROFILES: { html: true } }); // strips scripts, handlers, javascript: URLs
  return <div className={className} dangerouslySetInnerHTML={{ __html: clean }} />;
}
```

**`ui/index.ts`**

```ts
export { Button, type ButtonProps } from './button/button';
export { Field, type FieldControlProps } from './field/field';
export { Input } from './field/input';
export { Dialog } from './dialog/dialog';
export { ConfirmDialog } from './dialog/confirm-dialog';
export { Icon, type IconName } from './icon/icon';
export { SafeHtml } from './safe-html';
```
