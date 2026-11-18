---
name: bz-india-markets
description: Indian market structure for Bazaar - sessions, holidays, expiries, lot/tick sizes, F&O ban, circuits, events, charges taxonomy, data legality, calendar.toml and events.toml schemas, disclaimer wording, lakh/crore formatting. Facts are loaded from files, never hardcoded. Use when touching market facts, calendars, expiry time, or user-facing text.
---

# Indian Markets: Structure, Facts, Compliance

Domain knowledge for NSE/BSE cash and F&O. Rule 2 of the product is absolute: **market facts are loaded, never typed into code**. This skill tells you what kinds of facts exist, where each one comes from, and how to word things. Dated statements below are snapshots ("as of 2026-11-18, reported by ..., verify at ..."); they are examples to verify, never constants, and no session time, rate, limit or regulatory date in this pack is a fact until a human has checked the primary circular. Neighbours: `bz-market-data` (fetching/importing), `bz-core` (config, IST, Decimal), `bz-repo-structure` (`MarketCalendar` API), `bz-backtest-risk` (CostModel, DecisionGate), `bz-execution` (order pipeline), `bz-trading-discipline` (event/ban rules), `bz-testing` (fixture tree). Disclaimer text and no-advice wording live only in `references/compliance-and-disclaimer.md` (open it for any user-facing copy); the SEBI retail algo framework and the pre-live checklist live only in `bz-security` `references/sebi-algo-compliance.md`. Schemas of `calendar.toml` and `events.toml` are owned here: `references/calendar-and-events-schema.md`.

## 1. Where each fact comes from

| Fact | Source of truth | Refresh | Never |
|---|---|---|---|
| Lot size, tick size, expiry dates, strikes, F&O eligibility | Broker instrument master (Kite `GET /instruments`, CSV; `bz-market-data`) | daily, before the first scan | hardcode `NIFTY_LOT = 75`, or compute "last Tuesday" |
| Trading holidays, special sessions | `config/calendar.toml` filled from the exchange holiday calendar | yearly + on every exchange circular | `weekday < 5` as "market open" |
| Session timings | `config/calendar.toml` (`verified_on`) | on every exchange timing circular | literal `15:30` in logic |
| F&O ban list | exchange daily "securities in ban period" file | daily, pre-open | cache across days |
| Price bands / circuits | exchange daily price-band file + live tick circuit fields | daily | assume 20% |
| Corporate actions, result dates | exchange corporate-announcement archives / vendor / `config/events.toml` | daily | scrape a news site |
| Charges | `config/charges.toml` from broker charges page + circulars | quarterly + on circulars | numbers from memory |
| Margin | broker margin API | per order | estimate by hand |

Official places (verify they still resolve; exchanges reorganise pages):
- NSE holiday calendar (trading and clearing holidays are separate lists): https://www.nseindia.com/resources/exchange-communication-holidays
- NSE circulars (timings, expiry, lot-size changes), PDFs under https://nsearchives.nseindia.com/content/circulars/
- Bhavcopy (EOD): NSE archives host the UDiFF common bhavcopy zips (file names and folders are reported by third parties and change; the current list is at https://www.nseindia.com/static/resources/forms-formats-members). Legacy pre-UDiFF files use other names and column sets; retirement dates of older paths are unverified, confirm at the NSE circular list. URL templates live in `config/data.toml` (`bz-market-data`). BSE equivalent: https://www.bseindia.com/static/members/udiff.aspx
- Kite instruments dump: https://kite.trade/docs/connect/v3/market-data-and-instruments
- Zerodha charges: https://zerodha.com/charges

If a source is unreachable or a file is older than its refresh window, the app shows "data stale" and `just db-check` reports the instrument-master freshness. It never silently falls back to a remembered value.

## 2. Config files carry `verified_on`

Every config file starts with `schema_version`, `verified_on`, `source` (CONTRACTS section 1; `source = "owner"` for owner-set thresholds) and is validated at load. Staleness is a warning in the UI and a hard stop for anything that sizes or places an order. `calendar.toml` has effective-dated `[[session]]` windows per segment and instrument kind, `[[holiday]]` rows per segment, and `[[special_session]]` rows; the full schema, enum values and validation rules are in `references/calendar-and-events-schema.md` (the only copy).

```rust
pub struct Verified<T> { pub verified_on: NaiveDate, pub source: String, pub value: T }

impl<T> Verified<T> {
    pub fn age_days(&self, today: NaiveDate) -> i64 { (today - self.verified_on).num_days() }
}
// Loader returns Err(ConfigError::Stale { file, age_days, max_age_days }) past the limit
// declared per file; callers that size or send orders must propagate it.
```

Anti-patterns (quant-reviewer: grep for these): `const LOT_SIZE`, `if symbol == "NIFTY"` for expiry logic, `Weekday::Tue` outside the test fixtures, `NaiveTime::from_hms_opt(15, 30, 0)` in strategy or backtest code, a `HOLIDAYS: [&str; N]` array, percentages for bans/bands in source.

## 3. Sessions and the calendar

Session structure is **in flux** and this skill states no session time as fact. What was reported as of 2026-11-18, each item to be checked by a human against the primary source before it goes into `calendar.toml` (primary sources: the NSE closing auction page https://www.nseindia.com/static/products-services/closing-auction-session, NSE circulars at https://www.nseindia.com/resources/exchange-communication-circulars with PDFs under https://nsearchives.nseindia.com/content/circulars/, SEBI circulars at https://www.sebi.gov.in/legal/circulars.html):
- Closing Auction Session (CAS) for F&O stocks: reported to start 2026-08-03 with equity derivatives running 10 minutes later than before; verify at NSE/CMTR/74466 and NSE/FAOP/74467 (dated 2026-05-29, as cited by broker coverage). Non-F&O stocks reportedly keep the old close.
- Reported, verify at the NSE circulars of 2026-11-06 (one cash-market and one F&O circular, numbers not confirmed by this pack; locate them in the circular list): effective 2026-12-07 the CAS window moves later and shorter, equity derivatives (stock and non-expiring index) run a few minutes past the current close, and expiring index contracts stop earlier than other derivatives; revised treatment of some unexecuted orders reportedly starts separately on 2027-02-08. A broker article (https://www.kotakneo.com/news/regulations/nse-new-closing-auction-and-f-and-o-trading-rules-from-7-december/) gave exact times; other coverage disagreed, so no time is copied here.
- SEBI circular of October 2026 on reviewing CAS, market timings and derivatives settlement methodology (https://www.sebi.gov.in/sebi_data/attachdocs/oct-2026/1792079571420.pdf; title seen, content not verified by this pack): confirm whether it overrides any NSE item above.
- Pre-open order-entry split from 2026-09-07: reported by a broker note citing NSE/CMTR/74969 and NSE/FAOP/74970; verify there.
- Whether the old post-close session still runs under the new regime: unverified, confirm at the NSE circulars before modelling it.
- Zerodha square-off times for MIS, broker cut-offs and GTT rules are broker facts: take them from the broker, not this file.

Because dates differ by regime, `calendar.toml` windows are effective-dated; a circular adds rows, it never edits history.

Engineering consequences:
- The `MarketCalendar` (`bz-repo-structure`) answers `session_at(ts, segment, kind)`, `is_trading_day(date, segment)`, `next_trading_day` and `close_time(date, segment)`. Close time depends on the segment, the instrument kind and the date; all of it comes from `calendar.toml`.
- **Option expiry instant = `calendar.close_time(expiry_date, segment, kind)`**, never a literal 15:30 or 15:40; time-to-expiry, bar cut-offs and expiry-day rules all read it.
- Special sessions exist: Muhurat trading on Diwali (timing announced by circular), Saturday/Sunday Budget or special sessions, and Saturday mock/DR sessions that must NOT feed live data into analytics. Model the calendar as explicit dated entries, not "Mon-Fri minus holidays".
- A daily bar's "close" is the closing-price method of that day (auction price versus the earlier VWAP-based method; the switch date is per instrument kind and comes from `calendar.toml` regimes). Backtests spanning the change need a per-date flag; label the discontinuity instead of hiding it.
- The bar builder must not assume a 15:30 final tick. Bars are cut from the calendar's session end for that instrument.
- Settlement for equities is T+1 (verify on the exchange site); T+1 matters for paper-trade cash availability (`bz-execution`).
- Segment calendars differ (equity, derivatives, currency, commodity, SLB; trading vs clearing holidays). Key holidays by segment.

Time-of-day behaviour (context for strategy logic and Why? text, not rules):
- First 15 minutes: wide spreads, gaps resolve, false breakouts common; ORB strategies live here, beginners should wait.
- Midday: lower volume, range-bound drift; trend strategies underperform, mean reversion does better.
- Last hour: positioning and trend continuation; MIS square-off flows near the close.
- Expiry-day afternoon: gamma and theta dominate, premiums collapse or spike within minutes, wide option spreads, pinning near large-OI strikes. Defined-risk only; flag it in the UI.

## 4. Expiries, lots and ticks

Expiry schedules are data from the instrument master, not rules in code. Reported as of 2026-11-18 (broker and news coverage; unverified here): index options expire on a Tuesday (NSE) or Thursday (BSE), weekly expiries are limited to one benchmark index per exchange, and an expiry that falls on a trading holiday moves to the previous trading day, possibly cascading over consecutive holidays. Verify at the NSE contract-specification pages, the NSE circular list and the SEBI circular on expiry days (confirm the circular number there), plus https://www.nseindia.com/resources/exchange-communication-holidays; schedules have changed several times in two years. Tests must derive shifts from a fixture calendar, never from a remembered weekday.

Implementation:
- Expiries come from the instrument master: group `instrument_type in (CE, PE, FUT)` rows by `name` and `expiry`, sort, label weekly vs monthly by whether another expiry for the same underlying falls later in the same calendar month. Never derive them from weekday arithmetic.
- `days_to_expiry` uses the calendar (trading days and calendar days both, labelled). Time to expiry for option pricing uses the actual expiry instant, `calendar.close_time(expiry_date, segment, kind)`.
- Lot size and tick size are per instrument and change by circular (lot sizes are revised periodically, tick size is 0.05 for most but can differ). A lot-size change mid-series produces different lot sizes for different expiries of the same underlying; keep lot size on the instrument row, never on the underlying.
- F&O eligibility = presence in the instrument master's NFO segment today. Backtests need the historical eligibility list; if it is unavailable say so ("survivorship caveat", `bz-backtest-risk`).
- Freeze limits (max quantity per order) and strike steps change too; read them from the exchange file or reject when unknown.

## 5. F&O ban, circuits, price bands

- Ban: a stock enters the ban period when aggregate open interest crosses an entry percentage of its market-wide position limit (MWPL); during the ban only position-reducing trades are allowed, and normal trading resumes when OI falls below an exit percentage. Index derivatives have no MWPL ban. Reported by broker explainers (https://support.zerodha.com/category/trading-and-markets/trading-faqs/f-otrading/articles/why-do-futures-and-option-scrips-enter-ban-period-what-does-it-mean); verify the percentages at the exchange's F&O ban circular. They are config values (`ban_entry_pct`, `ban_exit_pct` with `verified_on`), not literals, and the test fixtures use fictional values around them. Approaching-ban stocks (say above ~80% MWPL) deserve a "ban risk" tag, with the cut-off configurable.
- Banned stocks: scanner marks "no new F&O positions" and DecisionGate (`bz-backtest-risk`) rejects openings; closing orders stay allowed. Paper trading enforces the same rule.
- Price bands: each cash stock has a daily band (percentage varies per stock and changes), carried in the exchange's daily band file and on the tick. F&O stocks and indices follow different (dynamic) mechanisms. Market-wide index circuit breakers halt the whole market at several levels. Backtest/paper fills must not execute at or beyond the band, and a locked circuit (no counter-side liquidity) means no fill. Never encode "10%/20%" in code.
- Stock under surveillance (ASM/GSM), T2T segment, and trade-for-trade lists restrict intraday and leverage; treat as `restrictions` flags from the exchange files, surfaced in Why? panels.

## 6. Events

`config/events.toml` (header and schema in `references/calendar-and-events-schema.md`) holds dated events; sources are official: exchange board-meeting and corporate-action announcements, RBI MPC schedule (rbi.org.in), Union Budget date (finmin), exchange expiry calendar derived from the instrument master. Kinds: `results`, `rbi_policy`, `budget`, `expiry`, `ex_date` (dividend, split, bonus, rights), `index_rebalance`, `special_session`.
- Scanner penalises, DecisionGate may block, results day and the day before for the affected stock, RBI/Budget days for index trades (`bz-trading-discipline` owns the rule keys).
- Corporate actions adjust history (`bz-market-data`: factor table); the adjusted and raw series are both kept, backtests use adjusted, order sizing uses raw live prices.
- If an event source is unavailable, the event is "unknown", not "none"; show it.

## 7. Charges taxonomy (no numbers here)

`config/charges.toml`, one table per segment and side (equity delivery, equity intraday, equity futures, equity options, plus currency/commodity if enabled), each with `verified_on` and `source`:
- brokerage (flat or percentage, with caps; per executed order);
- STT (equities, futures, options) or CTT (commodities): charged on a defined side and base (turnover, option premium, or intrinsic value on exercise);
- exchange transaction charges (rate differs by exchange and segment; NSE vs BSE are not the same);
- SEBI turnover fee;
- stamp duty (buy side, state-uniform, by segment);
- GST on brokerage, exchange charges and SEBI fee (not on STT or stamp duty);
- DP charges on delivery sells (per scrip per day, plus depository fee), investor-protection fund charge where applicable.

Rules:
- Rates live only in `config/charges.toml` (schema: `bz-backtest-risk` `references/charges-toml.md`), filled from https://zerodha.com/charges and exchange/SEBI/finance-ministry notices at implementation time, each with `verified_on`. Statutory rates (for example STT) changed in recent budgets and third-party summaries conflict: unverified, confirm at the Finance Act notification or the broker charges page. Record the source in `DECISIONS.md`. No rate appears in skill text or code.
- A calculator that cannot find a rate for a segment/side returns an error, never zero.
- Reference-value tests use a published worked example from the broker's calculator for each segment (store input, expected components, and `verified_on` in the fixture).
- Tax (STCG/LTCG on equity, business income on F&O, audit thresholds, loss carry-forward) is out of scope unless asked; point users to the Income Tax Department and a chartered accountant and never compute tax liability in the app.

## 8. Data legality

Allowed: the broker's API under its plan terms (Kite Connect Personal is free with no market data; Connect adds streaming and history: check https://support.zerodha.com and kite.trade for current price), licensed vendors, official exchange archives (bhavcopy, circulars, ban list, price bands), user-imported CSVs. Not allowed: scraping exchange sites or portals (including the unofficial JSON endpoints the NSE website calls), redistributing vendor data, bypassing login or rate limits, using a friend's API key. Respect plan rate limits (look them up, `bz-market-data`). The app is single-user personal use; no bundled data redistribution. FII/DII flows appear only if a legitimate feed exists; otherwise the tile says "not available".

## 9. Formatting and wording

- Money: Indian grouping, `₹1,00,000` and `₹12,34,56,789`; crore = 100 lakh = 1,00,00,000. Use `Intl.NumberFormat('en-IN', { style: 'currency', currency: 'INR' })` in the UI (`bz-web-ui`). In Rust (no ICU), write one tested helper in `crates/core`:

```rust
/// 1234567.5 -> "12,34,567.50"; last three digits grouped, then pairs.
pub fn group_indian(int_part: &str) -> String {
    let (head, tail) = int_part.split_at(int_part.len().saturating_sub(3));
    let mut out = String::new();
    for (i, c) in head.chars().rev().enumerate() {
        if i > 0 && i % 2 == 0 { out.push(','); }
        out.push(c);
    }
    let head: String = out.chars().rev().collect();
    if head.is_empty() { tail.to_string() } else { format!("{head},{tail}") }
}
```
- Display times in IST (`Asia/Kolkata`) with a 24h clock; dates `dd-MMM-yyyy`. Store UTC or tz-aware instants (`bz-core`).
- Prices to the instrument tick size; quantities as lots for F&O ("1 lot = N units" from the master).
- Terms: MWPL, OI, PCR, IV rank, ATM/ITM/OTM, MIS/NRML/CNC product types, GTT; every term shown in the UI has a plain-English tooltip.
- The disclaimer is permanent and visible; wording rules are in `references/compliance-and-disclaimer.md`. Recommendations say "setup" or "idea for analysis", never "you should buy", never guarantee, always show rupee risk and "low evidence" below ~30 trades.

## 10. Tests

- Calendar: timestamps passed as arguments (the calendar is pure); table tests for each session boundary, a holiday, a Muhurat day, a Saturday special session, an F&O vs non-F&O `close_time`, an effective-dated regime change (same date queried before and after). Use `fixtures/config/calendar.toml` with fictional times (layout and tolerances: `bz-testing`); a real-circular snapshot test is a human-run check, not CI.
- Expiry derivation: feed a recorded instrument-master CSV (including a holiday-shifted week and a lot-size change between series) and assert the derived weekly/monthly labels; never assert against weekday arithmetic.
- Ban/band logic: fixture ban files with boundary values (94.9%, 95%, 80%); position-reducing orders pass, opening orders are rejected.
- Staleness: a config older than its limit makes the loader error; order sizing refuses to run.
- `group_indian`: proptest vs a reference formatter; golden strings for lakh and crore boundaries.
- Static check in CI or review: ripgrep for the anti-patterns in section 2; no live exchange call in any test (`bz-testing`).

## 11. Red flags

- Market fact typed into source or a test oracle that is not a dated fixture.
- A new fetch of an exchange website's JSON/HTML endpoint (scraping), or any request without the official archive/API path.
- Charges or margin "estimated" when a lookup fails.
- Session or expiry logic keyed on weekdays or a fixed 15:30.
- Copy that says "recommended for you", "guaranteed", "sure-shot", or hides the disclaimer.
- Live ordering code merged without the checklist in `bz-security` `references/sebi-algo-compliance.md`.
- Option expiry or time-to-expiry computed from a literal clock time instead of `calendar.close_time`.

## 12. Checklist

- [ ] Every market fact read from instrument master or a config with `verified_on` and `source`.
- [ ] Staleness handled (UI badge, order sizing blocked).
- [ ] Calendar is per segment and per instrument, includes special sessions.
- [ ] Expiries derived from the instrument master; holiday shift covered by a fixture.
- [ ] Ban list, bands and events feed scanner and DecisionGate; opening orders blocked in ban.
- [ ] Charges complete per segment/side, errors on missing rates.
- [ ] Indian number format helper tested; IST display.
- [ ] Disclaimer and no-advice wording applied; unit tests included in the PR.
