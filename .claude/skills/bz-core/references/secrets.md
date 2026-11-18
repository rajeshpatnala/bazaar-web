# 6. Secrets

(From `bz-core` SKILL.md; open when the task touches this area.)


- **Agents never open `.env`, key files, credential files, secret mounts or token caches.** To learn whether configuration and connections work, run `just db-check` (prints `OK <check>` / `FAILED <check> <category>` for `sqlite`, `duckdb`, `kite-session`, `instruments`, `audit-chain`; no secrets). Add new keys to `.env.example` as a name with an empty value and document them in the README; there is no mode variable.
- Resolution order inside the program: macOS Keychain (`keyring-core` store behind `SecretStore`) first, then environment variables loaded from a local untracked `.env` by `dotenvy` in the bin entrypoints only (never in libraries or tests). Everything sits behind the `SecretStore` trait, so code asks for a named secret and does not care where it came from; tests use an in-memory fake store.
- Typed `Secrets` struct of `secrecy::SecretString`; expose with `expose_secret()` at the single call site that builds a request header, and nowhere else.
- Kite `api_secret`, `request_token`, `access_token`, the launch token, session cookies and any Telegram bot token: never in logs, `Debug`, error text, URLs, API responses, WebSocket messages, panic messages, snapshots or fixtures. The Kite access token expires daily; it lives in memory and in the Keychain via `SecretStore`, never in SQLite, `data/`, the repo or the browser (`bz-security`).
- Fixtures are recorded from real APIs only after scrubbing; use obviously fake values (`"test-access-token"`). Details in `bz-security` and `bz-market-data`.
