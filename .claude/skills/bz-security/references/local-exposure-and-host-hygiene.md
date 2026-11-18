# ## 8. Local exposure and host hygiene

(From `bz-security` SKILL.md; open when the task touches this area.)


- **Never public.** No port forward, tunnel (ngrok, cloudflared, `tailscale funnel`), reverse proxy or `ssh -R` to 8787. The Vite dev server keeps its default localhost binding (`server.host` unset). A recipe that starts a server on another interface is a critical finding.
- **Other local users and processes** can open `127.0.0.1:8787`: they get 403 or 401 without the launch token or a session cookie. Do not put the token in a file, env var or CLI flag of any long-lived process.
- **Files:** `data/` and any `.env` are owner-only (`chmod 700 data`, `chmod 600 .env`); `.gitignore` from day one: `.env`, `.env.*` except `.env.example`, `data/`, `*.parquet`, `*.sqlite*`, `*.pem`, `*.key`, `target/`, `node_modules/`, broker UI screenshots. FileVault on; the Keychain holds the tokens, so Time Machine copies of `data/` hold no broker secret but do hold the audit log and positions.
- **Browser hygiene (owner):** use a normal profile with few extensions for the Bazaar tab; the token URL stays in history but is dead after first use. Chrome's Local Network Access prompts and similar browser features are extra, not relied on.
- **Healthcheck** `/healthz` returns no data and needs no session; status detail only behind a session.
