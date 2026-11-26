# S10 slice 0a part 2: unmerged backup (saved 2026-11-26)

Reference only. `cost_pool.rs` and `tests.rs` are kept as saved. They sit outside `crates/*`, so they are not compiled or tested.

- What is only here: the ledger replay and the pairing of each charge with its trade (the replay was found in no git ref).
- Stale base: taken before the PR 185 review fixes. Never copy these over `crates/paper/src/cost_pool.rs` or `crates/paper/src/cost_pool/tests.rs`; port the missing parts onto the merged files.
- The merged code has, and this backup lacks: `check()`, `ZeroAmount`, public `Pool::buy` / `Pool::sell` with error docs, and the review-fix tests (for example `a_refused_trade_names_why_and_leaves_the_pool_unchanged` and `a_partial_sale_that_ends_the_loss_never_raises_the_figure`).
- Still owed (F10 record, Deferred): SEC-185-2 (bound amounts where ledger lines are parsed) and TS-185-3 (mixed buy/sell property test).
- Delete this folder in the PR that lands part 2.
