# 4. Expiries, lots and ticks

(From `bz-india-markets` SKILL.md; open when the task touches this area.)


Expiry schedules are data from the instrument master, not rules in code. Reported as of 2026-11-18 (broker and news coverage; unverified here): index options expire on a Tuesday (NSE) or Thursday (BSE), weekly expiries are limited to one benchmark index per exchange, and an expiry that falls on a trading holiday moves to the previous trading day, possibly cascading over consecutive holidays. Verify at the NSE contract-specification pages, the NSE circular list and the SEBI circular on expiry days (confirm the circular number there), plus https://www.nseindia.com/resources/exchange-communication-holidays; schedules have changed several times in two years. Tests must derive shifts from a fixture calendar, never from a remembered weekday.

Implementation:
- Expiries come from the instrument master: group `instrument_type in (CE, PE, FUT)` rows by `name` and `expiry`, sort, label weekly vs monthly by whether another expiry for the same underlying falls later in the same calendar month. Never derive them from weekday arithmetic.
- `days_to_expiry` uses the calendar (trading days and calendar days both, labelled). Time to expiry for option pricing uses the actual expiry instant, `calendar.close_time(expiry_date, segment, kind)`.
- Lot size and tick size are per instrument and change by circular (lot sizes are revised periodically, tick size is 0.05 for most but can differ). A lot-size change mid-series produces different lot sizes for different expiries of the same underlying; keep lot size on the instrument row, never on the underlying.
- F&O eligibility = presence in the instrument master's NFO segment today. Backtests need the historical eligibility list; if it is unavailable say so ("survivorship caveat", `bz-backtest-risk`).
- Freeze limits (max quantity per order) and strike steps change too; read them from the exchange file or reject when unknown.
