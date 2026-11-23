# Known pitfalls (read before coding; <= 15 lines)

Patterns reviewers flagged again and again (from ~160 blocker/major findings, PRs 113-137). The manager updates this after
each feature's efficiency review: add what keeps recurring, drop what stopped. Last update: 2026-11-23.

1. Failure paths: every write/append has a tested failure path. No `let _ =` on a Result. When step 2 fails after step 1
   wrote (audit row, trade line, charge), reverse step 1 or record it, and return both errors if the reversal fails too.
2. Bounded resources: no unbounded channels/queues/retries; a timed-out write must not land later.
3. Safety by type: guarded entry points take proof types (`ApprovedOrder`, gate verdict), never a raw `OrderIntent`;
   no safety-relevant plain `bool` a caller can forget (`circuit_locked`): use an enum that names the data source / Unknown.
4. Unknown fails closed: missing data, config or a 0 threshold blocks or warns; it never silently passes as "fine".
5. Tests prove the reason: a refusal test asserts WHICH rule refused; a money test checks every field that moves
   (cash, charges, position, audit row), and the double-failure case.
6. No panicking maths on money/ticks: checked add/sub/div; no unchecked `raw / tick`.
7. Exits vs blocks: kill switch, breaker and rate limiter must treat reduce-only exits consistently, as the plan says.
8. Every config value has bounds and is tied to its `config/*.toml` key; caller-supplied amounts get an upper bound.
