# 3. The MarketData port

(From `bz-market-data` SKILL.md; open when the task touches this area.)


Vendors stay swappable behind one trait in `crates/data`. Domain types (`Instrument`, `Tick`, `Candle`) live in `crates/core`.

```rust
#[async_trait::async_trait]
pub trait MarketData: Send + Sync {
    async fn instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>, DataError>;
    async fn candles(&self, req: CandleRequest) -> Result<Vec<Candle>, DataError>;   // chunked inside
    async fn quote_snapshot(&self, keys: &[InstrumentKey]) -> Result<Vec<Quote>, DataError>;
    async fn subscribe(&self, keys: &[InstrumentKey], mode: StreamMode)
        -> Result<TickStream, DataError>;                                            // Stream<Item = TickEvent>
    fn freshness(&self) -> Freshness;
}

pub enum TickEvent { Tick(Tick), Status(StreamStatus) }  // Connected, Reconnecting{attempt}, SessionExpired, Stale
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("session expired, login required")] SessionExpired,   // HTTP 403 / TokenException: never retried
    #[error("rate limited")] RateLimited { retry_after: Option<std::time::Duration> },
    #[error("vendor unavailable")] Unavailable,                   // 5xx, network: retryable
    #[error("bad request: {0}")] BadRequest(String),
    #[error("decode failed at {path}")] Decode { path: String },
    #[error("plan does not include this data")] NotEntitled,
}
```

- Key instruments by `InstrumentKey { exchange, tradingsymbol }`. Vendor tokens (Kite `instrument_token`) are a lookup detail resolved through the instrument master, because exchanges can reuse derivative tokens after expiry.
- Adapters: `KiteMarketData` (live + historical), `BhavcopyStore` (EOD only; implements `candles` for `day`), `CsvImport` (fallback). A `CompositeMarketData` routes by capability and plan. Candle requests the plan cannot serve return `NotEntitled`, and the UI offers the CSV or bhavcopy path.
- Request pacing lives in the adapter (a token-bucket per endpoint class, values from `config/data.toml`), not in callers.

### 3a. `config/data.toml` (owned here)

Starts with the standard header (`schema_version`, `verified_on`, `source`); each table below also carries its own `verified_on` and `source` (a docs URL or circular number), because vendor pages change independently. Missing key means the adapter refuses to start that feature (fail closed).

| Table | Keys (all values come from the cited vendor page, never from memory) |
|---|---|
| `[kite.urls]` | `api_base`, `login`, `ws` (no keys or tokens, only templates) |
| `[kite.rate_limits]` | requests per second per endpoint class: `quote`, `historical`, `default`; `max_retries`, `backoff_base_ms`, `backoff_cap_ms` |
| `[kite.limits]` | `ws_max_instruments_per_connection`, `ws_max_connections`, `quote_batch_size`, `ltp_batch_size` |
| `[kite.historical]` | `window_days.<interval>` per interval (not stated on the docs page; confirm in the docs or forum before filling) |
| `[bhavcopy.<exchange>_<segment>]` | `url_template` (placeholders like `{YYYYMMDD}`), `publish_after_ist`, `max_download_bytes`, `series_keep` |
| `[refresh]` | `instruments_after_ist`, `stale_after_secs`, `delayed_after_secs`, `max_reject_ratio`, `price_gap_flag_pct` |

Fixture copy: `fixtures/config/data.toml` with fictional values, loaded by tests with `--config-dir fixtures/config`.

Corporate actions are **not** a config file: they live in the store table `corporate_actions` (section 7) and are loaded with `bazaar-cli import-actions <csv>`.
