//! Test-only helper every indicator slice must call: the generic invariants of the `Indicator`
//! contract (length, determinism, no look-ahead, reset equals fresh, warm-up).

use crate::{Indicator, batch};

/// Check the contract for `make()` over `xs`. Panics (test-only) on violation.
///
/// - output length equals input length and the first `warmup()` outputs are `None`, the rest `Some`
///   (`expect_all_some_after_warmup`; pass false for indicators that may yield `None` later);
/// - two fresh instances agree (determinism);
/// - every prefix of `xs` yields the prefix of the full output (no look-ahead);
/// - after `reset`, the instance reproduces a fresh run.
pub(crate) fn check_contract<I>(
    make: impl Fn() -> I,
    xs: &[I::Input],
    expect_all_some_after_warmup: bool,
) where
    I: Indicator,
    I::Input: Clone,
    I::Output: PartialEq + std::fmt::Debug,
{
    let full = batch(make(), xs.iter().cloned());
    assert_eq!(full.len(), xs.len(), "length");
    assert_eq!(full, batch(make(), xs.iter().cloned()), "determinism");
    let warm = make().warmup();
    for (i, o) in full.iter().enumerate() {
        if i + 1 < warm {
            assert!(o.is_none(), "None during warm-up at {i}");
        } else if expect_all_some_after_warmup {
            assert!(o.is_some(), "Some after warm-up at {i}");
        }
    }
    for n in 0..=xs.len() {
        let pre = batch(make(), xs[..n].iter().cloned());
        assert_eq!(pre[..], full[..n], "prefix {n} differs: look-ahead");
    }
    let mut reused = make();
    for x in xs {
        reused.update(x.clone());
    }
    reused.reset();
    let again: Vec<_> = xs.iter().cloned().map(|x| reused.update(x)).collect();
    assert_eq!(again, full, "reset equals fresh");
}
