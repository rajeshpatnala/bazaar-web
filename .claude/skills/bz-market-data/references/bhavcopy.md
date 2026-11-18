# Bhavcopy importer reference (official EOD archives)

Read when building or changing `crates/data/src/bhavcopy`. Bhavcopy is the exchange's own end-of-day summary file, published for download. It is the legal base for the EOD universe, delivery percentage and F&O settlement data. Facts here come from 2026-11-18 research; exchange file names and formats change often, so treat every URL and column below as "verify at implementation time" and keep them in config.

## 1. Sources (official only)

| Exchange | Where to check the current list | Note |
|---|---|---|
| NSE | <https://www.nseindia.com/resources/forms-formats-members> and the "all reports" listing on nseindia.com | UDiFF files for CM and FO; archive host `nsearchives.nseindia.com` |
| BSE | <https://www.bseindia.com/static/members/udiff.aspx> and <https://www.bseindia.com/markets/marketinfo/BhavCopy.aspx> | the BSE site returned 403 to automated fetches during research; verify by hand |
| SEBI/UDiFF spec | guidance document <https://www.bseindia.com/downloads1/UDiFF_guidance_document_Ver1_0.pdf> and the exchange-published catalogue (Excel) | catalogue is the authority on field names |

Format history (verified in search results): older NSE `cm<DD><MON><YYYY>bhav.csv`-style and BSE `eqddmmyy.csv` / `BSE_EQ_BHAVCOPY_DDMMYYYY` files were retired in favour of UDiFF (BSE discontinued the legacy equity file in July 2024; NSE's UDiFF CM bhavcopy is the maintained one). NSE circulars in 2026 mention further bhavcopy and DAT-format changes in phases, so re-check the current listing before coding, not just this file.

Reported NSE UDiFF archive file names (third-party catalogue, confirm against NSE's listing before use):
- CM: `.../content/cm/BhavCopy_NSE_CM_0_0_0_<YYYYMMDD>_F_0000.csv.zip`
- F&O: `.../content/fo/BhavCopy_NSE_FO_0_0_0_<YYYYMMDD>_F_0000.csv.zip`
- Security-wise full file with delivery: `.../products/content/sec_bhavdata_full_<DDMMYYYY>.csv` (date style differs from the UDiFF files; third-party notes say it has no ISIN column and its header must be read from a real file)
- BSE CM UDiFF file name could not be verified; take it from BSE's UDiFF page.

Put URL templates in `config/data.toml` as `[bhavcopy.<exchange>_<segment>] url_template = "...{YYYYMMDD}..."` with the table's `verified_on` and `source` (schema: SKILL.md section 3a). When a template stops working the fix is a config edit, not a rebuild.

## 2. Downloading politely and legally

- This is an archive of published files: one request per trading day per file, using a plain, honest `User-Agent` such as `bazaar/<version>`. A 404 for today before the exchange publishes (evening IST) is normal: report "not yet published", retry later.
- Cache every downloaded zip under `data/raw/bhavcopy/<exchange>/<segment>/` with a SHA-256 in the import log; never re-download what you have.
- Space requests politely and cap concurrency at 1 for archive hosts. On `403`/`429`/captcha-like HTML, stop and tell the user to download the file manually into the raw cache folder (above) and run `bazaar-cli import-bhavcopy --date <YYYY-MM-DD>`, which picks up the cached file (the CLI surface is owned by `bz-repo-structure`; `import-bhavcopy` and the cache refresh touch live hosts, so they are human-only commands). Do not spoof browser headers, replay cookies, rotate IPs or parse HTML pages; third-party reports say some NSE paths need session cookies, which is exactly when the manual path applies.
- Check the content type and size (cap `max_download_bytes` from `data.toml`) before unzipping; zip entries limited to one expected CSV; reject path traversal names (`zip` entry `enclosed_name()`).
- Which trading days to fetch: from `MarketCalendar` (`bz-india-markets` schema, `calendar.toml`), not from weekdays alone. Missing days that the calendar says were trading days are a gap to backfill.

## 3. UDiFF layout (what to expect)

UDiFF (Unified Distilled File Formats) is a SEBI-coordinated naming standard: one set of ISO-20022-style column names across exchanges and segments, comma-separated, with ISO 8601 dates. Header names come from the exchange catalogue; the ones used for bhavcopy are widely reported as follows, but verify against the real header at runtime:

| Meaning | Typical column |
|---|---|
| Trade date | `TradDt` |
| Instrument type (equity, index/stock futures and options) | `FinInstrmTp` |
| ISIN | `ISIN` |
| Symbol | `TckrSymb` |
| Series (EQ, BE, ...) | `SctySrs` |
| Expiry, strike, option type | `XpryDt`, `StrkPric`, `OptnTp` |
| Open, high, low, close, last, previous close | `OpnPric`, `HghPric`, `LwPric`, `ClsPric`, `LastPric`, `PrvsClsgPric` |
| Settlement price (derivatives) | `SttlmPric` |
| Volume, value, trades | `TtlTradgVol`, `TtlTrfVal`, `TtlNbOfTxsExctd` |
| Open interest and change | `OpnIntrst`, `ChngInOpnIntrst` |

Parser rules:
- Read the header row, build a `HashMap<name, index>` and fail fast with a `MissingColumn(name)` reject for required columns; ignore unknown columns (forward compatible); trim fields; handle BOM and CRLF.
- Layout detection: pick a `LayoutId` from the header (UDiFF CM, UDiFF FO, legacy, sec_bhavdata_full, user CSV). One module per layout implementing `trait BhavParser { fn layout() -> LayoutId; fn parse(&self, rdr) -> ParsedBatch }`. Unknown header -> `UnknownLayout` error listing the first column names (not the data rows).
- Equity universe: keep series per config (`EQ` and optionally `BE`/`SM`), not a literal in the parser. F&O rows are mapped to instruments via `(symbol, expiry, strike, option_type)`; contracts absent from today's instrument master (already expired) still import, flagged `instrument_known=false`.
- Prices as `Decimal`; empty or `-` becomes `None`, never zero.

## 4. Delivery data

The security-wise delivery position (the `sec_bhavdata_full`-style file, or the exchange's delivery report) provides traded quantity, delivered quantity and delivery percentage per security per day. Column names in that file were not verified: open a real sample, record the header in the fixture, and map columns by name. Store `delivered_qty` and `deliv_pct` as `Decimal`/`u64` next to the day bar (a `delivery` Parquet dataset keyed by `(symbol, date)`), joined to bars at read time. Delivery percentage is a context field for `bz-quant` scoring, not a signal on its own.

## 5. Import pipeline

Pipeline: read (bounded) -> parse -> clean -> validate -> upsert -> report. Nothing leaves the importer except typed rows and a `CleaningReport`.

1. `ImportPlan`: list `(exchange, segment, date)` to import from the calendar minus `import_log` (gaps) or from an explicit `--date`.
2. Fetch or open the file; hash it; skip if `(file_hash)` already in `import_log` with status `ok`.
3. Parse into raw rows; clean (series filter, trim, empty to `None`); validate (price relations, positive volume, date equals file date).
4. Upsert into the candle store by `(exchange, symbol, interval='day', date)`. Same file twice gives the same rows; a corrected re-publication replaces rows and logs the replacement.
5. Write `ImportReport { read, accepted, rejected, by_reason, layout, file_hash }` to `import_log` and print it from `bazaar-cli import-bhavcopy`. Abort without writing when rejects exceed `max_reject_ratio` (`data.toml`).
6. After import, run the corporate-action cross-check (unexplained gap vs previous close).

Prices in bhavcopy are raw (unadjusted). Never adjust on import; see SKILL.md section 7.

## 6. CSV import fallback

`bazaar-cli import-csv <file> --mapping mapping.toml` (symbol and interval are declared in the mapping file): user-declared column mapping, explicit date format and timezone (IST assumed, stated), decimal separator rules, same validation and report. Imported rows are tagged `source = "csv:<file hash>"` so they can be audited or removed. This is the path for licensed vendor exports and for days the archive refuses to serve.

## 7. Tests

- Fixtures in `fixtures/bhavcopy/` (tree owned by `bz-testing`): tiny CM and FO UDiFF samples (3-10 rows each, hand-trimmed from a real file by the human owner or hand-written in the same shape), sec_bhavdata sample, legacy sample, malformed rows, header-only, empty, BOM + CRLF, wrong zip contents.
- Table-driven parser tests per layout; reject-reason assertions; `insta` snapshot of normalised output; idempotent re-import; gap detection from a fake calendar; downloader against `wiremock` (200 with zip, 404 not-yet-published, 403 -> "manual import" outcome, oversized body).
- No test downloads from NSE or BSE.
