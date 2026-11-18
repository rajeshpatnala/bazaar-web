# 8. Repo hygiene files

(From `bz-repo-structure` SKILL.md; open when the task touches this area.)


- `.gitignore` (kit fragment `fragments/gitignore` is authoritative): `target/`, `node_modules/`, `web/dist/`, `web/dist-mock/`, `web/openapi.json`, `web/playwright-report/`, `web/test-results/`, `data/`, `*.parquet`, `*.duckdb*`, `*.db*`, `*.sqlite*`, `logs/`, `coverage/`, with negations `!fixtures/**/*.parquet`, `!fixtures/**/*.db`. Also ensure `.env`, `.env.*` (keep `!.env.example`) and `*.pem` are ignored; if the fragment lacks them, report it rather than skipping.
- `.env.example`: placeholder **names with empty values**; no real-looking secrets; **no mode variable** (mode lives only in `risk.toml`) and no bind setting (loopback only). Keys today: `RUST_LOG`, `BAZAAR_PORT`, `BAZAAR_DATA_DIR`, `BAZAAR_CONFIG_DIR`, `KITE_API_KEY`, `KITE_API_SECRET`, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`. Preferred secret storage is the macOS Keychain; `.env` is the development fallback read by the bins only. Agents read only `.env.example`; a new key is added there first, with a README note.
- `deny.toml`: advisories version 2, yanked deny; license allow-list (MIT, Apache-2.0, BSD-2/3, ISC, Unicode-3.0, Zlib, plus MPL-2.0, OpenSSL and CC0-1.0 as owner-approved for the aws-lc tree; add what the Keychain crates need only after checking `cargo deny check`); `allow-wildcard-paths = true` (internal path deps); bans `openssl-sys` and `native-tls`; sources crates.io only.
- `README.md`: overview, prerequisites (rustup, Node via `.nvmrc`, pnpm, just, nextest, deny), `just setup`, `just run` (opens the browser via the one-time link), `just app-dev` for the mocked UI, testing, the paper-trading disclaimer pointer (`bz-india-markets` `references/compliance-and-disclaimer.md`). No deploy section: Bazaar is local-only.
- Naming: types `UpperCamel`; config keys `snake_case`; React components `PascalCase.tsx`, hooks `useX.ts`, feature folders `kebab-case`.
