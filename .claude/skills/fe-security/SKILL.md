---
name: fe-security
description: Front-end security standard and review procedure - injection sinks (innerHTML, eval, javascript URLs, postMessage), Content-Security-Policy and headers, secrets in bundles and public env prefixes, token storage, CSRF, supply chain and third-party scripts, payments and commerce trust boundaries, severity and confidence. Use when writing or reviewing security-sensitive front-end code, auth, checkout, headers or dependencies.
---

# Front-end security

The procedure, severity scale and findings format come from `security-review`; this skill adds what is specific to browsers and front-end stacks. Auth sessions, CSRF headers and real-time channels are built per `fe-auth-realtime`, whose `references/testing-and-security.md` holds the full auth checklist.

## 1. Ground rules

- **The browser is public.** Prices, discounts, stock, permissions, auth and payment status are decided on the server. Any client-side check is UX only.
- **Secrets never reach the bundle.** Anything behind a public env prefix (`VITE_`, `NEXT_PUBLIC_`, `PUBLIC_`) ships to every visitor: only public values (API base URL, public keys) belong there. Server keys live in the backend or CI secrets.
- **Agents never read `.env*`** (except `.env.example`), key files or `secrets/`; hooks block it. Check for tracked secrets files with `git ls-files '.env*' '*.pem' '*.key'` and report the location, never the value.
- **Defense in depth:** escaping by default, a strict CSP, sanitizing at the one place HTML is allowed (`SafeHtml` in `fe-design-system`), and server-side validation. Promise reduced risk, never "unhackable".

## 2. Red flags (run on the diff or the files in scope)

```
innerHTML|outerHTML|insertAdjacentHTML|document\.write|dangerouslySetInnerHTML|v-html|bypassSecurityTrust
eval\(|new Function|setTimeout\(["'`]|setInterval\(["'`]|srcdoc|javascript:
location\s*=|location\.href\s*=|location\.(assign|replace)|window\.open
postMessage|addEventListener\(["']message
localStorage|sessionStorage|document\.cookie
(VITE|NEXT_PUBLIC|PUBLIC)_[A-Z_]*(SECRET|PRIVATE|TOKEN|PASSWORD|KEY)
sk_live_|rk_live_|rzp_live_|shpat_|AKIA|-----BEGIN
```

For each hit, trace the data source to the sink. Dangerous sources: `location.*`, `document.referrer`, `URLSearchParams`, web storage, cookies, API responses, `postMessage`, and anything a user or editor typed.

## 3. Where the depth is

| Topic | Read |
|---|---|
| Sinks and fixes, CSP, headers, secrets and trust, storage/sessions/CSRF, supply chain, forms and redirects, regression guards | `references/security.md` |
| Cart, checkout and payments: trust boundaries, provider choice, seams, red flags | `references/commerce.md` |
| Cookie sessions, CSRF, refresh, guards, multi-tab, WebSocket/SSE security checklist | `fe-auth-realtime` -> `references/testing-and-security.md` |
| Third-party scripts and CSP origins | `fe-performance` -> `references/third-party-and-enforcement.md` |

## 4. Severity (front end)

Severity = business impact x ease of exploit; note prerequisites (anonymous visitor, logged-in customer, admin, victim must click, network position).

- **Critical:** low-effort script execution in a victim's browser, secret or credential exposure, payment or price tampering, auth bypass, customer data exposure.
- **High:** stored or reflected XSS sinks, missing CSRF on state changes, client-trusted prices or discounts, unpinned third-party script with page access.
- **Medium:** missing CSP or security headers, tokens in web storage, no SRI, open redirects, verbose errors.
- **Low:** hygiene and hardening without a demonstrated exploit.

Anything touching money, checkout or customer accounts ranks one level higher. Give each finding a **confidence** too: `Confirmed` (reproduced), `High` (complete code path), `Suspected` (depends on server or deployed config you can't see - say what the server must do), `Info`. A header missing from repo config doesn't prove the live site lacks it: say which layer you checked.

## 5. Done when

No new sink without sanitizing and a test; no secret or private key behind a public prefix; cookie-authenticated writes send the CSRF header; third-party scripts are pinned with SRI and listed in the CSP; payment success is decided by the server; every finding has `file:line`, a scenario, severity and confidence.
