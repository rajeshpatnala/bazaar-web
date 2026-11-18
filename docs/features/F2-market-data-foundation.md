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

## History
-
