# Zerodha brokerage-calculator results (pasted by owner 2026-11-20; source zerodha.com/brokerage-calculator)

All NSE unless stated. Quantity 1 used for #4 and #5 (owner found no lot option), so these are per unit, not per lot.
Totals use the rounded STT and stamp shown. DP charge is NOT included in delivery totals.

| # | case | buy / sell / qty | turnover | brokerage | STT | exch txn | GST | SEBI | stamp | total |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | equity intraday NSE | 500 / 510 / 100 | 101000 | 30.3 | 13 | 3.1 | 6.03 | 0.1 | 2 | 54.53 |
| 2 | equity delivery NSE | 500 / 510 / 100 | 101000 | 0 | 101 | 3.1 | 0.58 | 0.1 | 8 | 112.78 |
| 3 | equity intraday BSE | 500 / 510 / 100 | 101000 | 30.3 | 13 | 3.79 | 6.15 | 0.1 | 2 | 55.34 |
| 4 | futures NSE | 24000 / 24100 / 1 | 48100 | 14.43 | 12 | 0.88 | 2.76 | 0.05 | 0 | 30.12 |
| 5 | options NSE | 100 / 120 / 1 | 220 | 40 | 0 | 0.08 | 7.21 | 0 | 0 | 47.29 |

Net P&L shown: #1 945.47, #2 887.22, #3 944.66, #4 69.88, #5 -27.29.

## Manager's check against config/charges.toml (arithmetic, 2026-11-20)
- All five reproduce from the shipped rates, within rounding.
- GST = 18% x (brokerage + exchange txn + SEBI); STT, stamp and IPFT are NOT in the GST base (settles QUAN-66-1 for the calculator; IPFT is not shown at all).
- Rounding seen: STT and stamp are rounded to the whole rupee (12.75 -> 13, 1.5 -> 2, 7.5 -> 8, 0.48 -> 0, 0.18 -> 0, 12.05 -> 12). Brokerage, exchange txn and GST to 2 decimals.
- SEBI: 0.101 -> 0.1, 0.0481 -> 0.05, 0.022 -> 0 (and GST in #5 excludes it). Hypothesis: rounded to the nearest 0.05. UNVERIFIED; cost-engine tests should allow 0.05 tolerance on SEBI.
- Intraday/futures brokerage is applied per side (0.03% of each leg), then summed; #5 options is Rs 20 per leg.
- The calculator is an estimate; the cost engine should match within Rs 0.05 per line, not exactly. Not a contract-note check.
