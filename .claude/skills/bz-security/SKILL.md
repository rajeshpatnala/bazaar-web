---
name: bz-security
description: Security standard and review checklist for the local-only, single-user Bazaar axum + React app on the owner's Mac - launch-token session, Host/Origin/CSRF/CSP template, macOS Keychain secrets, Kite token lifecycle, the one ripgrep red-flag scan, supply chain, SEBI checklist. Use when writing or reviewing security-sensitive code.
---

# Bazaar Security

Scope: what can hurt a local web app that holds broker credentials and (later) can place real orders. It runs from the repo on the owner's Mac, bound to `127.0.0.1:8787`; there is no VPS, Docker, remote access or login password. **Threat model:** the attacker is any web page open in the owner's browser (CSRF, DNS rebinding, cross-origin WebSocket hijacking), another local process or user, and leaks through logs, URLs, history or screenshots. Not in scope: internet attackers and brute-force login (nothing is reachable), malware running as the owner (it can read the Keychain prompt-free after one "Always Allow"; FileVault and a sane Mac are the control). Assets: Kite API secret and daily access token, capital, the audit log, the owner's regulatory standing.

Neighbours: `bz-core` (secret types, tracing, errors), `bz-web-api` (routes, DTOs, `references/api-contract.md`), `bz-web-ui` (client code), `bz-execution` (kill switch, rate limiter, audit log, `live-trading` feature), `bz-market-data` (Kite client, legality), `bz-trading-discipline` (gate, overrides), `bz-india-markets` (SEBI summary), `bz-testing` (fakes, MSW, Playwright), `bz-architecture` (ADRs), `bz-repo-structure` (`.gitignore`, justfile). Findings go to `.agents/reviews/<feature-id>/security.md` using `.agents/templates/security-findings.md`; at most 10 lines go back to the manager.

Deep dives: `references/web-auth-template.md` (the `crates/server/src/security.rs` code sketch), `references/secrets-and-kite-session.md` (`SecretStore`, Keychain, Kite daily login, Telegram, incident playbook), `references/sebi-algo-compliance.md` (the only SEBI summary and pre-live checklist; open before work on `broker/` or `ExecutionMode::Auto`). This skill owns the only auth/browser-security template (section 5) and the only full red-flag scan (section 3); other skills keep two or three unique patterns and link here. The contract is `CONTRACTS.md` section 4; this file implements it and must not diverge.

## 1. Default tools (as of 2026-11-18)

In short: 
Full detail: `references/default-tools.md` - read it before changing or reviewing this area.

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

In short: All of it lives in `crates/server/src/security.rs`; this is the only copy in the pack. Any web page can send requests to `127.0.0.1`, and browsers do not apply CORS to WebSocket handshakes, so the server defends itself. Note that ...
Full detail, examples and patterns: `references/browser-security-template.md` - read it before changing this area.

## 6. API input validation

In short: Every route is an API boundary exposed to code you do not fully control (npm dependencies, a future XSS, a hostile local page that passes the checks by luck).
Full detail: `references/api-input-validation.md` - read it before changing or reviewing this area.

## 7. Order-placement safety (live phase and its scaffolding)

In short: Detail and tests live in `bz-execution` and `bz-trading-discipline`; the reviewer verifies:
Full detail: `references/order-placement-safety.md` - read it before changing or reviewing this area.

## 8. Local exposure and host hygiene

In short: **Never public.** No port forward, tunnel (ngrok, cloudflared, `tailscale funnel`), reverse proxy or `ssh -R` to 8787. The Vite dev server keeps its default localhost binding (`server.host` unset). A recipe that starts a server on another ...
Full detail: `references/local-exposure-and-host-hygiene.md` - read it before changing or reviewing this area.

## 9. Supply chain and build

In short: `deny.toml`: `[advisories]` deny vulnerabilities and unmaintained crates unless an ignore carries a reason and expiry; `[licenses]` allowlist; `[bans]` block `openssl` and `native-tls` (rustls only), duplicate majors reviewed; `[sources]` ...
Full detail: `references/supply-chain-and-build.md` - read it before changing or reviewing this area.

## 10. Disclaimer and no-advice language (compliance item)

The exact text and wording rules live only in `bz-india-markets` `references/compliance-and-disclaimer.md`. The reviewer checks that one shell component renders the shared constant on every route (test in `bz-web-ui`/`bz-testing`), that scan 17 is clean, that the app never claims SEBI registration, and that there is no feature to publish or share signals (`references/sebi-algo-compliance.md`). Low-evidence labelling (< ~30 trades) is part of this honesty duty.

## 11. Review procedure and severity

1. Read the plan and the diff (`git diff develop...<branch>`), not the whole repo. 2. Run the section 3 scan on changed paths; list each hit as fixed, justified or finding. 3. Walk sections 2 and 4 to 8 for the changed area. 4. Threat-model new routes, WebSocket messages, env vars or outbound calls in a few lines: which page or local process can trigger it, whether a GET changes state, what it touches, blast radius in rupees. 5. Each finding: id `SEC-<pr>-<n>`, severity, file:line, issue, impact (rupee or compliance), fix, verification; on re-check `fixed`, `not fixed` or `accepted risk (by tech lead)`. Anything weakening capital protection also goes to `risk-guardian` (veto).

| Severity | Meaning | Blocks merge |
|---|---|---|
| critical | Secret exposure, non-loopback reachability, data or order routes without session, order path without gate/approval/kill-switch, live feature on by default | Yes |
| high | Host/Origin/CSRF check missing or loosened, cookie flag weakened, CORS added, token in a URL log, scraping, unvalidated input reaching orders or SQL | Yes |
| medium | Defence-in-depth gap (noisy logs, missing header, missing test) | Tech lead decides |
| low / info | Hardening | No |

## 12. Tests (all with fakes, temp dirs and in-process requests)

In short: Build the router with a test constructor (fixed launch token, `dev` flag, `FakeClock`), drive it with `tower::ServiceExt::oneshot`, and always set `Host` (oneshot adds none).
Full detail: `references/tests.md` - read it before changing or reviewing this area.

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
