# Money, time and config patterns (open when touching core types, calendars or config loading)

Short, self-written patterns. Adapt names to what `crates/core` already defines; do not duplicate types that exist.

## Money and price newtypes

```rust
use rust_decimal::Decimal;

/// Rupee amount (`Money` in the shared contracts). Never construct from f64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Money(#[serde(with = "rust_decimal::serde::str")] pub Decimal);

/// Price on an instrument. Must be a multiple of that instrument's tick size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Price(Decimal);

impl Price {
    /// Tick size comes from the instrument master, never from a constant.
    pub fn snap_to_tick(raw: Decimal, tick: Decimal) -> Result<Self, DomainError> {
        if tick <= Decimal::ZERO {
            return Err(DomainError::InvalidTick);
        }
        Ok(Self((raw / tick).round() * tick))
    }
}
```

Rules:
- Decimal enters from strings in broker JSON and CSV (`Decimal::from_str`), never via `f64`. Configure serde so Decimal fields are strings on the wire (`serde-str` feature or per-field `with = "rust_decimal::serde::str"`; check the feature names of the pinned version).
- `f64` is allowed inside indicator and options math (`bz-quant`). Convert at the boundary with one named function (`Price::from_model_output`) that snaps to tick and rounds with the repo's documented rounding policy.
- Choose one rounding mode for ledger amounts (paise) and one for tick snapping; write both in `DECISIONS.md` and test the edge cases (exact half, negative P&L).
- Quantity is an integer (`u32`/`i64` shares or lots) with a separate `lot_size` read from the instrument master. Never store "lots" and "shares" in the same field.
- HTTP boundary: DTOs carry Decimals as strings (OpenAPI type `string`); the UI formats them (`bz-web-ui`) and never does arithmetic on money with JS `number`.

## IST and sessions

- Store instants as UTC (`DateTime<Utc>` or epoch millis). Convert to `chrono_tz::Asia::Kolkata` only for session logic and display.
- Trading day, session phase, close time and "is market open" live in `MarketCalendar` (crate `core`, API in `bz-repo-structure`), loaded from `calendar.toml`; nothing else knows the hours.
- Anything time-dependent takes a `&dyn Clock` (or generic `C: Clock`); `SystemClock` in production, `FakeClock` in tests. Direct `Utc::now()` outside `SystemClock` is a review flag.
- Beware of `NaiveDateTime` with no zone; use it only for exchange-file timestamps documented as IST, and convert immediately at the parser.

### The one `Clock` trait (this is the only definition; other skills link here)

```rust
use chrono::DateTime;
use chrono_tz::Tz;

pub trait Clock: Send + Sync {
    /// Current instant in Asia/Kolkata.
    fn now_ist(&self) -> DateTime<Tz>;
}

pub struct SystemClock;
impl Clock for SystemClock {
    #[allow(clippy::disallowed_methods)]   // the single wall-clock read in the workspace
    fn now_ist(&self) -> DateTime<Tz> { chrono::Utc::now().with_timezone(&chrono_tz::Asia::Kolkata) }
}

/// Deterministic clock for tests. Compiled only with the `test-util` feature of `bazaar-core`.
#[cfg(feature = "test-util")]
pub struct FakeClock(std::sync::Mutex<DateTime<Tz>>);

#[cfg(feature = "test-util")]
impl FakeClock {
    pub fn at(t: DateTime<Tz>) -> Self { Self(std::sync::Mutex::new(t)) }
    pub fn advance(&self, d: chrono::Duration) { /* lock, add, store; recover from poison with into_inner() */ }
    pub fn set(&self, t: DateTime<Tz>) { /* lock and replace */ }
}
#[cfg(feature = "test-util")]
impl Clock for FakeClock { fn now_ist(&self) -> DateTime<Tz> { /* lock and copy */ } }
```

- UTC instants for storage come from `now_ist().with_timezone(&Utc)`; there is no second method and no `FixedClock`.
- Feature `test-util` on `bazaar-core` gates `FakeClock` and `fixture_path()`. Consumers enable it only in `[dev-dependencies]` (`bazaar-core = { workspace = true, features = ["test-util"] }`). The name is `test-util` everywhere (never `test-support`); `bz-testing` links here.
- Tests advance time with `advance`/`set`, never `sleep`.

## Config file shape

Every `config/*.toml` starts with `schema_version`, `verified_on` and `source` (`source = "owner"` for owner-set thresholds). The eight files and their schema owners are listed in `bz-repo-structure` section 4. **Do not restate keys here**: `risk.toml` keys (`[mode]`, `[limits]`, ...) are defined only by the `bz-trading-discipline` G-table, `charges.toml` by `bz-backtest-risk` `references/charges-toml.md`, `calendar.toml` by `bz-india-markets`. Shape of a typed struct:

```rust
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskConfig {
    pub schema_version: u32,
    pub verified_on: chrono::NaiveDate,
    pub source: String,
    pub mode: ModeSection,            // [mode] execution_mode, default paper; no env/CLI override
    pub limits: LimitsSection,        // [limits]: Decimal fields as strings, key names per the G-table
    // ... remaining tables per the G-table
}

impl RiskConfig {
    /// Parse then validate; errors name the key, never echo secrets.
    pub fn load(path: &Path) -> Result<Self, ConfigError> { /* read, toml::from_str, validate() */ }
    pub fn validate(&self) -> Result<(), ConfigError> { /* ranges, e.g. 0 < pct <= 100 */ }
}
```

- Validation: ranges, non-empty, dates not in the future, no duplicate symbols, `[hard_limits]` ceilings not loosened. Fail startup with the key path.
- Staleness: `verified_on` older than the file's configured limit (e.g. 90 days for charges) raises a visible warning in the UI and a `warn!` in logs; it never silently uses stale numbers for live-phase decisions.
- Config files hold rates and rules, never secrets.
