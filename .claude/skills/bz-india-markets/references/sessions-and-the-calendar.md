# 3. Sessions and the calendar

(From `bz-india-markets` SKILL.md; open when the task touches this area.)


Session structure is **in flux** and this skill states no session time as fact. What was reported as of 2026-11-18, each item to be checked by a human against the primary source before it goes into `calendar.toml` (primary sources: the NSE closing auction page https://www.nseindia.com/static/products-services/closing-auction-session, NSE circulars at https://www.nseindia.com/resources/exchange-communication-circulars with PDFs under https://nsearchives.nseindia.com/content/circulars/, SEBI circulars at https://www.sebi.gov.in/legal/circulars.html):
- Closing Auction Session (CAS) for F&O stocks: reported to start 2026-08-03 with equity derivatives running 10 minutes later than before; verify at NSE/CMTR/74466 and NSE/FAOP/74467 (dated 2026-05-29, as cited by broker coverage). Non-F&O stocks reportedly keep the old close.
- Reported, verify at the NSE circulars of 2026-11-06 (one cash-market and one F&O circular, numbers not confirmed by this pack; locate them in the circular list): effective 2026-12-07 the CAS window moves later and shorter, equity derivatives (stock and non-expiring index) run a few minutes past the current close, and expiring index contracts stop earlier than other derivatives; revised treatment of some unexecuted orders reportedly starts separately on 2027-02-08. A broker article (https://www.kotakneo.com/news/regulations/nse-new-closing-auction-and-f-and-o-trading-rules-from-7-december/) gave exact times; other coverage disagreed, so no time is copied here.
- SEBI circular of October 2026 on reviewing CAS, market timings and derivatives settlement methodology (https://www.sebi.gov.in/sebi_data/attachdocs/oct-2026/1792079571420.pdf; title seen, content not verified by this pack): confirm whether it overrides any NSE item above.
- Pre-open order-entry split from 2026-09-07: reported by a broker note citing NSE/CMTR/74969 and NSE/FAOP/74970; verify there.
- Whether the old post-close session still runs under the new regime: unverified, confirm at the NSE circulars before modelling it.
- Zerodha square-off times for MIS, broker cut-offs and GTT rules are broker facts: take them from the broker, not this file.

Because dates differ by regime, `calendar.toml` windows are effective-dated; a circular adds rows, it never edits history.

Engineering consequences:
- The `MarketCalendar` (`bz-repo-structure`) answers `session_at(ts, segment, kind)`, `is_trading_day(date, segment)`, `next_trading_day` and `close_time(date, segment)`. Close time depends on the segment, the instrument kind and the date; all of it comes from `calendar.toml`.
- **Option expiry instant = `calendar.close_time(expiry_date, segment, kind)`**, never a literal 15:30 or 15:40; time-to-expiry, bar cut-offs and expiry-day rules all read it.
- Special sessions exist: Muhurat trading on Diwali (timing announced by circular), Saturday/Sunday Budget or special sessions, and Saturday mock/DR sessions that must NOT feed live data into analytics. Model the calendar as explicit dated entries, not "Mon-Fri minus holidays".
- A daily bar's "close" is the closing-price method of that day (auction price versus the earlier VWAP-based method; the switch date is per instrument kind and comes from `calendar.toml` regimes). Backtests spanning the change need a per-date flag; label the discontinuity instead of hiding it.
- The bar builder must not assume a 15:30 final tick. Bars are cut from the calendar's session end for that instrument.
- Settlement for equities is T+1 (verify on the exchange site); T+1 matters for paper-trade cash availability (`bz-execution`).
- Segment calendars differ (equity, derivatives, currency, commodity, SLB; trading vs clearing holidays). Key holidays by segment.

Time-of-day behaviour (context for strategy logic and Why? text, not rules):
- First 15 minutes: wide spreads, gaps resolve, false breakouts common; ORB strategies live here, beginners should wait.
- Midday: lower volume, range-bound drift; trend strategies underperform, mean reversion does better.
- Last hour: positioning and trend continuation; MIS square-off flows near the close.
- Expiry-day afternoon: gamma and theta dominate, premiums collapse or spike within minutes, wide option spreads, pinning near large-OI strikes. Defined-risk only; flag it in the UI.
