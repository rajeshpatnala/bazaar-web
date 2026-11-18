# SEBI retail algo framework and the pre-live checklist (single source)

This is the only place in the pack that states the SEBI/exchange retail algo framework and the pre-live checklist. `bz-india-markets` (`references/compliance-and-disclaimer.md`), `bz-execution` (`references/live-phase.md`) and `bz-architecture` (`references/live-trading-phase.md`) link here. Open it before any work on `crates/broker`, the `live-trading` feature, `ExecutionMode::SemiAuto` or `Auto`, or anything that sends an order to Zerodha. Companion to `bz-security` section 7, `bz-execution` (pipeline, kill switch, audit log) and `bz-trading-discipline` (readiness bar).

**Verify first.** Everything below is a developer summary, as of 2026-11-18, reported by broker pages and secondary articles; it is not legal advice, and exchanges revise thresholds and dates. Before building or enabling order placement, the Manager session re-reads the primary documents in section 6, records date and findings in `DECISIONS.md`, and updates this file. A primary source always wins over this text. Never put a rate, limit, time or date from here into code; such values live in config with `verified_on` and `source`.

## 1. The framework (own words)

Origin: SEBI circular of 4 Feb 2025 on safer participation of retail investors in algorithmic trading, with exchange implementation circulars (NSE/INVG/67858 of 5 May 2025; later NSE circulars incl. NSE/INVG/73992 of 30 Apr 2026 on client-direct API access). Reported fully mandatory for retail from 1 Apr 2026 after extensions (confirm the extension circular).

- **The broker is the principal.** API orders are the broker's responsibility: it authenticates, monitors, rate-limits, keeps records, and may be stricter than the exchange. The app is a client of the broker API, not a market participant.
- **No open APIs.** Client-specific API key, authenticated sessions with 2FA, orders only from broker-whitelisted IPs. Sessions end before the next trading day, so a fresh interactive login is needed daily; no perpetual sessions.
- **Static IP for orders.** Place, modify and cancel need a whitelisted static IP (Zerodha: up to two, one edit per calendar week, set in the developer console; family sharing only under a declaration; one FAQ page shows an earlier date, confirm). Reads (WebSocket data, order book, positions) work from any IP.
- **Order-rate threshold and Algo ID.** Secondary sources put the exchange line at about 10 orders per second per exchange per client: below it the algo is registration-free but orders still carry an exchange-issued identifier (generic for unregistered strategies); above it the strategy is registered with its own Algo ID. Zerodha's own cap is 10 orders per second per client (HTTP 429 above it; invalid requests count; an iceberg counts as one). Per-minute and per-day caps are unverified (sources conflict): keep numbers only in config. Treat 10/s as a ceiling and stay far below; Bazaar is a human-approved, handful-of-orders tool.
- **Order-to-trade ratio.** Reported (no primary source found) that a SEBI circular dated 2026-02-04, effective 2026-04-06, revised OTR rules. Confirm the circular; avoid modify/cancel churn either way.
- **Tagging and audit.** Algo orders carry the exchange tag and stay traceable for years. Bazaar's append-only, hash-chained audit log supports, never replaces, the broker's records.
- **Providers and sharing.** Third parties selling or sharing algos need exchange empanelment, and for black-box strategies research-analyst registration; individuals may share only within the family. Bazaar never grows a feature to publish, sell or distribute strategies or signals, and never claims or implies adviser registration (adviser rules: `bz-india-markets` compliance reference).
- **Market orders and products.** API market orders must carry market protection (zero is rejected); product/order-type restrictions vary by broker: read the current Kite docs and forum notices.
- **Hosting.** Some secondary articles mention exchange-approved hosting for retail algos; unconfirmed. Ask Zerodha how a self-coded personal client on a home machine is classified.
- **Plans.** Free Personal plan: orders, GTT, margins, portfolio, no market data; paid Connect adds live and historical data (price: check the page).

## 2. Design consequences for Bazaar

1. **Static IP.** A home Mac usually has a dynamic IP. Live orders need an ISP static address or a fixed-egress path the user controls (ADR before building `broker`). The app detects an IP-rejection error, engages the kill switch and says so in plain words. Never route orders through a third-party relay. Data and paper are unaffected.
2. **Mode ladder.** Default is `paper`; `semi_auto` (one user click per order, rupee risk shown) is the first live mode; `Auto` stays locked until every checklist item is ok and fresh (section 4), the readiness bar is met and the user opts in.
3. **Rate limits in code.** Token bucket in `execution`, values from `risk.toml` (`max_orders_per_minute` is a Hard ceiling), far below the broker limit.
4. **No perpetual sessions.** No refresh tricks, no stored credentials for unattended login; after daily expiry the app is read-only plus paper.
5. **Algo ID plumbing.** Optional `algo_id` on `OrderIntent` and the request builder, filled only with what the broker or exchange issued, never invented. Always send our own idempotency tag (`"BZ"` + 18 chars, `bz-execution` `references/live-phase.md`).
6. **Audit retention.** Default keep forever; never auto-delete.
7. **Change control.** A strategy-logic change the exchange would treat as a new or modified algo is a compliance event: record in `DECISIONS.md`, re-check registration.
8. **Zerodha uniqueness.** One API key per user for unregistered algos; extra keys add no capacity.

## 3. Code and tests

- `live-trading` is never in `default`; CI builds and clippy-checks it, and no test, agent or CI job calls the live API.
- Tests (fakes only): token expiry and 403 handling with `FakeClock`; IP-rejection engages the kill switch; rate ceiling with a fake clock; compliance loader tests from section 4; `live-trading` symbol absent from the default build.

## 4. Pre-live checklist and `config/compliance.toml`

The checklist state lives in `config/compliance.toml` (phase 9 only; absent file means every item false and live locked). The Manager and the user work through it; agents summarise and never set `ok = true`. File header as every config (`schema_version`, `verified_on`, `source = "owner"`), plus `max_age_days = 30` (owner-set freshness) and one table per item:

```toml
[[item]]
id = "static_ip"
required_for = ["semi_auto", "auto"]   # modes that need this item (see the table)
ok = false
verified_on = "2026-11-18"
evidence = "short note: what was checked, where, by whom"
```

Loader rules: `ok = true` needs non-empty `evidence`; an item older than `max_age_days` counts as false; `semi_auto` against a real broker needs every item with `semi_auto` in `required_for`, `auto` needs all. Ticked state is also noted in `DECISIONS.md` with dates.

| id | required for | Item |
|---|---|---|
| `sources_reread` | semi_auto, auto | Primary documents (section 6) re-read; changes since 2026-11-18 noted |
| `broker_personal_client` | semi_auto, auto | Zerodha confirmed in writing: personal self-coded client permitted; Algo ID/registration position at our order rate; market-protection rules |
| `static_ip` | semi_auto, auto | IP whitelisted, matches the machine's egress IP (via `just db-check`, never by an agent calling services); weekly-change procedure documented |
| `daily_login` | semi_auto, auto | Interactive login works; no stored password or TOTP seed; session end tested with a fake |
| `order_caps` | semi_auto, auto | Per second/minute/day caps below broker and exchange limits, unit-tested with a fake clock |
| `kill_switch_drill` | semi_auto, auto | Engage, confirm zero sends, release; daily-loss breaker, per-order gate and approval re-check tested |
| `audit_reconcile` | semi_auto, auto | Audit chain verifies at startup and export works; reconciliation with the broker order book on start and reconnect |
| `margin_source` | semi_auto, auto | Margin source works for the intended segments (F&O blocked otherwise, no hand estimates) |
| `disclaimer_no_sharing` | semi_auto, auto | Disclaimer on every screen; no sharing or publishing feature exists |
| `paper_readiness` | semi_auto, auto | Readiness bar met from the journal's `ReadinessReport`, not hand-entered; `risk-guardian` sign-off recorded |
| `first_live_limits` | semi_auto | Minimum quantity, one instrument, user present |
| `broker_auto_confirmed` | auto | Zerodha confirmed in writing that API orders without per-order manual approval are allowed |
| `opt_in_phrase` | auto | Typed opt-in confirmed in the UI and audited |

Outside this app's scope: a chartered accountant for tax treatment of F&O.

## 5. Red flags specific to this area

Full scan: `bz-security` section 3. Extra here: `rg -n 'algo_id|Algo ?ID' crates` (every hit must come from broker/exchange data, never a literal) and `rg -n 'static_ip|egress' crates` (IP checks must fail closed).

## 6. Sources (read the originals)

- SEBI circular, 4 Feb 2025, "Safer participation of retail investors in Algorithmic trading" (sebi.gov.in, Legal > Circulars) and its September 2025 extension circular: https://www.sebi.gov.in/legal/circulars/sep-2025/extension-of-timeline-for-implementation-of-sebi-circular-dated-february-04-2025-on-safer-participation-of-retail-investors-in-algorithmic-trading-_96979.html
- NSE circular NSE/INVG/67858 (5 May 2025): https://nsearchives.nseindia.com/content/circulars/INVG67858.pdf ; NSE/INVG/73992 (30 Apr 2026) via NSE's decision-support-tools page: https://www.nseindia.in/static/trade/platform-services-non-neat-decision-support-tools-algorithm-trading
- Zerodha: static IP https://support.zerodha.com/category/trading-and-markets/general-kite/kite-api/articles/static-ip ; Kite Connect FAQs https://support.zerodha.com/category/trading-and-markets/general-kite/kite-api/articles/kite-connect-api-faqs ; explainer (pre-effective-date) https://zerodha.com/z-connect/featured/explaining-the-latest-sebi-algo-trading-regulations ; Kite order API https://kite.trade/docs/connect/v3/orders/
- Secondary cross-checks only: Kotak Neo guide https://www.kotakneo.com/investing-guide/articles/retail-algo-trading-rules-nse-circular/ , Upstox announcement https://upstox.com/developer/api-documentation/announcements/algo-trading-circular , Kite Connect forum https://kite.trade/forum/
