# Zerodha equities charges (pasted by owner 2026-11-20)

Source: https://zerodha.com/charges/#tab-equities (owner copied the page text; verified_on 2026-11-20). No contract note supplied yet.

| | Equity delivery | Equity intraday | F&O - Futures | F&O - Options |
|---|---|---|---|---|
| Brokerage | Zero | 0.03% or Rs 20/executed order, whichever is lower | 0.03% or Rs 20/executed order, whichever is lower | Flat Rs 20 per executed order |
| STT/CTT | 0.1% on buy & sell | 0.025% on the sell side | 0.05% on the sell side | 0.15% of intrinsic value on options bought and exercised; 0.15% on sell side (on premium) |
| Transaction charges | NSE 0.00307%; BSE 0.00375% | NSE 0.00307%; BSE 0.00375% | NSE 0.00183%; BSE 0 | NSE 0.03553% (on premium); BSE 0.0325% (on premium) |
| GST | 18% on (brokerage + SEBI charges + transaction charges) | same | same | same |
| SEBI charges | Rs 10 / crore | Rs 10 / crore | Rs 10 / crore | Rs 10 / crore |
| Stamp charges (buy side) | 0.015% or Rs 1500 / crore | 0.003% or Rs 300 / crore | 0.002% or Rs 200 / crore | 0.003% or Rs 300 / crore |

Not in the owner's paste (seen only in an agent web fetch, UNVERIFIED): DP charge Rs 15.34 per scrip on sale (3.5 CDSL + 9.5 Zerodha + 2.34 GST); delivery min Rs 0.01 per contract note; physically settled F&O rates.
# Zerodha "Charges explained" (pasted by owner 2026-11-20; source https://zerodha.com/charges/#tab-equities)

Relevant to config (verbatim facts):
- STT/CTT: equity delivery both buy and sell; intraday and F&O sell side only.
- IPFT (NSE): Equity and Futures Rs 0.01 per crore + GST of traded value. Options Rs 0.01 per crore + GST on premium value. Currency Rs 0.05 per lakh (futures), Rs 2 per lakh (options premium) + GST.
- GST: 18% of (brokerage + SEBI charges + transaction charges).
- SEBI: Rs 10 per crore + GST.
- DP charges: Rs 15.34 per scrip (Rs 3.5 CDSL + Rs 9.5 Zerodha + Rs 2.34 GST), charged on the trading ledger when stocks are sold, irrespective of quantity. Female first holder: Rs 0.25 off the CDSL fee; MF/bond debit transactions: extra Rs 0.25 off.
- Delivery: minimum Rs 0.01 per contract note. Physical delivery contracts 0.25% of contract value; netted-off physically settled 0.1%. Companies/partnerships/trusts/HUF: 0.1% or Rs 20 delivery brokerage.
- Stamp: per Indian Stamp Act, buy side (rates in the table file).
- Not modelled (out of scope for F2): call & trade Rs 50, NRI brokerage, debit-balance Rs 20/order, MTF, pledge, AMC, corporate-action, off-market, CMR, payment gateway, delayed payment, 3-in-1.
