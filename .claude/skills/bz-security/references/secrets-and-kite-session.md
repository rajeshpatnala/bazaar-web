# Secrets, macOS Keychain and the Kite daily session

Open this when writing `crates/store/src/secrets.rs` (`KeychainStore`, `EnvStore`, `resolve`; the `SecretStore` trait and `InMemoryStore` live in `bazaar_core::ports`, which never depends on keyring crates), the Kite login routes in `crates/server`, the token exchange in `crates/data`, or Telegram alerts. Companion to `bz-security` sections 2 and 4; typing and tracing rules are in `bz-core`; the Kite client itself is in `bz-market-data` (`references/kite-connect.md`); the callback guard code is in `references/web-auth-template.md`.

## 1. Keychain: `keyring-core` + `apple-native-keyring-store`

As of 2026-11-18 (docs.rs, crates.io): `keyring-core` 1.0.1 holds the portable API (`Entry`, `Error`, `set_default_store`, `unset_default_store`); each platform store is a separate crate. `apple-native-keyring-store` 1.0.2 has no default features: enable `keychain` for the macOS login Keychain (`protected` is the iOS-style Protected Data store, not needed). Depend on these two directly, not on the `keyring` meta-crate. Sources: https://docs.rs/keyring-core , https://docs.rs/apple-native-keyring-store , https://github.com/open-source-cooperative/keyring-rs .

The consequence that bites: **no store is active until startup selects one**. `Entry::new` before `set_default_store` returns `Error::NoDefaultStore`. `set_default_store(Arc<CredentialStore>)` waits for in-flight entry creation, so call it once, early, in the composition roots (`bazaar-server` and `bazaar-cli` `main`), before anything reads a secret. The `Error` enum is `#[non_exhaustive]`: always keep a wildcard arm.

```toml
# crates/store/Cargo.toml (lockfile pins the exact minors; re-check docs.rs when bumping)
keyring-core = "1"
[target.'cfg(target_os = "macos")'.dependencies]
apple-native-keyring-store = { version = "1", features = ["keychain"] }
```

| Item | Shape | Note |
|---|---|---|
| select store | `keyring_core::set_default_store(Arc<CredentialStore>)`; macOS store from `apple_native_keyring_store::keychain::Store::new() -> Result<Arc<Store>>` | once, at process start; `new_with_configuration` targets a specific keychain |
| create entry | `Entry::new(service: &str, user: &str) -> Result<Entry>` | service `"bazaar"`, user = key name |
| text | `set_password(&str)`, `get_password() -> Result<String>` | UTF-8 secrets |
| bytes | `set_secret(&[u8])`, `get_secret() -> Result<Vec<u8>>` | binary |
| delete | `delete_credential()` | removes the stored item |
| errors | `NoEntry` (not set), `NoDefaultStore`, `NoStorageAccess` (locked), `PlatformFailure`, `BadEncoding`, `Invalid`, `Ambiguous`, ... | only `NoEntry` means "absent"; the rest are failures |

Wrapper (ours; the only code that touches `keyring_core`). **Compile-check against the pinned version** before copying: the error path (`keyring_core::Error` vs `keyring_core::error::Error`; docs.rs documents the latter) and the `Arc` coercion come from docs, not from a build.

```rust
// crates/store/src/secrets.rs (trait SecretStore comes from bazaar_core::ports)
use secrecy::{ExposeSecret, SecretString};

pub trait SecretStore: Send + Sync {
    fn get(&self, key: SecretKey) -> Result<Option<SecretString>, SecretError>;
    fn set(&self, key: SecretKey, value: &SecretString) -> Result<(), SecretError>;
    fn delete(&self, key: SecretKey) -> Result<(), SecretError>;
}

#[derive(Clone, Copy, Debug)]
pub enum SecretKey { KiteApiKey, KiteApiSecret, KiteAccessToken, TelegramBotToken }

impl SecretKey {
    fn name(self) -> &'static str {
        match self {
            Self::KiteApiKey => "kite_api_key",
            Self::KiteApiSecret => "kite_api_secret",
            Self::KiteAccessToken => "kite_access_token",
            Self::TelegramBotToken => "telegram_bot_token",
        }
    }
    /// Env fallback name, read only by `resolve` after `dotenvy` ran in a bin.
    fn env(self) -> Option<&'static str> {
        match self {
            Self::KiteApiKey => Some("KITE_API_KEY"),
            Self::KiteApiSecret => Some("KITE_API_SECRET"),
            Self::TelegramBotToken => Some("TELEGRAM_BOT_TOKEN"),
            Self::KiteAccessToken => None,   // the daily token is never read from env
        }
    }
}

pub struct KeychainStore;
impl KeychainStore {
    /// Call once at startup (composition roots), before any `get`/`set`.
    #[cfg(target_os = "macos")]
    pub fn init() -> Result<Self, SecretError> {
        let store = apple_native_keyring_store::keychain::Store::new()?;
        keyring_core::set_default_store(store);
        Ok(Self)
    }
}
impl SecretStore for KeychainStore {
    fn get(&self, key: SecretKey) -> Result<Option<SecretString>, SecretError> {
        match keyring_core::Entry::new("bazaar", key.name())?.get_password() {
            Ok(v) => Ok(Some(SecretString::from(v))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn set(&self, key: SecretKey, value: &SecretString) -> Result<(), SecretError> {
        keyring_core::Entry::new("bazaar", key.name())?.set_password(value.expose_secret())?;
        Ok(())
    }
    fn delete(&self, key: SecretKey) -> Result<(), SecretError> {
        match keyring_core::Entry::new("bazaar", key.name())?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Keychain first, then the env var that `dotenvy` loaded at startup (bins only).
pub fn resolve(store: &dyn SecretStore, key: SecretKey) -> Result<Option<SecretString>, SecretError> {
    if let Some(v) = store.get(key)? { return Ok(Some(v)); }
    Ok(key.env().and_then(|name| std::env::var(name).ok()).map(SecretString::from))
}
```

Notes:
- `SecretError` never embeds the secret value; its `Display` names the key, not the content. If `init` fails (Keychain locked or denied), the server starts with the `.env` fallback for api key and secret only, and daily login reports "cannot store token" instead of falling back to a file or SQLite.
- `InMemoryStore` (a `Mutex<HashMap>`) implements `SecretStore` for tests and the CLI's fixture mode. CI and agents never touch the real Keychain (it prompts or fails).
- macOS may prompt on first access by a new, unsigned or rebuilt binary ("Always Allow" is the owner's choice). Expected locally; never work around it by writing secrets to files. Every `cargo` rebuild can re-prompt; that is a reason to keep the access token in an in-memory cache too, so one prompt serves a whole run.
- The access token is stored under `KiteAccessToken` as a small serialised item holding the token and its IST issue date, so yesterday's token is rejected without a broker call. It is deliberately not resolvable from env.
- First-run setup of api key and secret is owner-run in a terminal: `bazaar-cli set-secret kite_api_key` / `set-secret kite_api_secret` (prompts without echo, writes through `SecretStore` into the Keychain, so the attributes always match what the server reads), or `.env` as the dev fallback (names below). No HTTP route accepts broker credentials.

## 2. Kite Connect daily lifecycle (web flow)

Design is stable; facts are as of 2026-11-18 from the Kite Connect docs (https://kite.trade/docs/connect/v3/user/) and Zerodha support pages: verify there and in `sebi-algo-compliance.md` before the live phase.

1. The owner clicks "Login to Zerodha" in the UI, which navigates (top-level link) to `GET /api/v1/kite/login`. The route needs a session, mints a one-time nonce (5-minute TTL, in memory) and redirects to `https://kite.zerodha.com/connect/login?v=3&api_key=<key>&redirect_params=nonce%3D<nonce>`. Password and TOTP are entered on Zerodha's page; Bazaar never sees them.
2. Kite redirects to the redirect URL registered in the developer console, `http://127.0.0.1:8787/api/v1/kite/callback`, with a short-lived single-use `request_token`; per the docs, whatever was in `redirect_params` is appended to that URL. The browser arrives by a cross-site top-level redirect, so the `SameSite=Strict` session cookie is not sent: the nonce is the only proof that this flow started from the owner's session. The callback requires a known, unused, unexpired nonce, ignores other query keys (Kite may add `status`, `action`, `type`; the docs do not list them), and rejects everything else with 403. If the owner is already logged in to Kite web, a redirect may land on the Kite dashboard instead; fix the redirect URL in the console rather than loosening the callback.
3. Exchange: `POST` to the session endpoint with `api_key`, `request_token` and `checksum` = SHA-256 hex of `api_key + request_token + api_secret`, computed in memory; never log it or its inputs or put them in a persisted URL (`kite-connect.md` has the call shape).
4. Store the returned access token via `SecretStore` (Keychain) with the IST date and cache it in memory. Drop `request_token` immediately; 303 to `/settings?kite=connected` on the UI origin (debug builds: `http://localhost:5173/settings?kite=connected`, so `just ui-dev` lands on the page that holds the session cookie). The UI sees connection status only (`GET /api/v1/status`).
5. The access token is reported to expire at 6 AM the next day, or earlier on logout or master logout from Kite web; a new session with the same API key invalidates the previous token. One running Bazaar per API key; logging in from another tool kills your session.
6. On any `TokenException` or 403: clear the stored token, emit `data_status` with a login-required state, stop the Kite WebSocket without a reconnect loop, block order paths, show the login prompt.
7. Do not script the login or store the password or TOTP seed (`sebi-algo-compliance.md`).

Plan limits (which Kite plan has market data, static IP rules) live only in `sebi-algo-compliance.md` and `config/data.toml`. For orders, SEBI's static-IP requirement is an owner decision for phase 9 (home connection needs an ISP static IP or another owner decision), recorded in `DECISIONS.md`; the kit does not solve it.

## 3. `.env` fallback

- Variables: `KITE_API_KEY`, `KITE_API_SECRET`, `TELEGRAM_BOT_TOKEN`, optional `TELEGRAM_CHAT_ID`. `.env.example` holds names with empty values and is the only dotenv-style file agents may read or edit; the real `.env` is untracked, mode 600, and never read by agents.
- The program calls `dotenvy::dotenv().ok()` once at startup in the bin entrypoints (`crates/server/src/main.rs`, `crates/cli/src/main.rs`); library crates never read env directly except `secrets::resolve`. Scan item 3 flags any other `dotenvy` use.
- The owner may move values from `.env` into the Keychain and delete them from `.env`; the app never edits `.env` itself.
- `just db-check` runs `bazaar-cli check-connections`, which prints `OK kite-session` or `FAILED kite-session <category>`; it never prints tokens, key lengths or prefixes.

## 4. Redaction in tracing

- Credentials are `SecretString`, so `Debug` prints a placeholder. The remaining leaks are URLs and headers: use the custom `TraceLayer` span (method and path only) from the template, never `include_headers(true)`, never `%uri`, never the callback `Query`.
- Add a field-name redaction layer for `secret|token|password|authorization|cookie|csrf|nonce|api_key` as defence in depth. It is a safety net, not the control.
- Errors from `reqwest` use `.without_url()` before they are logged or wrapped (Telegram embeds its token in the URL; the Kite WebSocket URL carries the access token in its query).
- A tracing-capture test runs the `/auth`, Kite callback and failed-exchange paths and asserts that the launch token, nonce, `request_token`, access token and cookie values are absent from the captured output.

## 5. Telegram alerts (optional)

- Alerts normally use the browser Notification API while a Bazaar tab is open; Telegram is an optional second channel behind the `Notifier` port (`bz-architecture`). Telegram uses `reqwest` to `https://api.telegram.org/bot<token>/sendMessage`; build the URL in a single private function, set a timeout, and map errors with `.without_url()` so the token cannot reach logs. The token is a `SecretString` exposed only inside that function.
- Send-only. No polling `getUpdates`, no webhook, no inline commands. Chat id comes from settings and is validated as an integer.
- Templates contain instrument, event and rupee figures only if the owner enabled "include amounts"; never tokens, replayable order ids or screenshots.
- Test with `wiremock` pointing the base URL at the fake; assert the token never appears in captured logs.

## 6. Incident playbook (short)

Suspected leak: stop the server (this drops every session and the launch token), rotate the Kite API secret in the developer console (this also invalidates tokens), delete the Keychain items (`bazaar` service), revoke the Telegram token via BotFather, check `git log -S` for committed secrets and purge if found, review the audit log hash chain, check the broker's order book for unknown orders and kill-switch the account at the broker if any appear, check the terminal scrollback and shell history for a printed launch URL, then add a scan rule or test for the leak path.
