# F2 - Market data foundation (offline reference data)

Status: building
Summary: reference data foundation: real calendar and charges data, typed charges loader, instrument master parse + SQLite store + CLI import.
Owner area: `config/{calendar,charges}.toml`, `crates/{data,store}`, `core/src/{instrument,ports}.rs`, `core/src/config/charges.rs`, `fixtures/data/`, `cli`.

<!-- Plan: .agents/plans/F2-market-data-foundation.md (8 slices, 0..7; slice 7 fills "What was built"). -->

## Acceptance criteria
1. Given the real `config/calendar.toml`, when loaded, then it is not `placeholder`, carries `verified_on` and a per-row `source`, covers the stated years, and `MarketCalendar` answers from it (holidays, sessions, special sessions) with no market literal in source.
2. Given `config/charges.toml`, when loaded, then every `[[rate_set]]` is effective-dated with `verified_on` + `source`, rates are `Decimal` from strings (a TOML float is rejected), unknown keys, duplicate (exchange, segment, side, effective_from) and missing `source` are load errors; a lookup with no rate set in force returns an error, never a fallback.
3. Given a fixture Kite-format instruments CSV, when parsed, then `Instrument` has `Decimal` tick/strike, `NaiveDate` expiry, an `Other(_)` fallback for unknown types, tolerates extra columns, fails with a clear error on a missing required column, and bad rows (non-positive tick/lot, malformed numbers) are rejected and counted in a `CleaningReport`; truncated or oversize input never panics.
4. Given the instrument master, then "next expiry of an underlying" and "F&O eligible (cash_fno)" are functions over the data; a test that moves the fixture expiry weekday shows the code follows it.
5. Given an in-memory/temp SQLite, when the same file is imported twice, then rows are identical (idempotent upsert on `(exchange, tradingsymbol)`), a dated snapshot is kept, `fetched_on` is recorded, migrations run on an empty DB, and Decimal values round-trip exactly.
6. Given `bazaar-cli check-connections --config-dir fixtures/config --data-dir <tmp>`, then it prints `OK sqlite` and `OK|FAILED instruments <category>` (freshness vs the calendar's last trading day); `import-instruments --file <csv>` loads a fixture file. No network, no secret read.
7. Given the calendar deferred items, then TS-21-1..4, TS-21-6 and CR-21-3 (moved here from F1) are done.

## What was built
- Slice 0: `core::instrument` (`Instrument`, `InstrumentType`, `Exchange`, `CleaningReport`), `core::ports::InstrumentRepo` + `RepoError`; new workspace deps `sqlx`, `csv`, `sha2`, dev `tempfile` (versions in root `Cargo.toml`).
- Port contract: `next_expiry(exchange, underlying, kind, on_or_after)` filters by exchange and instrument type; `is_cash_fno(symbol)` needs a cash equity row whose tradingsymbol equals a derivative row's `name`. `Instrument::normalise` (strike 0 to `None` on non-options) then `validate` (expiry/strike by type). The port has no snapshot query yet (slice 4 adds it with the store).
- Slices 1..7 fill the rest.

## Decisions
- Offline F2 / network F3 (Q3); deps approved by the owner (Q4); the owner supplies calendar and charges sources (Q1, Q2): BSE calendar for holidays and special sessions, charges checked against the broker; dummy charges allowed if blocking. Charges are configurable by the owner in `config/charges.toml`.
- `data` and `store` depend only on `core`; only `cli` wires them (cli edges added when first used).

## Known limits
- No live data, no Kite, no candles, no CostModel, no BSE/currency/commodity charges; real data is only as fresh as its `verified_on`. Kite CSV column names are from docs read 2026-11-18 and may change; the parser tolerates extra columns and fails on a missing one.
- If owner input for slices 6/7 is late, the files stay `placeholder` and F2 closes with them deferred.

## Deferred
- TS-21-1..4, TS-21-6 and CR-21-3 (moved from F1): done in slice 1 (calendar tests and a comment in `next/prev_trading_day`).
- Delisting/prune policy for the SQLite instrument master (CR-34-4): F2 does not prune rows absent from a newer import; snapshots keep history and expired rows are filtered by `on_or_after`. Revisit in F3.
- QUAN-33-3 (F3): Kite index rows (tick 0 / lot 0) are dropped as `NonPositiveTick`; decide with a real dump whether to keep an index kind for spot tokens.
- SEC-33-1: CLOSED. `import-instruments --file` bounds the file read with `take(limit + 1)`; test `oversize_file_is_refused_before_reading`.
- SEC-33-2 (F3): one huge quoted field can grow a csv buffer up to `max_bytes`; lower the default once the real master size is known.
- F2 calendar 2027 data: owner supplies the NSE 2027 holiday list before 2027-01-01. Until then dates in 2027 are `OutOfRange` (fail closed, intended); `check-connections` reports `FAILED instruments calendar_out_of_range`.
- Muhurat 2026-11-08 special session: not shipped (timings 14:20-15:50 and circular 172/2025 unverified); add the row once the owner confirms from the NSE circular.
- SEC-64-2 (closed): the owner CONFIRMED on 2026-11-20 that nseindia.in is the official NSE host used as the calendar source.

## Not verified (charges)
- Contract-note cross-check pending: no Zerodha contract note was supplied, so the computed charges are not yet compared with a real one.
- Delivery rates, `[dp]` (Rs 15.34) and IPFT are now owner-supplied ("Charges explained" paste, 2026-11-20), no longer unverified. Not modelled (no schema place): delivery minimum Rs 0.01 per contract note, physically settled F&O rates, DP discounts, currency segments.
- IPFT is the NSE rate (Rs 0.01 per crore = 0.0000001 percent); the paste lists no BSE IPFT, so BSE rows keep `ipft_pct = "0"` meaning not listed.
- Statutory rates (STT/CTT, exchange, SEBI, stamp) are as Zerodha lists them, not checked against NSE/SEBI circulars. effective_from 2026-11-20 is the verified date (the page gives none); `max_age_days = 90` is a default, not from the source.

## History
- Slice 3: instrument master parser (`bazaar_data::instruments`), bounded, dedupes, counts rejects.
- Slice 5: `bazaar-cli import-instruments --file` and `check-connections` (bounded read, 0700 on a newly created data dir only, freshness vs calendar; placeholder calendar never fresh). Acceptance criterion 7 (TS-21-1..4, TS-21-6, CR-21-3) was closed by PR 32 (slice 1).
- Slice 6 of 7, calendar data only (PR 64): real `config/calendar.toml`, 2026 only (`covered_years = [2026]`, verified_on 2026-11-20), from the owner-pasted NSE holiday page (`docs/owner-input/F2-calendar.md`). Equity pre-open 09:00-09:15 and normal 09:15-15:30 for cash_non_fno/cash_fno. Assumption to confirm: holidays also apply to `equity_derivatives`. Gaps: no derivatives/currency/commodity session windows and no closing-auction rows (not in the source), so `session_at` returns Closed for equity_derivatives on trading days. Deviation: the schema has no per-row `source` on `[[holiday]]` rows; they are covered by the top-level `source`. Charges (slice 7) and other criteria are other slices.
- Slice 7 of 7, charges data only: real `config/charges.toml` (Zerodha page pasted 2026-11-20; NSE and BSE, intraday, futures and options, buy and sell; GST 18, option exercise STT 0.15). Pinned by `crates/core/tests/real_charges.rs`.
- Slice 8: charges delivery + `[dp]` + IPFT shipped; `real_charges.rs` asserts the full 16-row grid (closes QUAN-65-4).
