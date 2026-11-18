---
name: bz-india-markets
description: Indian market structure for Bazaar - sessions, holidays, expiries, lot/tick sizes, F&O ban, circuits, events, charges taxonomy, data legality, calendar.toml and events.toml schemas, disclaimer wording, lakh/crore formatting. Facts are loaded from files, never hardcoded. Use when touching market facts, calendars, expiry time, or user-facing text.
---

# Indian Markets: Structure, Facts, Compliance

Domain knowledge for NSE/BSE cash and F&O. Rule 2 of the product is absolute: **market facts are loaded, never typed into code**. This skill tells you what kinds of facts exist, where each one comes from, and how to word things. Dated statements below are snapshots ("as of 2026-11-18, reported by ..., verify at ..."); they are examples to verify, never constants, and no session time, rate, limit or regulatory date in this pack is a fact until a human has checked the primary circular. Neighbours: `bz-market-data` (fetching/importing), `bz-core` (config, IST, Decimal), `bz-repo-structure` (`MarketCalendar` API), `bz-backtest-risk` (CostModel, DecisionGate), `bz-execution` (order pipeline), `bz-trading-discipline` (event/ban rules), `bz-testing` (fixture tree). Disclaimer text and no-advice wording live only in `references/compliance-and-disclaimer.md` (open it for any user-facing copy); the SEBI retail algo framework and the pre-live checklist live only in `bz-security` `references/sebi-algo-compliance.md`. Schemas of `calendar.toml` and `events.toml` are owned here: `references/calendar-and-events-schema.md`.

## 1. Where each fact comes from

In short: Official places (verify they still resolve; exchanges reorganise pages):
Full detail, examples and patterns: `references/where-each-fact-comes-from.md` - read it before changing this area.

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

In short: Session structure is **in flux** and this skill states no session time as fact. What was reported as of 2026-11-18, each item to be checked by a human against the primary source before it goes into `calendar.toml` (primary sources: the NSE ...
Full detail, examples and patterns: `references/sessions-and-the-calendar.md` - read it before changing this area.

## 4. Expiries, lots and ticks

In short: Expiry schedules are data from the instrument master, not rules in code. Reported as of 2026-11-18 (broker and news coverage; unverified here): index options expire on a Tuesday (NSE) or Thursday (BSE), weekly expiries are limited to one ...
Full detail, examples and patterns: `references/expiries-lots-and-ticks.md` - read it before changing this area.

## 5. F&O ban, circuits, price bands

In short: Ban: a stock enters the ban period when aggregate open interest crosses an entry percentage of its market-wide position limit (MWPL); during the ban only position-reducing trades are allowed, and normal trading resumes when OI falls below ...
Full detail, examples and patterns: `references/f-o-ban-circuits-price-bands.md` - read it before changing this area.

## 6. Events

`config/events.toml` (header and schema in `references/calendar-and-events-schema.md`) holds dated events; sources are official: exchange board-meeting and corporate-action announcements, RBI MPC schedule (rbi.org.in), Union Budget date (finmin), exchange expiry calendar derived from the instrument master. Kinds: `results`, `rbi_policy`, `budget`, `expiry`, `ex_date` (dividend, split, bonus, rights), `index_rebalance`, `special_session`.
- Scanner penalises, DecisionGate may block, results day and the day before for the affected stock, RBI/Budget days for index trades (`bz-trading-discipline` owns the rule keys).
- Corporate actions adjust history (`bz-market-data`: factor table); the adjusted and raw series are both kept, backtests use adjusted, order sizing uses raw live prices.
- If an event source is unavailable, the event is "unknown", not "none"; show it.

## 7. Charges taxonomy (no numbers here)

In short: `config/charges.toml`, one table per segment and side (equity delivery, equity intraday, equity futures, equity options, plus currency/commodity if enabled), each with `verified_on` and `source`:
Full detail, examples and patterns: `references/charges-taxonomy.md` - read it before changing this area.

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
