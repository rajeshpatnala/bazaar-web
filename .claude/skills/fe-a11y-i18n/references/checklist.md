# Accessibility and i18n PR checklist

Checklist to run on every PR that changes UI or text.

## Checklist for PRs

**Accessibility**

- [ ] Native elements used (`button`, `a`, `dialog`, `table`, `label`); ARIA only where no native element exists.
- [ ] Every control has an accessible name; icon-only buttons have `aria-label`.
- [ ] Keyboard-only pass done: everything reachable, focus visible, logical order, no traps.
- [ ] Focus is moved deliberately after route changes, dialogs, deletions and failed submits.
- [ ] Async results and errors are announced (announcer or `role="alert"`).
- [ ] Contrast checked in light and dark themes; meaning never carried by colour alone.
- [ ] Works at 200% zoom and 320 px width; respects reduced motion and forced colours.
- [ ] Storybook and Playwright axe checks pass with zero violations.

**Internationalization**

- [ ] No hard-coded user-facing strings; every message has an id, a default and a description.
- [ ] Plurals and variable sentences use ICU syntax; no concatenated sentences.
- [ ] Dates, numbers, money and lists formatted through `intl` with the user's locale and time zone.
- [ ] Layout checked in `en-XA` (long text) and `en-XB` (right-to-left); logical CSS properties only.
- [ ] `lang/en.json` re-extracted and committed.
