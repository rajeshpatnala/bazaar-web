# 8. Position sizing (risk crate)

(From `bz-backtest-risk` SKILL.md; open when the task touches this area.)


Risk rupees divided by stop distance, floored to whole lots, with a cost and slippage buffer so a stopped-out trade cannot exceed the limit.

```rust
pub struct SizingInput {
    pub capital: Decimal,
    pub risk_pct: Decimal,            // e.g. 1 means 1 percent
    pub side: Side,
    pub entry: Decimal,
    pub stop: Decimal,
    pub lot_size: u32,                // 1 for cash equity, from the instrument master otherwise
    pub buffer_per_unit: Decimal,     // slippage + expected per-unit charges at the stop, >= 0
}

pub struct Sized { pub qty: u64, pub lots: u64, pub risk_rupees: Decimal, pub risk_budget: Decimal }

pub fn size_position(i: &SizingInput) -> Result<Sized, SizingError> {
    if i.lot_size == 0 || i.capital <= Decimal::ZERO || i.buffer_per_unit < Decimal::ZERO {
        return Err(SizingError::InvalidInput);
    }
    let stop_ok = match i.side { Side::Buy => i.stop < i.entry, Side::Sell => i.stop > i.entry };
    if !stop_ok { return Err(SizingError::StopOnWrongSide); }

    let budget = i.capital * i.risk_pct / dec!(100);
    let per_unit = (i.entry - i.stop).abs() + i.buffer_per_unit;
    let lot = Decimal::from(i.lot_size);
    let mut lots = (budget / (per_unit * lot)).floor().to_u64().ok_or(SizingError::Overflow)?;
    // Decimal division rounds at 28 digits; re-check so rounding can never push us over budget.
    while lots > 0 && Decimal::from(lots) * lot * per_unit > budget { lots -= 1; }
    if lots == 0 { return Err(SizingError::BudgetBelowOneLot { budget, one_lot_risk: per_unit * lot }); }
    let qty = lots * u64::from(i.lot_size);
    Ok(Sized { qty, lots, risk_rupees: Decimal::from(qty) * per_unit, risk_budget: budget })
}
```

- Options: a bought option's risk is premium x qty (defined); size from the premium stop or the full premium, whichever the strategy spec says. Spreads and condors: risk is the structure's maximum loss from the payoff engine (`bz-quant`), not a stop distance. Naked short options are never sized automatically; they need the margin and worst-case warning path in `bz-trading-discipline`.
- Cash and margin cap: after sizing, cap by available cash (equity) or by margin from the broker margin API (F&O). If margin cannot be fetched, F&O sizing returns an error; never estimate margin by hand.
- `BudgetBelowOneLot` surfaces to the user as "this setup needs more capital than your risk limit allows" with the rupee figures.
- Portfolio limits (max open positions, total open risk, sector concentration, correlation, daily loss) are gate rules; sizing only enforces the per-trade budget.
