# `calendar.toml` and `events.toml` schemas (owned by `bz-india-markets`)

Open this when writing `bazaar_core::calendar`, the config loader for these two files, or a calendar fixture. The Rust API (`MarketCalendar`, `Segment`, `InstrumentKind`) is in `bz-repo-structure` `references/config-and-calendar.md`; it must use exactly the enum values below. Values in this file are illustrative shapes, **not market facts**: real windows and holidays are copied from official circulars with a `verified_on` date.

## 1. `calendar.toml`

File header (CONTRACTS section 1; every config file starts with it):

```toml
schema_version = 1
verified_on = "YYYY-MM-DD"                 # date a human compared this file with the sources
source = "NSE holiday page + circular nos. listed per [[session]]"
covered_years = [2026, 2027]               # a date outside these years returns CalendarError::OutOfRange
```

Enumerations (strings in TOML, enums in Rust, unknown value is a load error):

| Key | Values |
|---|---|
| `segment` | `equity`, `equity_derivatives`, `currency`, `commodity` (add `slb` only when used) |
| `applies_to` (instrument kind) | `all`, `cash_non_fno`, `cash_fno`, `derivative`, `derivative_expiring_index` |
| `phase` | `pre_open`, `normal`, `closing_auction`, `post_close` |

```toml
# One row per window. Times are IST "HH:MM"; names are labels, times are data.
# A circular that changes timings adds rows with a new effective_from; it never edits history,
# so backtests over old dates keep the old regime.
[[session]]
segment = "equity"
applies_to = "cash_non_fno"
phase = "normal"
start = "<HH:MM>"
end = "<HH:MM>"
effective_from = "<YYYY-MM-DD>"
source = "<circular no.>"

[[session]]
segment = "equity"
applies_to = "cash_fno"
phase = "closing_auction"
start = "<HH:MM>"
end = "<HH:MM>"
effective_from = "<YYYY-MM-DD>"
effective_to = "<YYYY-MM-DD>"             # optional; omitted = open-ended
source = "<circular no.>"                  # one row per regime change (see SKILL.md section 3)

[[holiday]]
date = "2026-01-26"
segments = ["equity", "equity_derivatives"]   # trading holidays are per segment
kind = "trading"                              # trading | clearing (clearing holidays do not close trading)
name = "Republic Day"

[[special_session]]
date = "2026-11-08"
segments = ["equity", "equity_derivatives"]
kind = "muhurat"                              # muhurat | budget | other | mock
start = "<HH:MM>"
end = "<HH:MM>"
live_data = true                              # false for mock/DR sessions: never feed analytics
source = "<circular no.>"
```

Validation (`validate()`): within one segment and `applies_to` the windows of a date must not overlap and `start < end`; holiday dates unique per segment; every `effective_from` parses; a `mock` session with `live_data = true` is rejected; every `[[session]]` and `[[special_session]]` carries a `source`. `close_time(date, segment)` is the `end` of the last window effective on that date (special session first if present), converted from IST to UTC; it is the option-expiry instant and the bar cut-off. The signature has no instrument kind, so if an official circular gives expiring index contracts an earlier end than other derivatives, model it as its own segment value rather than a literal in code, and record that decision in `DECISIONS.md` (open question for the `bz-repo-structure` owner).

Staleness limit: `verified_on` older than the configured days (in `risk.toml` `[data]`) is a UI warning and a stop for order sizing.

## 2. `events.toml`

```toml
schema_version = 1
verified_on = "YYYY-MM-DD"
source = "exchange announcements, rbi.org.in, finmin"

[[event]]
date = "2026-12-05"
kind = "rbi_policy"            # results | rbi_policy | budget | expiry | ex_date | index_rebalance | special_session
symbol = ""                    # empty = market-wide; otherwise exchange + tradingsymbol
note = "plain-English label shown in the Why panel"
source = "<url or announcement id>"
```

An instrument with no event source reachable is `unknown`, never "no events". Corporate-action ex-dates also live in the store table (`bz-market-data`); `events.toml` only holds the ones the scanner and the gate need to display.

## 3. Fixtures

Fixture calendar: `fixtures/config/calendar.toml` (layout and tolerances in `bz-testing`), with fictional holidays and fictional window times so no test depends on a real circular. Include: one non-F&O and one F&O window set, an effective-dated regime change, a Saturday special session, a mock session with `live_data = false`, a holiday for one segment only, and a year outside `covered_years`.
