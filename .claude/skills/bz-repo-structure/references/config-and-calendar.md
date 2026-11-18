# Config loader and MarketCalendar sketches

Open this when writing `bazaar_core::config` or `bazaar_core::calendar`. This file shows Rust structure only. It does **not** restate any TOML schema or key list; each has one owner:

| File | Schema / keys owned by |
|---|---|
| `risk.toml` (`[mode]`, `[limits]`, `[override]`, `[hard_limits]`, ...) | `bz-trading-discipline` (the G-table is the only key list) |
| `charges.toml` | `bz-backtest-risk` `references/charges-toml.md` |
| `calendar.toml` (holidays, sessions per segment, special sessions) | `bz-india-markets` (section 2 and 3) |
| `events.toml` | `bz-india-markets` |
| `strategies.toml` | `bz-quant` |
| `data.toml` | `bz-market-data` |
| `compliance.toml` (phase 9 only) | `bz-security` `references/sebi-algo-compliance.md` |
| `watchlists.toml` | this skill (SKILL.md section 4) |

Every file starts with `schema_version`, `verified_on`, `source` (`source = "owner"` for owner-set thresholds). Values in fixtures are fictional; real values need a dated official source.

## 1. Loader pattern

```rust
pub trait ConfigFile: serde::de::DeserializeOwned {
    const FILE: &'static str;
    fn header(&self) -> &Header;
    fn validate(&self) -> Result<(), ConfigError>;
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header { pub schema_version: u32, pub verified_on: chrono::NaiveDate, pub source: String }

pub fn load<T: ConfigFile>(dir: &std::path::Path) -> Result<T, ConfigError> {
    let path = dir.join(T::FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| ConfigError::Io { file: T::FILE, source: e })?;
    let value: T = toml::from_str(&text).map_err(|e| ConfigError::Parse { file: T::FILE, source: e })?;
    value.validate()?;
    Ok(value)
}
```

- `#[serde(flatten)]` conflicts with `deny_unknown_fields` on some serde versions: prefer explicit header fields in each struct plus a shared `header()` accessor.
- Decimal fields use `rust_decimal::serde::str` so TOML strings parse and TOML floats fail.
- `validate()` checks ranges, ordering (session starts before ends), unique holiday dates per segment, non-empty symbols. It returns the first failing key path, never file contents. `RiskConfig::validate` also enforces the `[hard_limits]` ceilings and refuses a loosened value (rules in `bz-trading-discipline`).
- `Config::stale_warnings(today, &StalenessPolicy) -> Vec<StaleWarning>` compares `verified_on` with `today` from the `Clock`, not the system.
- The error type carries file name and key path only, so logging it cannot leak anything.
- Execution mode is read from `risk.toml` `[mode]` only; there is no env var and no CLI flag for it.

## 2. MarketCalendar sketch

The API takes the segment and the instrument kind, because session end differs (F&O stock closing auction vs non-F&O cash, derivatives vs cash). The `Segment` and `InstrumentKind` variants and the `applies_to` values in the TOML are the ones defined by the `bz-india-markets` schema; do not invent a second set.

```rust
use chrono::{DateTime, NaiveDate, NaiveTime, Utc, Weekday, Datelike};
use chrono_tz::Asia::Kolkata;

pub enum Session { Closed, PreOpen, Normal, ClosingAuction, Special(String) }

impl MarketCalendar {
    pub fn from_config(cfg: &CalendarConfig) -> Result<Self, CalendarError>;

    pub fn is_trading_day(&self, d: NaiveDate, seg: Segment) -> Result<bool, CalendarError> {
        self.ensure_covered(d)?;                       // OutOfRange if the year is not in the file
        if self.specials.contains_key(&(d, seg)) { return Ok(true); }
        Ok(!matches!(d.weekday(), Weekday::Sat | Weekday::Sun) && !self.is_holiday(d, seg))
    }

    /// Phase at an instant for one segment and instrument kind. Windows come from calendar.toml.
    pub fn session_at(&self, t: DateTime<Utc>, seg: Segment, kind: InstrumentKind)
        -> Result<Session, CalendarError>;

    /// End of the last session of the day for the segment: option expiry time, bar cut-off.
    pub fn close_time(&self, d: NaiveDate, seg: Segment, kind: InstrumentKind) -> Result<DateTime<Utc>, CalendarError>;

    pub fn next_trading_day(&self, d: NaiveDate, seg: Segment) -> Result<NaiveDate, CalendarError>;
    pub fn prev_trading_day(&self, d: NaiveDate, seg: Segment) -> Result<NaiveDate, CalendarError>;

    /// Expiry on a holiday moves back to the prior trading day.
    pub fn shift_to_prior_trading_day(&self, mut d: NaiveDate, seg: Segment)
        -> Result<NaiveDate, CalendarError> {
        while !self.is_trading_day(d, seg)? { d = d.pred_opt().ok_or(CalendarError::OutOfRange(d))?; }
        Ok(d)
    }
}
```

Rules:
- Session windows, special sessions and holidays only from `calendar.toml`; `session_at` finds the window whose `applies_to` covers `kind`. No `09:15` or `15:30` literal in Rust outside tests.
- Option expiry time, time-to-expiry and bar cut-offs use `calendar.close_time(expiry_date, segment, kind)`, never a literal.
- The calendar is pure (no clock inside): tests pass timestamps built from IST local times and a fixture calendar from `fixtures/config/calendar.toml`.
- A timing circular that changes sessions is a `calendar.toml` change with a new `verified_on`, plus fixture and test updates; the Rust API stays the same.

Test cases (fixture calendar; `FakeClock` is unnecessary because timestamps are arguments):
- Non-F&O cash case: 09:14:59 IST is `PreOpen`, 09:15:00 `Normal`, 15:29:59 `Normal`, 15:30:00 `Closed`. This 15:30 case is only valid for the **non-F&O** instrument kind in the fixture.
- F&O-stock case: the fixture's closing-auction window gives `ClosingAuction` at its start and `Closed` at its end; `close_time` differs from the non-F&O close for the same date. Expect fixture values to change when `calendar.toml` does.
- A holiday on a Thursday shifts a Thursday expiry to Wednesday; consecutive holidays shift further; a Monday holiday shifts back across the weekend to Friday.
- A holiday listed for one segment does not close another segment.
- A date in a year absent from the file returns `OutOfRange`.
- A special session on a Saturday is a trading day with its own hours.
- Property: for trading days `next_trading_day(prev_trading_day(d)) >= d`, and every returned day satisfies `is_trading_day`.

## 3. Clock and fixture helpers (feature `test-util`)

The `Clock` trait, `SystemClock`, `FakeClock` and the `test-util` feature are defined once in `bz-core` `references/money-time-config.md`. This crate re-exports them; do not define a second trait (`FixedClock` does not exist).

```rust
#[cfg(feature = "test-util")]
pub fn fixture_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(rel)
}
```

`env!("CARGO_MANIFEST_DIR")` expands in `core`, so `../../fixtures` resolves to the repo root for every caller. `test-util` is enabled only through `[dev-dependencies]`, so none of it is in release builds. The fixture tree itself is owned by `bz-testing`.
