# Security lens

Goal: stop attackers injecting script/markup, tampering with data, stealing sessions, or abusing third parties. Defense in depth: assume any single layer fails.

## 1. Find injection sinks (grep first)
```
innerHTML|outerHTML|insertAdjacentHTML|document\.write|\.html\(|dangerouslySetInnerHTML|v-html|\[innerHTML\]|bypassSecurityTrust
eval\(|new Function|setTimeout\(["'`]|setInterval\(["'`]|srcdoc|javascript:|on\w+=
location\s*=|location\.href\s*=|location\.(assign|replace)|window\.open
postMessage|addEventListener\(["']message
```
For each hit, trace the data source. Dangerous sources: `location.*`, `document.referrer`, `URLSearchParams`, `localStorage`/`sessionStorage`/cookies, API responses, `postMessage`, anything user-typed or editor-entered (product names, reviews, addresses).
- Fix: `textContent`/`createElement`/framework default escaping; build DOM, don't concatenate HTML strings. If HTML is unavoidable, sanitize with DOMPurify (pinned, allow-list) and Trusted Types.
- Template literals that put data into `innerHTML` are the most common hole on "vanilla" sites (cart rows, search results, toasts).
- URLs from data: allow-list schemes (`https:`, `mailto:`, `tel:`), reject `javascript:`/`data:`. Add `rel="noopener noreferrer"` on `target=_blank`.

## 2. Content-Security-Policy (the strongest single control)
- Prefer an HTTP header from the host; `<meta http-equiv>` CSP works but cannot do `frame-ancestors`, `report-uri`, or sandbox.
- Target policy: `default-src 'self'; script-src 'self' [nonce/hash]; style-src 'self'; img-src 'self' data: https:; connect-src 'self' <api>; frame-src <payment iframe only>; object-src 'none'; base-uri 'none'; form-action 'self' <gateway>; frame-ancestors 'none'; upgrade-insecure-requests`
- Remove inline `<script>`, inline `on*=` handlers and inline `style=` where feasible (move to files) so `'unsafe-inline'` is not needed. Use nonces/hashes for unavoidable inline.
- Roll out with `Content-Security-Policy-Report-Only` first, then enforce. Add `require-trusted-types-for 'script'` where the stack allows.

## 3. Headers (host config: `_headers`, `vercel.json`, nginx, Cloudflare)
`Strict-Transport-Security` (preload after verifying), `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`, `Permissions-Policy` (deny camera/mic/geolocation/payment unless needed), `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy`, frame protection via CSP `frame-ancestors` (clickjacking). Disable directory listing; don't serve `.git`, `.env`, `backup/`, `plans/`, source maps with secrets, or `tools/` dev servers in production (check the deploy output!).

## 4. Secrets and trust
- grep for `api[_-]?key|secret|token|password|Bearer |sk_live|sk_test|AKIA|-----BEGIN`, and in git history. Public keys (Stripe `pk_`, Razorpay key id) are fine; secret keys never.
- **Client-side values are not trusted**: price, discount, shipping, stock, "paid" status, role/admin flags. The server recomputes totals from SKU+qty and verifies payment by webhook/signature.
- No auth decisions in JS (hidden buttons are not access control).

## 5. Storage, sessions, CSRF
- Session/auth tokens: `HttpOnly; Secure; SameSite=Lax|Strict` cookies, never `localStorage` (XSS-readable). Cart ids and non-sensitive prefs may live in storage; never PII, card data, or tokens. Validate/parse stored JSON defensively (treat as untrusted, it can be edited).
- State-changing requests: CSRF token or SameSite + custom header + origin check. No state change on GET.
- CORS: no `*` with credentials; allow-list origins.
- Login/forms: rate limiting and bot protection server-side; generic error messages; `autocomplete` attributes correct; no sensitive data in URLs.

## 6. Supply chain and third parties
- Every external `<script>`/`<link>`/font/CDN: list it. Pin exact versions, add `integrity` (SRI) + `crossorigin`, or self-host. Tag managers, chat widgets, and analytics have full page access: minimize, sandbox in iframes where possible, load after consent.
- `npm audit`/`osv-scanner`, lockfile committed, `npm ci` in CI, no `postinstall` surprises, remove unused deps, Dependabot/Renovate. Beware typosquats.
- Payment pages: no third-party scripts at all besides the gateway (Magecart defense). Gateway fields in the gateway's iframe/hosted page so card data never touches your DOM.

## 7. Forms, uploads, redirects, misc
- Client validation is UX only; server validates everything. Escape on output, not on input.
- Open redirects: `?next=`/`?return=` must be allow-listed or same-origin relative paths.
- `postMessage`: always check `event.origin` and shape; never `'*'` with sensitive data.
- Service workers: scope tightly, don't cache authenticated/API responses blindly.
- Errors/logs: no stack traces, tokens or PII to console or analytics in production.
- Prototype pollution: avoid recursive merge of untrusted JSON; use `Object.create(null)`/`Map`, block `__proto__`/`constructor`.
- Static-site specifics: forms posting to a third party (Formspree etc.) need spam protection and a CSP `form-action`; mailto/tel injection via data.

## 8. Regression guards (leave these behind)
A grep-based CI check banning `innerHTML` with non-literal args, `eval`, `document.write`; CSP in report-only → enforced; `npm audit` in CI; a `SECURITY.md` and `/.well-known/security.txt`.
