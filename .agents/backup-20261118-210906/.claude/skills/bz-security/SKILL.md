---
name: bz-security
description: Security standard and review checklist for the local-only, single-user Bazaar axum + React app on the owner's Mac - launch-token session, Host/Origin/CSRF/CSP template, macOS Keychain secrets, Kite token lifecycle, the one ripgrep red-flag scan, supply chain, SEBI checklist. Use when writing or reviewing security-sensitive code.
---

# Bazaar Security

Scope: what can hurt a local web app that holds broker credentials and (later) can place real orders. It runs from the repo on the owner's Mac, bound to `127.0.0.1:8787`; there is no VPS, Docker, remote access or login password. **Threat model:** the attacker is any web page open in the owner's browser (CSRF, DNS rebinding, cross-origin WebSocket hijacking), another local process or user, and leaks through logs, URLs, history or screenshots. Not in scope: internet attackers and brute-force login (nothing is reachable), malware running as the owner (it can read the Keychain prompt-free after one "Always Allow"; FileVault and a sane Mac are the control). Assets: Kite API secret and daily access token, capital, the audit log, the owner's regulatory standing.

Neighbours: `bz-core` (secret types, tracing, errors), `bz-web-api` (routes, DTOs, `references/api-contract.md`), `bz-web-ui` (client code), `bz-execution` (kill switch, rate limiter, audit log, `live-trading` feature), `bz-market-data` (Kite client, legality), `bz-trading-discipline` (gate, overrides), `bz-india-markets` (SEBI summary), `bz-testing` (fakes, MSW, Playwright), `bz-architecture` (ADRs), `bz-repo-structure` (`.gitignore`, justfile). Findings go to `.agents/reviews/<feature-id>/security.md` using `.agents/templates/security-findings.md`; at most 10 lines go back to the manager.

Deep dives: `references/web-auth-template.md` (the `crates/server/src/security.rs` code sketch), `references/secrets-and-kite-session.md` (`SecretStore`, Keychain, Kite daily login, Telegram, incident playbook), `references/sebi-algo-compliance.md` (the only SEBI summary and pre-live checklist; open before work on `broker/` or `ExecutionMode::Auto`). This skill owns the only auth/browser-security template (section 5) and the only full red-flag scan (section 3); other skills keep two or three unique patterns and link here. The contract is `CONTRACTS.md` section 4; this file implements it and must not diverge.

## 1. Default tools (as of 2026-11-18)

| Concern | Choice | Note |
|---|---|---|
| Server | `axum` 0.8 (0.8.9 current), features `ws`; `axum-extra` with `cookie` | middleware via `axum::middleware::from_fn_with_state` |
| Headers, limits | `tower-http` 0.7: features `set-header`, `timeout`, `trace`, `fs`; axum `DefaultBodyLimit` | no `cors` feature. tower-http 0.7 also ships a `csrf` feature: do not use it as the Origin check, it lets requests with neither `Origin` nor `Sec-Fetch-Site` through; ours rejects them |
| Randomness | `getrandom` 0.4 (`getrandom::fill`) | launch token, session ids, CSRF tokens, nonces; 32 bytes each |
| Constant-time compare | `subtle` 2.6 (`ConstantTimeEq`) | token and CSRF compares |
| Sessions | in-memory `HashMap` behind a `Mutex` in `crates/server` | restart logs out; no `tower-sessions`, `argon2`, `totp-rs` |
| Secret types | `secrecy::SecretString` everywhere + redacting `Debug` | `bz-core` |
| Secret store | `keyring-core` 1.0 + `apple-native-keyring-store` 1.0 (macOS Keychain) behind `SecretStore` | `.env` via `dotenvy` as dev fallback in bin entrypoints only |
| Rust supply chain | `cargo deny check` (advisories, licenses, bans, sources) | `just check` |
| JS supply chain | `pnpm -C web audit --prod`, lockfile committed, `--frozen-lockfile` in CI | |
| Secret scanning | `gitleaks` on every PR (`secrets-scan` CI job) | |
| Lint | `unsafe_code = "forbid"`; clippy `unwrap_used`, `expect_used`, `dbg_macro`, `print_stdout` | `bz-repo-structure` |

## 2. Golden rules

1. **Agents never read `.env`.** No Read, cat, grep, copy, diff or edit of `.env*` (except `.env.example`), `*.pem`, `*.key`, `secrets/`. Connection status comes only from `just db-check` (`bazaar-cli check-connections`: `OK <check>` or `FAILED <check> <category>`; never a secret). Human-only commands (the list in CONTRACTS section 5, incl. `set-secret` and `live-opt-in`) are never run by agents, who use fixtures, temp dirs and in-process `tower::ServiceExt::oneshot` tests.
2. **Secrets live in the Keychain, enter by the program.** The `SecretStore` trait lives in `bazaar_core::ports`; `KeychainStore` (macOS-only target dependency) and `EnvStore` live in `crates/store/src/secrets.rs`: Keychain first, `.env` (loaded by `dotenvy` in `bazaar-server` and `bazaar-cli` entrypoints only) as fallback. No secrets in `config/*.toml`, fixtures, snapshots, docs, commits or PR text.
3. **Redact by type.** Every credential is a `SecretString`; `Debug`, `Display`, `Serialize` never print it. Never log cookies, `Authorization`, CSRF tokens, the launch token, nonces, request or response bodies, or URLs with a query (log the path only).
4. **No broker secret ever reaches the browser, a DTO, `localStorage` or a URL.** The UI sees `{ csrf_token, mode, live_enabled }` and connection status. The browser holds one opaque `HttpOnly` session cookie and one CSRF token in memory.
5. **Loopback only, and refuse otherwise.** Bind `127.0.0.1` (port via `--port` only); no bind-address flag, env var or config key exists; startup aborts on any other address.
6. **The browser is the attacker, so every request is checked in this order:** Host allowlist, then Origin (non-GET and WebSocket upgrade), then session cookie, then CSRF header on non-GET. Exempt only `/healthz`, `/auth` (launch token) and `/api/v1/kite/callback` (nonce); the exempt list is a test constant.
7. **No CORS at all.** The UI is same-origin (the server serves `web/dist`; dev uses the Vite proxy). Any `CorsLayer` or `Access-Control-Allow-*` header is a high finding. GET handlers stay side-effect free, with one reviewed exception: `GET /api/v1/kite/login` stores a one-time nonce (session-protected, no cross-site cookie, harmless if repeated).
8. **One path to an order.** `Signal -> Recommendation -> OrderIntent -> DecisionGate -> Approval -> Executor`. No `skip_gate`, no auto-approve, `live-trading` never in `default`, `Auto` locked (`bz-execution`).
9. **Fail closed.** Kill switch persisted and checked before every send; unknown mode, missing config, expired token, stale quote, clock skew mean no order.
10. **The audit log is append-only and hash-chained**; no UPDATE or DELETE on it (`bz-execution`).
11. **Legal data only.** Official broker APIs, licensed vendors, exchange archives, user CSV. No scraping, undocumented endpoints, headless browsers, cookie-borrowing (`bz-market-data`).
12. **Every PR with security-relevant code has unit tests** for the failure path (bad Host, bad Origin, bad token, redaction, fail-closed), with fakes and temp dirs only (`bz-testing`).

## 3. Red-flag scan (run on changed files before review; every hit needs a reason or a fix)

```bash
# 1. Secrets leaking into logs, errors, URLs, messages (Rust)
rg -n -i '(api_secret|access_token|request_token|enc_token|bot_token|checksum|launch_token|csrf|nonce|session_id)' crates \
  | rg -i '(info!|warn!|error!|debug!|trace!|println!|eprintln!|dbg!|format!|panic!|expect\(|json!)'
# 2. Credential fields typed as plain String (should be SecretString)
rg -n -i 'pub(\(crate\))?\s+(api_secret|access_token|request_token|bot_token|launch_token|secret_key)\s*:\s*(String|&str)' crates
# 3. Reading secret files or env dumps; dotenvy or .env outside the bin entrypoints
rg -n '(\.env\b|std::env::vars\(\)|env::var_os|read_to_string\(.*(pem|key|secret))' crates web/src --glob '!**/secrets.rs' --glob '!**/*.md'
rg -n 'dotenvy' crates --glob '!crates/{server,cli}/src/main.rs'
# 4. Secrets persisted: tokens in SQLite, migrations, Parquet or config
rg -n -i '(access_token|api_secret|request_token|bot_token)' migrations crates/store config --glob '!**/*.md'
# 5. Order safety bypasses
rg -n -i '(skip_gate|bypass_gate|force_send|auto_approve|approve_all|ignore_kill|disable_kill|unchecked_order|NoGate)' crates web/src
rg -n 'default\s*=\s*\[[^]]*live-trading' --glob 'Cargo.toml'
rg -n -i '(place_order|/orders/(regular|amo|co|iceberg)|api\.kite\.trade)' crates --glob '!crates/broker/**' --glob '!crates/data/**' --glob '!**/fixtures/**'
rg -n '(fn place\(|Executor::)' crates --glob '!crates/{execution,broker}/**' --glob '!**/tests/**'
rg -n '(ApprovedOrder \{|GatedIntent \{)' crates --glob '!crates/{execution,risk}/**' --glob '!**/tests/**'
# 6. Audit-log mutation
rg -n -i '(UPDATE|DELETE\s+FROM|DROP\s+TABLE|TRUNCATE)\s+\w*audit' crates
# 7. TLS downgrades, native TLS, plain HTTP to brokers
rg -n '(danger_accept_invalid_certs|danger_accept_invalid_hostnames|native-tls|openssl|http://[a-z0-9.-]+\.(com|in|trade))' crates Cargo.toml web/src
# 8. Listening beyond loopback or a configurable bind (Vite host:true exposes the dev server to the LAN)
rg -n '(0\.0\.0\.0|\[::\]:\d|INADDR_ANY|Ipv[46]Addr::(UNSPECIFIED|new\(0))' crates web justfile --glob '!web/node_modules/**'
rg -n -i '(BAZAAR_BIND|bind_addr|--bind|--host\b|\bhost:\s*(true|["\x27]0\.0\.0\.0["\x27])|vite preview.*--host)' crates web/vite.config.ts web/package.json justfile
rg -n -i '(ngrok|cloudflared|tailscale (funnel|serve)|ssh -R|localtunnel)' justfile crates web/package.json
# 9. CORS or cross-origin opening (the app is same-origin; there is no ADR path for this)
rg -n '(CorsLayer|Access-Control-Allow|allow_origin|allow_credentials|allow_private_network|AllowOrigin)' crates web/src web/vite.config.ts
rg -n -i '(tower-sessions|argon2|totp-rs|"cors")' Cargo.toml crates/*/Cargo.toml
# 10. Cookie and session weakening
rg -n -i '(SameSite=(None|Lax)|SameSite::(None|Lax)|http_only\(false\)|\.domain\(|document\.cookie)' crates web/src
# 11. Guards skipped: all three must be wired once in the app builder; absence is the finding
rg -n 'from_fn(_with_state)?\(.*(host_guard|origin_guard|session_guard)' crates/server/src --glob '!**/tests/**'
rg -n -i '(skip|bypass|disable|no)_(host|origin|session|csrf|auth)|allow_any_(host|origin)' crates web/src
rg -n 'WebSocketUpgrade' crates/server/src | rg -v 'tests'      # each hit sits behind origin_guard + session_guard
rg -n 'Router::new\(\)|\.route_layer\(' crates/server/src --glob '!**/tests/**'   # routes added after route_layer escape it
rg -n 'ServeDir::new\(' crates | rg -v 'web/dist'                # only web/dist is served
# 12. Token in URLs and request logging (default TraceLayer spans log the full URI incl. query)
rg -n '(DefaultMakeSpan|include_headers\(true\)|TraceLayer::new_for_http\(\)\s*[,)])' crates
rg -n -i '(info|warn|error|debug|trace)!.*(uri|url|query|headers|cookie|set-cookie|authorization|body)' crates
rg -n -i '[?&](token|request_token|access_token|csrf)=' crates web/src --glob '!**/tests/**'
rg -n 'DefaultBodyLimit::disable|RequestBodyLimitLayer::new\(usize::MAX' crates
# 13. Weak CSP anywhere (code, meta tags)
rg -n -i "(unsafe-eval|script-src[^;]*unsafe-inline|default-src\s+\*|script-src[^;]*https?:|frame-ancestors\s+\*)" crates web --glob '!web/node_modules/**'
# 14. Browser injection, remote content, tokens in storage, hard-coded server URL
rg -n '(dangerouslySetInnerHTML|\.innerHTML\s*=|insertAdjacentHTML|\beval\(|new Function\(|document\.write|<script[^>]+src=["\x27]https?:|<link[^>]+href=["\x27]https?:|fetch\(["\x27`]https?:)' web/src web/index.html
rg -n -i '((local|session)Storage.*(token|secret|csrf|session|key)|https?://(127\.0\.0\.1|localhost):\d+)' web/src --glob '!**/mocks/**' --glob '!**/*.test.*'
# 15. Scraping and unofficial data sources
rg -n -i '(scraper::|Html::parse|chromiumoxide|fantoccini|thirtyfour|headless|cookie_store|nseindia\.com/api|bseindia\.com/BseIndiaAPI|moneycontrol|screener\.in)' crates web/src Cargo.toml
# 16. Panics and sloppy errors in security-sensitive crates
rg -n '(\.unwrap\(\)|\.expect\(|panic!|todo!|unimplemented!|unsafe\s*\{|#\[allow\()' crates/{core,data,execution,broker,risk,paper,server}/src --glob '!**/tests/**'
# 17. Advice-like or certainty wording in user-facing strings
rg -n -i '(guarantee|sure[- ]?shot|risk[- ]free|assured (profit|return)|can.t lose|multibagger|hot tip|you should (buy|sell)|we recommend you)' web/src crates --glob '!**/*.test.*' --glob '!**/fixtures/**'
# 18. Telegram or webhook tokens in URLs
rg -n -i '(api\.telegram\.org/bot|hooks\.slack\.com/services)' crates web/src
```

Patterns are deliberately noisy; tune them in `.agents/` if a hit class is always a false positive, never silence by deleting the scan. Also check the diff for new dependencies, new routes (each in the exempt-allowlist test or behind `session_guard`), new env vars, new `justfile` recipes that start a server, changed `risk.toml` defaults.

## 4. Secrets and the Kite session (detail: `references/secrets-and-kite-session.md`)

- `SecretStore` (get/set/delete, `SecretString` values) is backed by the macOS Keychain (`keyring-core` + `apple-native-keyring-store`, `set_default_store` once at startup in the composition roots). Fallback: environment variables loaded from an untracked `.env` by `dotenvy` in `bazaar-server`/`bazaar-cli` `main` only. Library crates never touch env directly. Tests use an in-memory store; CI and agents never touch the real Keychain.
- The daily Kite access token is stored via `SecretStore` (Keychain) with its IST issue date, cached in memory. Never in SQLite, Parquet, config, logs, crash reports, the browser, a DTO or a URL.
- Daily login is a browser redirect flow protected by a one-time nonce passed through Kite's `redirect_params` (the `SameSite=Strict` cookie is not sent on the cross-site return). Never log the callback query string.
- Tokens are valid for one trading day (reported to expire at 6 AM next day by the Kite docs, as of 2026-11-18; verify) and a new session with the same key invalidates the old one. `TokenException` or 403 means session over: clear the token, show "Login required", stop order paths and the Kite WebSocket without a reconnect loop. Never automate the login (no stored password or TOTP seed for Zerodha).
- Suspected leak: rotate at the broker (regenerate the API secret), delete Keychain items, restart the server (clears all sessions), purge git history, add a regression test or scan rule.

## 5. Browser-security template (code: `references/web-auth-template.md`)

All of it lives in `crates/server/src/security.rs`; this is the only copy in the pack. Any web page can send requests to `127.0.0.1`, and browsers do not apply CORS to WebSocket handshakes, so the server defends itself. Note that `SameSite` does not isolate ports: a page on another local port of the same host (a dev server, an XSS'd tool) is same-site, so the Origin and CSRF checks carry real weight.

- **Bind guard:** build `127.0.0.1:<port>`; a startup check refuses any other IP (also `::`, `0.0.0.0`, `::1`, LAN addresses). No bind setting anywhere.
- **Launch token:** 32 bytes from the OS RNG, hex. At startup print `http://127.0.0.1:<port>/auth?token=<t>` (debug builds: `http://localhost:5173/auth?token=<t>` for `just ui-dev`) once with a plain stdout write (never `tracing`) and open it with the default browser; failure to open is not fatal. `GET /auth` compares in constant time (`subtle`), consumes the token only on success (a wrong guess must not burn it), sets `bz_session` (random 32-byte id; `HttpOnly; SameSite=Strict; Path=/`; no `Domain`; no `Secure` because loopback HTTP is a secure context) and answers 303 to `/` so the token leaves the URL. Single use; a restart mints a new token and drops every session. Residual risk accepted: the URL is briefly visible in `ps` args and terminal scrollback; single use and a 10-minute TTL bound it.
- **Host allowlist (DNS rebinding):** only `127.0.0.1:<port>` and `localhost:<port>` (case-insensitive); anything else, or a missing Host, gets 403. Runs first on every route including static files. The Vite dev proxy uses `changeOrigin: true`, so it arrives as `127.0.0.1:<port>`; no extra debug Host entry.
- **Origin check:** on every non-GET request and every WebSocket upgrade, `Origin` must exactly equal `http://127.0.0.1:<port>` or `http://localhost:<port>` (debug builds also `http://localhost:5173`). Missing, `null`, or a prefix/suffix lookalike gets 403. Exact string match, no substring logic. An `OPTIONS` preflight is a non-GET, so it is refused too.
- **Session guard:** cookie `bz_session` must name a live in-memory session, else 401 problem+json. Non-GET also needs header `X-CSRF-Token` equal (constant time) to the session's CSRF token, else 403. The token comes from `GET /api/v1/auth/me` and lives in browser memory only. Content types stay strict (`Json` rejects `text/plain` form tricks).
- **Kite callback:** `/api/v1/kite/login` (session required) mints a 16-byte nonce with a 5-minute TTL, stores it, and redirects to Kite with `redirect_params` carrying `nonce=<n>` (URL-encoded query string; Kite appends it to the redirect URL, per the Kite login docs, checked 2026-11-18). `/api/v1/kite/callback` accepts only a known, unexpired, unused nonce; extra query keys from Kite are ignored (do not `deny_unknown_fields` here); exchange the `request_token` at once, store the access token via `SecretStore`, 303 to the UI. Redirect URL registered at Kite: `http://127.0.0.1:8787/api/v1/kite/callback`.
- **Headers:** CSP constant exactly as in CONTRACTS section 4 item 5 (`default-src 'self'; script-src 'self'; ...; object-src 'none'`), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` on every response; `Cache-Control: no-store` on `/api/*`. Nothing more is required; do not add CORS-adjacent headers.
- **Limits:** `DefaultBodyLimit` 1 MiB, `TimeoutLayer::with_status_code(408, 30 s)` (`TimeoutLayer::new` is deprecated). Max 4 concurrent WebSockets; page-size caps on list routes. An upload route needs its own cap and an ADR (imports are CLI commands by default).
- **WebSocket** `/api/v1/ws`: passes Host, Origin and session guards on the handshake; closes when its session disappears; server-to-client messages plus a typed subscribe; carries no order action.
- **Logging:** `TraceLayer` with a custom `make_span_with` recording method and `uri().path()` only. The default span logs the whole URI, which would put `/auth?token=` and the Kite callback query in logs. A tracing field-name redaction layer (`secret|token|password|authorization|cookie|csrf|nonce|api_key`) is defence in depth.
- **Chart and CSP, decided once:** start with `style-src 'self'`. lightweight-charts draws on canvas and sets styles through the DOM style API, which CSP does not block (unconfirmed from the library docs), so a test decides: the Playwright smoke (`bz-testing`) loads the built UI under the real CSP constant and fails on any `securitypolicyviolation` while the chart renders. If it fires, the owner records a `DECISIONS.md` line and CONTRACTS changes first; do not widen the policy in code alone. The dev server (Vite HMR) is outside the production policy, never merged into it.
- **Debug UI (`just ui-dev`):** settled in CONTRACTS section 7: debug builds print and open the launch link on `http://localhost:5173` (so the host-scoped cookie belongs to the page), the proxy rewrites Host, and only the Origin allowlist gains `http://localhost:5173`, in debug builds only.

## 6. API input validation

Every route is an API boundary exposed to code you do not fully control (npm dependencies, a future XSS, a hostile local page that passes the checks by luck).

- Request bodies are DTOs with `serde` `deny_unknown_fields`, converted by `TryFrom` into domain newtypes (`Symbol` resolved against the instrument master, `Qty` > 0 and a lot multiple for F&O, `Price` a tick multiple, `Decimal` not `f64`). Reject, never clamp silently.
- Routes accept no file paths, SQL, URLs, shell strings or mode flags (`execution_mode` is only set in `risk.toml`).
- Risk, money and limits are recomputed server-side from the store and `config/risk.toml`; a UI-supplied `risk_rupees` or `quantity` is a request validated by `DecisionGate`. `POST /api/v1/approvals/{id}/approve` re-loads the pending intent, re-runs the gate with fresh prices, checks expiry, kill switch, mode and the intent hash the user saw (`bz-execution`); a replayed or stale id fails.
- Errors are `application/problem+json` with a closed `code` and a safe message: no SQL, paths, tokens, stack traces, driver text. Detail goes to `tracing` after redaction. Paginate store reads.
- Never render broker or news text as HTML. SQL: `sqlx` bind parameters only; table and column names from enums. DuckDB: never interpolate user text into `read_parquet('...')` paths; build them from validated `Symbol` and year values. `ServeDir` serves `web/dist` only (never `data/` or the repo root).

## 7. Order-placement safety (live phase and its scaffolding)

Detail and tests live in `bz-execution` and `bz-trading-discipline`; the reviewer verifies:

- `broker` compiles only behind `live-trading`; default builds and tests never link a live executor; CI builds the feature but never calls live APIs.
- Mode changes (`paper -> semi_auto -> auto`) are explicit typed confirmations, audited, and impossible by config edit alone for `auto`.
- Kill switch: global, persisted in SQLite, checked in the executor just before every send, set and released only by `PUT /api/v1/kill-switch` from a logged-in session (never Telegram, CLI flag or env var). The daily-loss breaker trips the same switch. Hard ceiling rows in the `bz-trading-discipline` table.
- Rate limiter in front of the broker client, far below the broker limit (as of 2026-11-18 Zerodha reports 10 orders per second per client; verify in `sebi-algo-compliance.md`). Client order tag per intent; after a timeout reconcile via the order book, no blind retry. Reconciliation on start and reconnect freezes orders on mismatch.
- Audit rows hold ids, times, actor, mode, intent hash, verdict, override reason, broker order id, result, `prev_hash` and `hash`; no secrets or full broker payloads. Agents never set `ok = true` in `config/compliance.toml`.
- **Live-phase prerequisite (owner decision, not a kit feature):** SEBI's retail algo framework requires order APIs to be called from a whitelisted static IP (reported, as of 2026-11-18). A home connection needs a static IP from the ISP, or another owner decision, before phase 9. The owner records the choice, date and evidence in `DECISIONS.md`; never route orders through a third-party relay. Details, checklist item `static_ip` and the verification sources: `references/sebi-algo-compliance.md`. Paper and data phases are unaffected.

## 8. Local exposure and host hygiene

- **Never public.** No port forward, tunnel (ngrok, cloudflared, `tailscale funnel`), reverse proxy or `ssh -R` to 8787. The Vite dev server keeps its default localhost binding (`server.host` unset). A recipe that starts a server on another interface is a critical finding.
- **Other local users and processes** can open `127.0.0.1:8787`: they get 403 or 401 without the launch token or a session cookie. Do not put the token in a file, env var or CLI flag of any long-lived process.
- **Files:** `data/` and any `.env` are owner-only (`chmod 700 data`, `chmod 600 .env`); `.gitignore` from day one: `.env`, `.env.*` except `.env.example`, `data/`, `*.parquet`, `*.sqlite*`, `*.pem`, `*.key`, `target/`, `node_modules/`, broker UI screenshots. FileVault on; the Keychain holds the tokens, so Time Machine copies of `data/` hold no broker secret but do hold the audit log and positions.
- **Browser hygiene (owner):** use a normal profile with few extensions for the Bazaar tab; the token URL stays in history but is dead after first use. Chrome's Local Network Access prompts and similar browser features are extra, not relied on.
- **Healthcheck** `/healthz` returns no data and needs no session; status detail only behind a session.

## 9. Supply chain and build

- `deny.toml`: `[advisories]` deny vulnerabilities and unmaintained crates unless an ignore carries a reason and expiry; `[licenses]` allowlist; `[bans]` block `openssl` and `native-tls` (rustls only), duplicate majors reviewed; `[sources]` crates.io only, no git dependencies without an ADR. `cargo deny check` and `pnpm audit --prod` run in CI and `just check`.
- New dependency checklist: maintained recently, `unsafe` footprint, transitive count, licence, and for npm packages install scripts (pnpm `onlyBuiltDependencies` allowlist). Trading-adjacent crates (TA libraries, broker SDKs) are not trusted by default: the Kite client is ours over `reqwest`.
- Critical or high advisories are patched or mitigated within 48 hours; record in `DECISIONS.md`.
- Pin third-party GitHub Actions to a version or SHA; CI holds no broker secrets and no network access to broker APIs. Nothing is packaged or deployed, so there is no image to scan.

## 10. Disclaimer and no-advice language (compliance item)

The exact text and wording rules live only in `bz-india-markets` `references/compliance-and-disclaimer.md`. The reviewer checks that one shell component renders the shared constant on every route (test in `bz-web-ui`/`bz-testing`), that scan 17 is clean, that the app never claims SEBI registration, and that there is no feature to publish or share signals (`references/sebi-algo-compliance.md`). Low-evidence labelling (< ~30 trades) is part of this honesty duty.

## 11. Review procedure and severity

1. Read the plan and the diff (`git diff develop...<branch>`), not the whole repo. 2. Run the section 3 scan on changed paths; list each hit as fixed, justified or finding. 3. Walk sections 2 and 4 to 8 for the changed area. 4. Threat-model new routes, WebSocket messages, env vars or outbound calls in a few lines: which page or local process can trigger it, whether a GET changes state, what it touches, blast radius in rupees. 5. Each finding: id `SEC-<feature>-<n>`, severity, file:line, issue, impact (rupee or compliance), fix, verification; on re-check `fixed`, `not fixed` or `accepted risk (by tech lead)`. Anything weakening capital protection also goes to `risk-guardian` (veto).

| Severity | Meaning | Blocks merge |
|---|---|---|
| critical | Secret exposure, non-loopback reachability, data or order routes without session, order path without gate/approval/kill-switch, live feature on by default | Yes |
| high | Host/Origin/CSRF check missing or loosened, cookie flag weakened, CORS added, token in a URL log, scraping, unvalidated input reaching orders or SQL | Yes |
| medium | Defence-in-depth gap (noisy logs, missing header, missing test) | Tech lead decides |
| low / info | Hardening | No |

## 12. Tests (all with fakes, temp dirs and in-process requests)

Build the router with a test constructor (fixed launch token, `dev` flag, `FakeClock`), drive it with `tower::ServiceExt::oneshot`, and always set `Host` (oneshot adds none).

- HTTP security tests (bind, launch token, session, Host, Origin, CSRF, CORS absence, headers, limits, Kite callback nonce, no secret in responses): the single table in `bz-testing` section 5.6, run against every OpenAPI route. Keep them green; add new cases there.
- Secrets: `SecretString` redaction; `resolve` prefers the store over env; a token from day D is rejected on D+1 IST (`FakeClock`); a 403 from the fake clears the token and blocks order sends.
- Validation: table tests per DTO (bad symbol, zero qty, off-tick price, unknown field, oversize list) return the closed error. Gate bypass, kill switch and audit chain tests as in `bz-execution`.
- Playwright smoke (`bz-testing`): chart screen renders with the production CSP and zero `securitypolicyviolation` events.

## 13. Checklist (done means)

- [ ] Red-flag scan run; no unexplained hits; no `CorsLayer`, no `0.0.0.0`, no bind setting.
- [ ] No secret in code, config, fixtures, logs, SQLite or UI payloads; `.env` untouched by agents; `just db-check` used for connectivity.
- [ ] Every new route is behind `session_guard` (or in the reviewed exempt list); non-GET needs Origin and CSRF; WebSocket checks Host, Origin and session; GET handlers have no side effects.
- [ ] Cookie flags, headers, CSP, body limit and timeout verified by tests; no URL query in logs; chart CSP decision recorded.
- [ ] Routes validate input into domain types; errors are the closed safe problem type.
- [ ] Order path: gate, approval re-check, kill switch, rate limit, audit append; `live-trading` not default.
- [ ] `cargo deny check` and `pnpm audit --prod` clean or justified; no git dependencies.
- [ ] Disclaimer rendered and tested; copy free of advice claims; legal-data rule respected.
- [ ] Unit tests for failure paths in the PR; findings written with ids if any.
- [ ] For live-phase work: `references/sebi-algo-compliance.md` walked, static-IP owner decision in `DECISIONS.md`, current circulars re-verified.
- [ ] No human-only command run by an agent; evidence came from fixtures and temp dirs.
