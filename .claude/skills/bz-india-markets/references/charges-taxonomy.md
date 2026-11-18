# 7. Charges taxonomy (no numbers here)

(From `bz-india-markets` SKILL.md; open when the task touches this area.)


`config/charges.toml`, one table per segment and side (equity delivery, equity intraday, equity futures, equity options, plus currency/commodity if enabled), each with `verified_on` and `source`:
- brokerage (flat or percentage, with caps; per executed order);
- STT (equities, futures, options) or CTT (commodities): charged on a defined side and base (turnover, option premium, or intrinsic value on exercise);
- exchange transaction charges (rate differs by exchange and segment; NSE vs BSE are not the same);
- SEBI turnover fee;
- stamp duty (buy side, state-uniform, by segment);
- GST on brokerage, exchange charges and SEBI fee (not on STT or stamp duty);
- DP charges on delivery sells (per scrip per day, plus depository fee), investor-protection fund charge where applicable.

Rules:
- Rates live only in `config/charges.toml` (schema: `bz-backtest-risk` `references/charges-toml.md`), filled from https://zerodha.com/charges and exchange/SEBI/finance-ministry notices at implementation time, each with `verified_on`. Statutory rates (for example STT) changed in recent budgets and third-party summaries conflict: unverified, confirm at the Finance Act notification or the broker charges page. Record the source in `DECISIONS.md`. No rate appears in skill text or code.
- A calculator that cannot find a rate for a segment/side returns an error, never zero.
- Reference-value tests use a published worked example from the broker's calculator for each segment (store input, expected components, and `verified_on` in the fixture).
- Tax (STCG/LTCG on equity, business income on F&O, audit thresholds, loss carry-forward) is out of scope unless asked; point users to the Income Tax Department and a chartered accountant and never compute tax liability in the app.
