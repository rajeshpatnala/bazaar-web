# 5. Automation safety (`auto` and any unattended path)

(From `bz-trading-discipline` SKILL.md; open when the task touches this area.)


Owned by `bz-execution`; policy here:

1. Kill switch: global, persisted (survives restart), checked immediately before every send, and flipped automatically on reconciliation mismatch, repeated order rejects, or data staleness.
2. Daily-loss circuit breaker (G03) active before the first order of the day; it also cancels working entry orders.
3. Order-rate limiter below both the broker limit and the SEBI per-segment threshold (`bz-india-markets`); the per-minute cap is `hard_limits.max_orders_per_minute` (Hard ceiling, G16); the per-day and per-order notional caps are the `[limits]` keys named under the table.
4. The auto policy is just another approver: the full gate runs at send time, with fresh data, exactly as for a human click. No separate "fast path".
5. Limit orders only (with a marketable-limit band) for illiquid names; idempotent client order ids; reconcile positions with the broker every few minutes.
6. Every position has a protective stop at the broker or exchange side (SL order or GTT), not only inside the server, so a server crash or restart does not leave exposure unprotected.
7. First `auto` sessions run at `limits.auto_size_factor = 0.5` for the first N sessions; the owner gets a browser notification (and Telegram if enabled; WebSocket `order_update` / `gate_alert` messages feed both) for each order, fill and gate rejection.
8. Daily login/2FA is never automated. When the session expires, `auto` stops and says so.
9. No self-modifying behaviour: auto never edits `risk.toml`, strategy params or overrides, and never widens a stop.
