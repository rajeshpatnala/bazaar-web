# 10. The `broker` crate and `live-trading`

(From `bz-execution` SKILL.md; open when the task touches this area.)


- `crates/broker/Cargo.toml` is a workspace member whose real code is `#[cfg(feature = "live-trading")]`; `default = []` in every crate. `crates/server` and `cli` forward the feature (`live-trading = ["bazaar-broker/live-trading"]`) and never enable it by default. CI: `cargo check` and `clippy -p bazaar-broker --features live-trading` compile it; tests never call a live API.
- Compiled-in is not enabled. `LiveExecutor::new` additionally requires `mode != Paper`, a passed compliance checklist, a valid same-day session, the owner's static IP (from the ISP; an owner decision for the live phase) as the whitelisted egress, and the opt-in row written by the human-only `bazaar-cli live-opt-in` (never a UI toggle or HTTP route). Build wiring that selects `LiveExecutor` exists only in the composition roots.
- `LiveExecutor` is a thin adapter: map `ApprovedOrder` to Kite parameters, set `tag` from the intent (idempotency), `market_protection` for market/SL-M orders, classify errors into `ExecError`, push fills from order updates (postback/WS) into the same `ExecEvent` channel, and reconcile on start and after every `Unknown`.
- Credentials come from the program's config/secret loader (`bz-core`, `bz-security`); agents never read `.env`, never print tokens, and check connection state only via `just db-check`.
- Open `references/live-phase.md` before writing any code in this crate: Kite order API mechanics, order placement sequence, reconciliation algorithm. The SEBI framework and pre-live checklist live only in `bz-security` `references/sebi-algo-compliance.md`.
