# ADR 0002: where the Kite access token and API credentials live

Status: proposed for F3 BZ-404 (2026-11-21); the owner accepts it by merging.

## Context
Kite issues an access token after the owner logs in; it expires about 06:00 IST the next day.
The token and the API key/secret grant read access to the owner's broker account. They must
survive a server restart within the day, never appear in logs, errors, Debug output, DTOs or
the repo, and never be read by agents.

## Decision
- **API key and secret:** written only by the human `just set-secret` recipe into `secrets/`;
  the process reads them at start. Agents never run it and never read `secrets/`.
- **Access token:** one file under `secrets/` (default `secrets/kite_token`), mode `0600`
  (directory `0700`), written atomically (temp file in the same directory created with `0600`,
  fsync, rename). It stores the token and its expiry instant (IST RFC 3339).
- **Port:** `core::ports::TokenStore { load, save, clear }` returns `AccessToken` (a `Secret` plus
  `expires_at`). The file implementation lives in `data` (BZ-406). A corrupt or unreadable file
  is `TokenError`, treated as "login required", never repaired or logged with contents.
- **Type safety:** `core::secret::Secret` has a redacting `Debug`, no `Display`, no `Serialize`,
  no `Clone`; the value is reachable only through `expose()`, used for the outbound header and
  the login checksum. Errors carry categories only.
- **Expiry:** the token is used only while `Clock::now() < expires_at`; after that every data
  adapter answers `Unavailable` and `/status` says "login required".
- **Never in:** DTOs, OpenAPI types, audit payloads, tracing fields, the `data/` directory, or
  the repo (`secrets/` is git-ignored).

## Alternatives rejected
- OS keychain: adds a dependency and platform code; revisit for F14 (live).
- SQLite table: the database is backed up and inspected more freely than `secrets/`.
- Environment variable: leaks to child processes and dumps.

## Limits
Anyone who can read the owner's home directory as the same user can read the token; 0600 only
protects against other users. Memory is not zeroed on drop (no new dependency in BZ-404).
