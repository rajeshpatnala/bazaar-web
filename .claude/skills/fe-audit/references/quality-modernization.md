# Bugs, quality and legacy-modernization lens

## 1. Bug hunting (grep + read the risky spots)
- **JS correctness:** unguarded `querySelector` results (null deref), missing `await`/unhandled promise rejections, `==` coercion, mutated shared state, stale closures, listeners added repeatedly (leaks), `parseInt` without radix, float money math, timezone/date parsing, `JSON.parse` of storage without try/catch, race conditions between async responses (use AbortController / request ids).
- **DOM/CSS:** duplicate ids, broken label/for links, `100vh` on mobile (use `svh/dvh`), z-index wars, fixed heights clipping content, horizontal overflow at 360px, hover-only interactions, missing `:focus-visible`, layout shift from images without dimensions.
- **Flows:** walk every user path (browse, filter, add, edit, remove, checkout, error, back button, refresh mid-flow, empty state, offline). Test with disabled JS for content pages, and with a stale/corrupt `localStorage`.
- **Links/assets:** broken links, 404 assets, case-sensitive path mismatches (works on macOS, breaks on Linux hosts), mixed content, wrong MIME, missing `charset`/viewport meta.

## 2. Accessibility (WCAG 2.2 AA minimum)
Semantic landmarks and headings order; every interactive element is a real `button`/`a`; visible focus; full keyboard operation incl. menus/dialogs (focus trap and return, Esc); `alt` text; labels for inputs, error association (`aria-describedby`, `aria-live` for toasts/cart updates); contrast ≥ 4.5:1 text, 3:1 UI; touch targets ≥ 44px (WCAG min 24px); `prefers-reduced-motion` and `prefers-color-scheme` respected; no color-only meaning; zoom to 200%/400% reflow works. Prefer native `<dialog>`, `<details>`, `popover`.

## 3. SEO and sharing
One `<h1>`, unique `<title>`/meta description, canonical, Open Graph/Twitter cards, `sitemap.xml`, `robots.txt`, structured data (`Product`, `Offer`, `Organization`, `BreadcrumbList`), clean URLs, `lang` attribute, hreflang if multilingual, 404 page, no content that exists only after JS if indexing matters.

## 4. Browser support and progressive enhancement
Decide a support matrix (e.g. last 2 versions + Safari 15.4+). Use `@supports` fallbacks (`backdrop-filter`, `:has`, container queries, `dvh`). Core content and navigation should work before JS loads. Feature-detect, never UA-sniff.

## 5. Modernizing legacy UI (jQuery/table layouts/Bootstrap 2-3/Flash-era)
Order of operations (each step shippable):
1. **Stabilize:** version control, baseline screenshots, smoke tests, fix security criticals first (see `security.md`).
2. **Foundations:** HTML5 doctype/charset/viewport, semantic markup, replace table/float layouts with grid/flex, `rem`-based type scale, tokens, system or self-hosted fonts.
3. **Componentize** repeated UI behind shared CSS/partials; delete dead CSS (coverage tool) and dead JS.
4. **Replace libraries by platform:** jQuery → `querySelector`/`fetch`/`classList`; `$.ajax` → `fetch` + API client; carousels/modals → native or small accessible implementations; old icon fonts → inline SVG sprite.
5. **Responsive + mobile-first** from 360px; fluid `clamp()` spacing; layout by structure (grid/flex that cannot collide), never per-viewport hand-tuned numbers.
6. **Visual refresh** last, via tokens and an override layer, so rebranding stays cheap. Modern look = restraint: consistent spacing scale, 2 typefaces max, real hierarchy, motion with purpose and reduced-motion support, glass/blur only with `@supports` fallback and sufficient contrast.
7. **Tooling:** lint/format/CI, then bundling only if needed (Vite/esbuild), image pipeline to AVIF/WebP, hashed assets.
Do not rewrite in a framework unless there is real app state complexity; a content/commerce site is usually best as semantic HTML + small modules or a static generator.

## 6. Quality gates before "done"
- Lighthouse (perf/a11y/best-practices/SEO) numbers recorded before/after; axe scan; keyboard-only pass; 360/768/1280px screenshots of every affected page; console clean (no errors/warnings); network tab shows no failed/unexpected requests; `npm audit` clean or triaged.
- Report exactly what was and was not tested.
