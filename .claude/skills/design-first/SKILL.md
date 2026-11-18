---
name: design-first
description: Show-before-you-build for look-and-feel work on the front end - turning vague taste words (modern, premium, smooth, interactive, redesign) into up to three clickable demo pages the owner picks from, building the demos fast with real brand tokens and assets, and turning the pick into owner rules and acceptance criteria. Use in the preview-first lane before any slice is planned.
---

# Design first

A non-designer owner can't choose between "scroll storytelling" and "subtle and refined" in words, and a wrong guess
built into the real app costs ~5-10x a demo. So: show, then ask. Text questions must be answerable by pointing.

## 1. One question, then demos
- Ask at most one question: "Any site or app you like the feel of?" Skip it if the owner already said.
- Never ask the owner to pick between style or motion adjectives.

## 2. The demo menu
- Up to **3 directions that differ in kind** (e.g. calm and editorial / bold product-led / motion-rich), not three
  shades of one idea. If the owner named a reference, one direction follows it.
- `previews/<id>/index.html` is the menu: one card per direction, one line each, links to `a.html`, `b.html`, `c.html`.
- Each demo is **one standalone HTML file**: plain HTML, CSS and a little JS; opens with a double-click; no build, no
  framework, no package installs, no network calls except fonts/assets the site already uses.
- Use the **real** brand: colours, fonts and spacing from the design tokens (`fe-design-system`; copy the values, don't
  import the build), the real logo and product images from the repo's assets (relative paths), and realistic copy.
- Show the part being changed in its real context (e.g. the whole homepage above the fold plus one section), at
  desktop and phone widths (responsive CSS, no separate files).
- Motion: include a visible **"Reduce motion"** toggle and respect `prefers-reduced-motion`. Effects must start
  immediately, not after long delays. No scroll-jacking unless that is the direction being shown.
- Accessibility basics still apply (contrast, focus visible, real buttons and links).

## 3. Check, then show
- One screenshot per demo at 1440 and 360 wide (`pnpm exec playwright screenshot ...` if available, otherwise open and
  describe). Fix only what is broken.
- The manager shows the menu and asks: "Which do you keep, which do you drop, more or less of what?"

## 4. Turn the pick into rules
- Write what the owner said into `.agents/owner-rules.md` as short, testable lines ("sections fit the viewport on
  desktop", "no pinned scroll", "hover effects start immediately").
- Write acceptance criteria the owner can check by looking ("Open /, the hero fills the screen at 1440x900 without
  scrolling"). These feed the plan; the demo itself is thrown away, never merged.
