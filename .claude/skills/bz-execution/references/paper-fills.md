# Paper fills: realistic simulation rules

Open this when writing or changing `FillSimulator`, `PaperExecutor` or `PaperAccount` (crate `paper`). The goal: a paper result that a real broker account could have achieved, and slightly pessimistic when in doubt. Costs come from `CostModel` and slippage config in `bz-backtest-risk`; do not copy formulas here.

## 1. Shape

```rust
/// Pure and deterministic: same ticks + same seed => same fills.
pub trait FillSimulator {
    fn on_tick(&mut self, tick: &Tick, book: &mut WorkingBook) -> Vec<SimFill>;
    fn on_clock(&mut self, now: Ts, book: &mut WorkingBook) -> Vec<SimFill>;   // square-off, expiry of DAY validity
}
```

`PaperExecutor::place` validates, checks funds/margin, and puts the order into the `WorkingBook` (state `Working`). Fills happen only in `on_tick`/`on_clock`, driven by the same `MarketData` ticks the UI sees (live ticks in the app, fixture ticks in tests). `place` never fills instantly.

## 2. Rules

| Case | Rule |
|---|---|
| Market order | Fills on the **next** tick after acceptance, at that tick's price adjusted by slippage against the trader (buy higher, sell lower). Never at the price the user saw when clicking |
| Slippage | bps from `config` per segment, wider for illiquid options (use spread, OI and volume buckets); seeded random component allowed, always against the trader on average; never negative |
| Limit buy / sell | Fills only when the market **trades through** the limit (last trade strictly below a buy limit, strictly above a sell limit) or quotes cross it. Touching the price is not a fill. When it fills, the price is the limit (no price improvement unless a crossing quote proves it) |
| SL / SL-M | Trigger when last price reaches trigger; SL-M then fills like a market order (next tick + slippage), SL becomes a limit at its limit price. Gap past the trigger fills at the gap price, not the trigger |
| Circuit | If the instrument is at its upper/lower circuit (price band from instrument master or tick flags) and there is no opposite-side liquidity, **no fill**; the order stays working. Buy orders at upper circuit and sell orders at lower circuit do not fill |
| Partial fills | Cap each fill by a fraction of the tick/bar volume (config `max_participation`) and by displayed depth when depth is available. Thin option strikes (low volume/OI bucket) fill in several parts across ticks; remaining qty stays working until validity ends. Emit `PartialFill` events; state machine goes `PartiallyFilled` |
| Quantity rules | Lot multiples, tick-size price alignment, freeze quantity: reject with `InvalidInput` exactly like a broker would. Values from instrument master, never literals |
| Validity | `DAY` orders cancel at session end; `IOC` fills what it can on the next tick then cancels the rest |
| Stale data | If the last tick is older than `stale_after_secs`, refuse to fill and flag the order; a paper fill on stale data is worse than none |
| Session | No fills outside the normal session unless the order is an AMO queued for next open (fills at the first tick with slippage). Calendar from `MarketCalendar` |
| MIS square-off | At `paper.mis_square_off_time` (IST, from config, from the broker's published time, with a `verified_on`), `on_clock` closes every open MIS position with a market-like fill (next tick + slippage + charges), emits `ExecEvent::SquareOff`, journals "auto square-off". Warn via `ExitMonitor` N minutes before. This is a simulated broker action, not an approved order, but it still writes an audit entry |

## 3. Cash and margin

1. **Funds check before acceptance**, using the ledger (`available`).
   - Cash-segment CNC buy: `qty x limit/ref price + estimated charges` must be `<= available`. Block that amount (`MarginBlock`) while the order works, release the unused part on cancel or price improvement.
   - Intraday (MIS) and F&O (NRML) orders: required margin comes from the broker's margins API (`POST /margins/orders` for single orders, `/margins/basket` for spreads, which considers existing positions). Kite's order-margins response includes the components and charges; use `total` as the block amount.
   - **If the margin cannot be fetched** (no session, plan lacks it, network down, endpoint error) the order is **blocked** with `ExecError::Unavailable("margin")`. Never fall back to a hand-written percentage, a flat per-lot number or the last known value. The UI shows "margin unavailable, order blocked".
   - Cache a margin answer for at most a few seconds and key it by the full order parameters; do not reuse across price or quantity changes.
2. On fill, replace the block with the real debit (`FillDebit`) and apply `Charge` from `CostModel` for that fill (STT/CTT, exchange, SEBI fee, stamp duty, GST, brokerage, DP charge on CNC sells when the config says so). Charges reduce cash immediately; they are never folded silently into the price.
3. F&O: premium paid/received moves cash for options; futures and short options hold margin until closed or expiry. Mark-to-market (MTM) settlement of futures is a ledger entry per day (`MtmDebit/MtmCredit`) so cash tracks real behaviour; expiry settlement of index options is cash-settled, stock derivatives physically settle (config flag; paper may simply refuse to hold stock derivatives into expiry and force-close, which must be visible in the UI).
4. Sell proceeds on CNC: availability of sale proceeds for new buys depends on the broker's rule. Model it as a config option (`cnc_proceeds = "t+1" | "immediate"`), default conservative `t+1`, with a note and `verified_on`. Delivery trades settle T+1 in India: shares bought today are in holdings but the DP/settlement rules (auction risk, BTST) are broker-specific; do not invent them, expose a config switch or refuse the case.
5. Reset/new account creates a new epoch; the old ledger stays for the journal and audit.

## 4. What must NOT happen

- A fill at a price that never traded in the window between order acceptance and fill.
- Filling the full quantity of a large order in one tick on an illiquid strike.
- Zero charges, zero slippage "for simplicity" in any mode; setting them to zero in config must print a warning banner "unrealistic paper settings".
- Using `f64` for price or cash, or the system clock inside the simulator.
- Margin estimated from memory or from a hand-written table.

## 5. Tests

Use fixture tick sequences in `fixtures/paper/*.json` (hand-built or recorded, never live). One test per rule: market fills on the next tick not the current; limit touch without trade-through does not fill, trade-through does; gap through an SL fills at the gap price; circuit tick leaves the order working; illiquid fixture gives >= 2 partial fills summing to qty; margin fetch failure blocks with `Unavailable`; MIS position closes at the configured time using a `FakeClock`; charges lower cash by the `CostModel` amount; same seed gives identical fills. Property tests: total filled <= ordered, cash never negative, no fill outside the tick price range seen since acceptance.
