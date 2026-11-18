# 8. Tests and verification

(From `bz-trading-discipline` SKILL.md; open when the task touches this area.)


- **Per rule:** boundary tests just below, at and above the limit (G02: one rupee under/over budget; G03: -2.99%, -3.00%, -3.01%), table-driven, using `rust_decimal_macros::dec!`. Use a fake `Clock` for cool-off, event days, expiry cutoff and next-session lockout release.
- **Properties (`proptest`):** no sequence of fills and quotes lets sized risk exceed the budget; gate is pure (same input, same verdict); stop updates never raise ₹ risk; lockout holds until the clock crosses into the next session; override cap never exceeded.
- **Fail-closed tests:** stale ban list, missing quote, empty events file, corrupt config each produce `Reject`.
- **Config tests:** the committed `risk.toml` parses, validates, matches the documented defaults (`insta` snapshot), and the loader refuses values above `[hard_limits]`, `allow_stop_widening = true` and `allow_averaging_down = true`.
- **Pipeline tests:** with a mock `Executor`, assert `Executor::place` is never called when the gate rejects, and that the gate is called a second time at approval; ephemeral in-memory SQLite for the audit log, kill switch persistence across a simulated restart, hash-chain verification.
- **Readiness tests:** synthetic ledgers on each side of every threshold. No live API, no real broker, no secrets; connection status only via `just db-check`.
- **UI:** override dialog cannot confirm without a reason; gate reasons rendered in plain English with ₹ first (`bz-web-ui`, `bz-testing`).
