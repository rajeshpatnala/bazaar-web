# 5. Core types and `MarketCalendar` (dev-core)

(From `bz-repo-structure` SKILL.md; open when the task touches this area.)


`core` has no I/O besides config file reading, depends on nothing internal, and everything depends on it.

- Money and prices: `Money`/`Price` over `rust_decimal::Decimal` (`bz-core`); arithmetic returns the same type; no `From<f64>`. Quantities are integers (`Qty(u32)`, `Lots(u32)`); lot size comes from the instrument master.
- Domain: `Instrument`, `Tick`, `Candle` (timeframe, `close_time`, adjusted flag), `OptionChain`, `Order`, `Position`, `Signal`, `Recommendation` (fields: `bz-quant`). Plain `serde` data; behaviour lives in the owning crate. `DataView` and persistence ports follow `bz-backtest-risk` and `bz-architecture`.
- Time: instants as `DateTime<Utc>`, IST trading days as `NaiveDate`; convert at the edge with `chrono_tz::Asia::Kolkata`. Never `Local`, never bare `NaiveDateTime`.
- One `Clock` trait, `SystemClock` and `FakeClock` (feature `test-util`), defined in `bz-core` `references/money-time-config.md`. Session-aware code takes `&dyn Clock` or a timestamp argument.
- Errors: `thiserror` enum per crate (`DomainError`, `ConfigError`, `CalendarError`); no `anyhow` in libraries.

`MarketCalendar` (`bazaar_core::calendar`) is built from `calendar.toml` (schema in `bz-india-markets`). Its methods take a segment, and the session lookup an instrument kind, because close time differs between F&O stocks, non-F&O cash and derivatives:

```rust
is_trading_day(d, segment) -> Result<bool, CalendarError>
session_at(ts, segment, kind) -> Result<Session, CalendarError>
close_time(d, segment, kind) -> Result<DateTime<Utc>, CalendarError>   // expiry time, bar cut-off
next_trading_day(d, segment) / prev_trading_day(d, segment)
shift_to_prior_trading_day(d, segment)                           // expiry on a holiday
```

Rules:
1. Holidays, session windows, special sessions and muhurat hours come **only** from `calendar.toml`. No `09:15`/`15:30` literal or `NaiveDate::from_ymd_opt(20` in source outside tests.
2. Option expiry time is `calendar.close_time(expiry_date, segment, kind)`, never a literal.
3. Weekends are non-trading unless a dated `special_session` entry says otherwise.
4. A date outside the covered years returns `CalendarError::OutOfRange`; never guess "weekday means open".
5. Expiry dates come from the instrument master (`bz-market-data`); the calendar supplies only the holiday shift.
6. The calendar is pure (no clock inside): tests pass timestamps and the fixture `fixtures/config/calendar.toml`. A 15:30 `Closed` test is the non-F&O case only. Code and cases: `references/config-and-calendar.md`.

---
