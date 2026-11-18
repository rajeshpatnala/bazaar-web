# 7. Corporate actions

(From `bz-market-data` SKILL.md; open when the task touches this area.)


Kite gives no adjustment data. Corporate actions live in the store (SQLite table below), never in a config file. Sources, in order: a user-reviewed CSV loaded with `bazaar-cli import-actions <csv>` (idempotent upsert on the unique key, with an `ImportReport`), then official exchange corporate-action files if their terms allow download. Every row carries `source` and `verified_on`; nothing is typed in from memory.

```sql
corporate_actions(id, isin, exchange, symbol, ex_date, kind,        -- split | bonus | dividend | rights | demerger
                  ratio_num, ratio_den, amount_text, source, verified_on,
                  UNIQUE(exchange, symbol, ex_date, kind));
```

- Per-action price factor applied to bars **before** `ex_date`: split `old:new` -> `old/new` of prior price (split 1->5 gives 0.2); bonus `a` new for `b` held -> `b/(a+b)`; cash dividend -> `1 - D / close_before_ex` (ratio method). Volume factors are the inverse for splits and bonuses. Rights, demergers, mergers: manual factor entry only, flagged `needs_review`.
- Cumulative factor at bar date `t` = product of factors for all actions with `ex_date > t`. Build the factor table once per symbol and cache; apply in the read path (`AdjustedCandles` view), never mutate stored raw bars.
- Cross-check: after each import, flag any overnight move beyond a configured threshold with no action row (probable unrecorded split/bonus). Report, do not auto-fix.
- Key by ISIN where available (symbols change); keep a `symbol_history` mapping.
- Backtests must declare adjusted or raw data in their config; `bz-backtest-risk` uses the adjusted view by default.
