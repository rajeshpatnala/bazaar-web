---
name: fe-a11y-i18n
description: "Makes the front end accessible (WCAG 2.2 AA) and translatable: legal obligations, accessible widget patterns (announcer, tabs, focus management), visual requirements (contrast, reduced motion, forced colours), automated and manual a11y testing with axe, react-intl/FormatJS messages with ICU plurals, extraction/compilation/pseudo-locales, locale negotiation and lazy message loading, formatting, RTL layouts and the translation workflow. Use whenever building interactive widgets, adding user-facing text, dates, numbers or currency, handling focus, supporting a new locale or RTL, or reviewing accessibility."
---

# Accessibility & internationalization

Every user-facing string becomes a message and every widget meets WCAG 2.2 AA. Load `fe-standards` alongside it.

## Best practices

Binding rules; the reference files show how to apply them.

**Accessibility**

1. **Target WCAG 2.2 level AA for everything you ship** — a clear, testable bar that matches most legal requirements.
2. **Native HTML first, ARIA last** — a real `<button>` or `<dialog>` brings keyboard support and screen-reader semantics that custom ARIA widgets often get wrong.
3. **Everything works with a keyboard alone, with a visible focus indicator** — many users, including power users, never touch a mouse.
4. **Announce changes that happen without a page load** — screen-reader users otherwise miss "Saved", new results and errors entirely.
5. **Manage focus deliberately** — on route change, when dialogs open and close, and after deleting the focused item.
6. **Never convey meaning by colour alone; meet contrast ratios in the tokens** — every component inherits accessible colours.
7. **Respect user settings** — reduced motion, forced colours (high contrast), zoom to 200–400%, and larger text.
8. **Automate what you can, and test the rest by hand** — automated tools catch only part of the problems; a short keyboard and screen-reader pass on every new flow catches the rest.

**Internationalization**

9. **No user-facing string lives in code; every string is a message with an id and a description** — translators need context, and engineers need to find strings.
10. **Use ICU message syntax for plurals, genders and selections** — languages have up to six plural forms, so `count === 1 ? 'item' : 'items'` is wrong in most of them.
11. **Never build sentences by concatenation** — word order differs between languages; pass values into one message instead.
12. **Format dates, numbers, currencies and lists with `Intl`, always with an explicit locale** — the browser already knows the rules for every locale.
13. **Store times in UTC and display them in the user's time zone** — "yesterday at 11 pm" depends on where the user is.
14. **Lay out with logical CSS properties and set `dir`** — right-to-left languages then work without a separate stylesheet.
15. **Load only the active locale's messages** — translations are code-split like routes.
16. **Test with pseudo-localization before real translations exist** — longer, accented strings reveal truncation and hard-coded text early.

## Workflow

1. Know the obligations and what WCAG 2.2 added → `references/a11y.md` (Standards).
2. Use native elements first; build announcer, tabs and focus management per the pattern table → `a11y.md` (Accessible patterns).
3. Visual requirements and CSS (scroll padding, reduced motion, forced colours) → `a11y.md` (Visual accessibility).
4. Automated a11y layers, `e2e/a11y.spec.ts`, manual test script → `a11y.md` (Testing accessibility).
5. Set up react-intl, lint rules, extract/compile/pseudo scripts, `i18n.ts`, `IntlProvider` startup step → `references/i18n.md`.
6. RTL mirroring and the translation workflow → `i18n.md`.
7. Run the PR checklist → `references/checklist.md`.

## Files this playbook owns (Build mode)

`src/shared/ui/announcer.tsx`, `src/shared/ui/tabs.tsx`, `src/shared/i18n/**`, `lang/**`, `e2e/a11y.spec.ts`, i18n scripts in `package.json`, formatjs ESLint rules (as a change request to `fe-repo-foundation`), the locale step in `main.tsx` (with `fe-cicd-observability`).

## Done when

axe reports zero WCAG A/AA violations on every route and open dialog; the manual keyboard and screen-reader script passes; no hard-coded user-facing strings (formatjs lint passes); `en-XA` pseudo-locale renders without truncation and `en-XB` mirrors correctly; locale messages load lazily and `<html lang dir>` updates.

## Reference files

Read only the file the current step needs. Paths are relative to this skill's folder (`.claude/skills/<skill>/`).

- `references/a11y.md` — Obligations, WCAG 2.2 additions, widget patterns with code, visual requirements and a11y testing.
- `references/i18n.md` — Translation pipeline, react-intl setup and rules, scripts, locale loading, formatting, RTL and translation workflow.
- `references/checklist.md` — Checklist to run on every PR that changes UI or text.

## Sources

- Legal summaries are general information as of October 2026; confirm specifics with counsel.
