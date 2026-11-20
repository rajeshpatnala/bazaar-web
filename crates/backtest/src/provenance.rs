//! Provenance of a result: config hashes, the charges `verified_on` range, data range, universe
//! and the trial count with its penalty (F9 acceptance 6; policy section 4).

use std::collections::BTreeMap;

use bazaar_core::config::ChargesConfig;
use chrono::NaiveDate;
use sha2::{Digest, Sha256};

use crate::config::Trials;
use crate::types::{BacktestError, HistoryPolicy, Provenance};

/// Lower-case hex SHA-256 of `bytes`.
#[must_use]
pub fn hash_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    out
}

/// Earliest and latest `verified_on` over the whole charges file (file, GST, DP, option exercise
/// and every rate set). `None` when the file carries no date at all.
#[must_use]
pub fn charges_verified_range(cfg: &ChargesConfig) -> Option<(NaiveDate, NaiveDate)> {
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
    /// Parameter sets evaluated in-sample, in order; repeats count once.
    pub parameter_sets: &'a [String],
    pub trials: &'a Trials,
}

/// Build the provenance. The universe is sorted and de-duplicated, hashes are ordered by file
/// name (deterministic); `costs_approximate` follows the history policy.
///
/// # Errors
/// `Invalid` if the charges file has no `verified_on` date or the data range is reversed;
/// `Overflow` if the parameter set count does not fit.
pub fn build(i: &ProvenanceInputs<'_>) -> Result<Provenance, BacktestError> {
    if i.data_from > i.data_to {
        return Err(BacktestError::Invalid("data range is reversed".into()));
    }
    let (from, to) = charges_verified_range(i.charges)
        .ok_or_else(|| BacktestError::Invalid("charges file has no verified_on date".into()))?;
    let mut sets: Vec<String> = Vec::new();
    for s in i.parameter_sets {
        if !sets.contains(s) {
            sets.push(s.clone());
        }
    }
    let in_run = u32::try_from(sets.len()).map_err(|_| BacktestError::Overflow("trial count"))?;
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
    })
}

#[cfg(test)]
mod tests;
