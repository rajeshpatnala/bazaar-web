# 1. Where each fact comes from

(From `bz-india-markets` SKILL.md; open when the task touches this area.)


| Fact | Source of truth | Refresh | Never |
|---|---|---|---|
| Lot size, tick size, expiry dates, strikes, F&O eligibility | Broker instrument master (Kite `GET /instruments`, CSV; `bz-market-data`) | daily, before the first scan | hardcode `NIFTY_LOT = 75`, or compute "last Tuesday" |
| Trading holidays, special sessions | `config/calendar.toml` filled from the exchange holiday calendar | yearly + on every exchange circular | `weekday < 5` as "market open" |
| Session timings | `config/calendar.toml` (`verified_on`) | on every exchange timing circular | literal `15:30` in logic |
| F&O ban list | exchange daily "securities in ban period" file | daily, pre-open | cache across days |
| Price bands / circuits | exchange daily price-band file + live tick circuit fields | daily | assume 20% |
| Corporate actions, result dates | exchange corporate-announcement archives / vendor / `config/events.toml` | daily | scrape a news site |
| Charges | `config/charges.toml` from broker charges page + circulars | quarterly + on circulars | numbers from memory |
| Margin | broker margin API | per order | estimate by hand |

Official places (verify they still resolve; exchanges reorganise pages):
- NSE holiday calendar (trading and clearing holidays are separate lists): https://www.nseindia.com/resources/exchange-communication-holidays
- NSE circulars (timings, expiry, lot-size changes), PDFs under https://nsearchives.nseindia.com/content/circulars/
- Bhavcopy (EOD): NSE archives host the UDiFF common bhavcopy zips (file names and folders are reported by third parties and change; the current list is at https://www.nseindia.com/static/resources/forms-formats-members). Legacy pre-UDiFF files use other names and column sets; retirement dates of older paths are unverified, confirm at the NSE circular list. URL templates live in `config/data.toml` (`bz-market-data`). BSE equivalent: https://www.bseindia.com/static/members/udiff.aspx
- Kite instruments dump: https://kite.trade/docs/connect/v3/market-data-and-instruments
- Zerodha charges: https://zerodha.com/charges

If a source is unreachable or a file is older than its refresh window, the app shows "data stale" and `just db-check` reports the instrument-master freshness. It never silently falls back to a remembered value.
