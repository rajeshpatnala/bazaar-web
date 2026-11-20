//! Provenance of a result: config hashes, the charges `verified_on` range, data range, universe
//! and the trial count with its penalty (F9 acceptance 6; policy section 4).

use std::collections::{BTreeMap, HashSet};

use bazaar_core::config::ChargesConfig;
use chrono::NaiveDate;
use sha2::{Digest, Sha256};

use crate::config::Trials;
use crate::types::{BacktestError, HistoryPolicy, Provenance};
use crate::walkforward::WalkForwardResult;

/// Lower-case hex SHA-256 of `bytes`.
#[must_use]
pub fn hash_hex(bytes: &[u8]) -> String {
    // sha2 0.11 digests do not implement `LowerHex`, so encode by nibble.
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    out
}

/// Earliest and latest `verified_on` over the WHOLE charges file (file, GST, DP, option exercise
/// and every rate set), not only the sets a run used; use it for staleness warnings. The range of
/// what a run was charged with is `CostModel::used_verified_range` (pass it as `charges_used`).
/// `None` when the file carries no date at all.
#[must_use]
pub fn charges_file_verified_range(cfg: &ChargesConfig) -> Option<(NaiveDate, NaiveDate)> {
    let dates: Vec<NaiveDate> = cfg
        .verified_on
        .into_iter()
        .chain(cfg.gst.as_ref().map(|g| g.verified_on))
        .chain(cfg.dp.as_ref().map(|d| d.verified_on))
        .chain(cfg.option_exercise.as_ref().map(|o| o.verified_on))
        .chain(cfg.rate_set.iter().map(|r| r.verified_on))
        .collect();
    Some((*dates.iter().min()?, *dates.iter().max()?))
}

/// What a run knows when it finishes.
#[derive(Debug, Clone)]
pub struct ProvenanceInputs<'a> {
    /// `(file name, raw bytes)` of every config file used.
    pub config_files: &'a [(&'a str, &'a [u8])],
    pub charges: &'a ChargesConfig,
    pub data_from: NaiveDate,
    pub data_to: NaiveDate,
    pub history_policy: HistoryPolicy,
    pub universe: &'a [String],
    /// `(strategy id, parameter set)` evaluated in-sample, in order; repeats count once. A fixed
    /// parameter run still counts as one trial.
    pub parameter_sets: &'a [(String, String)],
    pub trials: &'a Trials,
    /// Plain or walk-forward; a walk-forward run cannot be built without its evidence text.
    pub kind: RunKind<'a>,
    /// Earliest and latest `verified_on` of what the run was charged with, from
    /// `CostModel::used_verified_range`. Required: there is no whole-file fallback, so a runner
    /// cannot claim the weaker range by omission. A run with no legs has no range to record and
    /// cannot build provenance.
    pub charges_used: (NaiveDate, NaiveDate),
}

/// The `gate_evidence` text of a walk-forward result. It has no public constructor: the only way
/// to get one is [`RunKind::of`], so free text cannot be passed as walk-forward evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceText<'a>(&'a str);

/// What kind of run the provenance describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind<'a> {
    /// A single run: no walk-forward evidence.
    Plain,
    /// A walk-forward run, carrying the `gate_evidence` text (`walk_forward_pass` was assumed true
    /// in OOS windows). Build it with [`RunKind::of`] so the text comes from the result.
    WalkForward(EvidenceText<'a>),
}

impl<'a> RunKind<'a> {
    /// The walk-forward kind of `r`, with the text taken from the result itself.
    #[must_use]
    pub const fn of(r: &'a WalkForwardResult) -> Self {
        Self::WalkForward(EvidenceText(r.gate_evidence))
    }
}

/// Build the provenance. The universe is sorted and de-duplicated, hashes are ordered by file
/// name (deterministic); `costs_approximate` follows the history policy.
///
/// # Errors
/// `Invalid` if the charges file has no `verified_on` date or the data or charges range is reversed;
/// `Overflow` if the parameter set count does not fit.
pub fn build(i: &ProvenanceInputs<'_>) -> Result<Provenance, BacktestError> {
    if i.data_from > i.data_to {
        return Err(BacktestError::Invalid("data range is reversed".into()));
    }
    charges_file_verified_range(i.charges)
        .ok_or_else(|| BacktestError::Invalid("charges file has no verified_on date".into()))?;
    let (from, to) = i.charges_used;
    if from > to {
        return Err(BacktestError::Invalid("charges range is reversed".into()));
    }
    let gate_evidence = match i.kind {
        RunKind::Plain => None,
        RunKind::WalkForward(EvidenceText(g)) => Some(g),
    };
    let mut seen = HashSet::new();
    let mut sets: Vec<String> = Vec::new();
    for (strategy, params) in i.parameter_sets {
        if seen.insert((strategy, params)) {
            sets.push(format!("{strategy}: {params}"));
        }
    }
    let distinct = u32::try_from(sets.len()).map_err(|_| BacktestError::Overflow("trial count"))?;
    let in_run = distinct.max(1);
    let mut universe = i.universe.to_vec();
    universe.sort();
    universe.dedup();
    let config_hashes: BTreeMap<String, String> = i
        .config_files
        .iter()
        .map(|(n, b)| ((*n).to_owned(), hash_hex(b)))
        .collect();
    Ok(Provenance {
        config_hashes,
        charges_verified_from: Some(from),
        charges_verified_to: Some(to),
        data_from: Some(i.data_from),
        data_to: Some(i.data_to),
        history_policy: i.history_policy,
        costs_approximate: i.history_policy == HistoryPolicy::LatestForAll,
        universe,
        parameter_sets: sets,
        trial_penalty_pf: i.trials.profit_factor_penalty(in_run),
        trials: i.trials.total_trials(in_run),
        gate_evidence: gate_evidence.map(str::to_owned),
    })
}

#[cfg(test)]
mod tests;
