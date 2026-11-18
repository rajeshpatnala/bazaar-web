# config/charges.toml: schema, loader and computation order

Open this when creating or changing `config/charges.toml`, the `CostModel` in `crates/backtest`, or any test that asserts a charge. **This file holds no rate numbers on purpose.** Every rate is filled at implementation time from the broker's charges page (zerodha.com/charges), the exchange circulars (NSE and BSE circular archives) and the SEBI circular, never from memory, never from a third-party calculator. Third-party figures conflict, and F&O STT reportedly changed on 1 April 2026 (verify against the exchange circular and the broker page).

## 1. Schema

**This is the one schema for `config/charges.toml`.** Other skills link here and must not restate it; segment names are exactly those below (`equity_futures`, `equity_options`, not `fno_*`). Like every config file it opens with a file-level `schema_version`, `verified_on` and `source`; the per-entry `verified_on`/`source` below are additional, because rates are verified one at a time.

Rates are strings (TOML floats are binary and would lose exactness); the loader parses them into `rust_decimal::Decimal`. One `[[rate_set]]` entry per (exchange, segment, side, effective_from), so a backtest over 2024-2026 can use the rates that were in force on each trade date.

```toml
schema_version = 1
verified_on = "<YYYY-MM-DD>"          # file-level: date the whole file was last reviewed
source = "<URL or circular id>"       # file-level: broker charges page plus circulars

[meta]
broker = "zerodha"
currency = "INR"
max_age_days = 90            # loader warns (UI badge) when verified_on is older than this
history_policy = "effective_dated"   # or "latest_for_all" (flagged as approximate in results)

[gst]
rate_pct = "<decimal>"
verified_on = "<YYYY-MM-DD>"
source = "<URL or circular id>"
# which components GST applies to is code, not config: see section 3

[[rate_set]]
exchange = "NSE"             # NSE | BSE
segment = "equity_delivery"  # equity_delivery | equity_intraday | equity_futures | equity_options
side = "buy"                 # buy | sell
effective_from = "<YYYY-MM-DD>"
verified_on = "<YYYY-MM-DD>"
source = "<URL or circular id, e.g. NSE/FA/xxxxx>"
turnover_basis = "notional"  # notional (price x qty) | premium (options: premium x qty)
brokerage = { kind = "flat_or_pct_lower", flat = "<decimal>", pct = "<decimal>" }  # also: flat | pct | zero
stt_ctt_pct = "<decimal>"    # "0" where the leg is not taxed
exchange_txn_pct = "<decimal>"
sebi_per_crore = "<decimal>"
ipft_pct = "<decimal>"       # confirm whether the broker passes it through separately
stamp_pct = "<decimal>"      # buy side only in practice; confirm on the broker page

[dp]                          # delivery sells only, per scrip per day (not per order)
per_scrip_total = "<decimal>"   # confirm whether the page quotes it with or without GST
includes_gst = false
verified_on = "<YYYY-MM-DD>"
source = "<URL>"

[option_exercise]             # separate STT on intrinsic value for exercised options
stt_pct = "<decimal>"
verified_on = "<YYYY-MM-DD>"
source = "<URL>"
```

Currency and commodity segments are out of scope until the product asks for them.

Loader rules (`CostModel::load(path, today)`):

- `deny_unknown_fields`; missing or duplicate (exchange, segment, side, effective_from) is a hard error.
- The file header and every entry must carry non-empty `verified_on` and `source`; a missing one is a load error, not a warning.
- A trade date with no rate set in force returns `CostError::NoRateSet { segment, side, date }`. It never falls back silently.
- Staleness (`today - verified_on > max_age_days`) produces a `CostWarning` that the backtest result and the UI "costs may be stale" badge carry. It does not stop a run.
- `BacktestResult` records the file hash and the newest and oldest `verified_on` used, so a result can be reproduced and audited.

## 2. Verification routine (implementation-time task)

1. Open the broker charges page and the exchange circulars in effect on each `effective_from` date. Record URL and circular number in `source`, today's date in `verified_on`.
2. Cross-check one computed example against a real contract note or the broker's own brokerage calculator output; save that example as `fixtures/charges/contract_notes/<date>-<segment>.json` (inputs and the broker's line items).
3. Where sources disagree (GST base, DP per-scrip figure, IPFT treatment, STT on F&O after the reported 1 Apr 2026 change), the broker's own page and the contract note win. Record the disagreement and the choice in `DECISIONS.md`.
4. Re-run the golden backtest and review the stats diff; costs changed, so baselines change in the same PR.

## 3. Computation order (per executed leg)

Inputs: exchange, segment, side, trade date, price, quantity (units, not lots), and whether this is the first delivery sell of that scrip on that day.

```
turnover   = price x qty                       (options: premium x qty)
brokerage  = per rate_set.brokerage kind       (flat_or_pct_lower: min(flat, pct x turnover))
stt_ctt    = stt_ctt_pct x turnover            (sell side only for most segments; per rate_set)
exch_txn   = exchange_txn_pct x turnover
sebi       = sebi_per_crore x turnover / 10_000_000
ipft       = ipft_pct x turnover
stamp      = stamp_pct x turnover              (buy side)
gst        = gst_rate x (brokerage + exch_txn + sebi)    <- the broker page's stated base
dp         = [dp] amount on the first delivery sell of a scrip per day, else 0
total      = brokerage + stt_ctt + exch_txn + sebi + ipft + stamp + gst + dp
```

- GST is not applied to STT or stamp duty. The base above is what the broker's page states (brokerage, SEBI charges and transaction charges). Some third-party calculators use brokerage plus exchange charges only; if a real contract note disagrees with this line, the contract note wins and the formula changes in one place.
- Return a `ChargeBreakdown` struct with one `Decimal` field per component plus `total`. The UI and Why panels show the breakdown; metrics use `total`.
- Rounding: decide once (for example round each component to 2 decimals, half-up, then sum), verify against a contract note, record in `DECISIONS.md`. Never round inside intermediate multiplications.
- A round trip is two legs; compute each leg with its own rate set and side. Intraday squared off (MIS) uses `equity_intraday` for both legs; delivery uses `equity_delivery`.
- F&O: exercise and assignment of options add the `[option_exercise]` leg on intrinsic value.

## 4. Tests

- **Arithmetic and order**: `fixtures/charges/synthetic_rates.toml` with obviously fake round numbers (file header says TEST ONLY, never loaded by app code). Assert the exact total, that GST uses the stated base, that STT is excluded from the GST base, that DP is charged once per scrip per day, that a zero-rate component yields zero.
- **Golden**: each recorded contract note reproduces to the paisa with the real `config/charges.toml`. These tests fail loudly when rates are edited without re-verifying.
- **Loader**: missing `verified_on`, duplicate keys, unknown field, date with no rate set, stale warning (fake `today` passed in; no wall clock).
- **Properties** (proptest): total is non-negative; total never decreases when qty or price grows (same rate set); buy and sell legs with the same inputs differ only by side-specific components.
