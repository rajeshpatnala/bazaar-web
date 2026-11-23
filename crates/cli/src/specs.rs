//! Instrument specs for the paper executor (BZ-521 S1): `InstrumentSnapshot` is a `SpecSource`
//! built ONCE at startup from the instrument master for the watchlist symbols.
//!
//! - NSE `EQ` cash instruments only; lot, tick and kind come from the master, the freeze quantity
//!   is `None` because the master carries none. Nothing is defaulted: a symbol that is missing,
//!   not NSE EQ, a derivative, or has a zero tick or lot has NO spec (`None`, order refused).
//! - The snapshot is only built while the master is fresh by the SAME rule as
//!   `check-connections` (`commands::instruments_state`); a stale, never-fetched or
//!   calendar-less master gives an empty snapshot and a warning.
//! - The port is sync, so the snapshot is read-only data: `spec()` never touches the database.
//! - More symbols than [`MAX_SPECS`] gives an empty snapshot and a warning, never a partial one.
//!
//! Warnings are returned for the caller (S2) to log once; this crate has no logger.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use bazaar_core::clock::Clock;
use bazaar_core::config::{self, MAX_SYMBOLS_PER_WATCHLIST, WatchlistsConfig};
use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::ports::InstrumentRepo;
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_paper::executor::SpecSource;
use bazaar_paper::fill::InstrumentSpec;
use rust_decimal::Decimal;

use crate::commands::instruments_state;

/// Most distinct watchlist symbols loaded; more is "load nothing" (and a warning).
pub const MAX_SPECS: usize = MAX_SYMBOLS_PER_WATCHLIST;

/// Read-only specs by trading symbol.
#[derive(Debug, Clone, Default)]
pub struct InstrumentSnapshot {
    specs: HashMap<String, InstrumentSpec>,
}

impl InstrumentSnapshot {
    /// A snapshot with no instruments: every order is refused.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Number of instruments with a spec.
    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// True when no instrument has a spec.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Build the snapshot for the symbols of every watchlist in `config_dir`. Never fails: any
    /// problem gives fewer (or no) specs plus a warning line for the caller to log.
    pub async fn load<R: InstrumentRepo>(
        repo: &R,
        config_dir: &Path,
        clock: &dyn Clock,
    ) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        if let Err(cat) = instruments_state(repo, config_dir, clock).await {
            warnings.push(format!("instrument specs unavailable: master {cat}"));
            return (Self::empty(), warnings);
        }
        let Ok(lists) = config::load::<WatchlistsConfig>(config_dir) else {
            warnings.push("instrument specs unavailable: watchlists unreadable".to_owned());
            return (Self::empty(), warnings);
        };
        let symbols: BTreeSet<&str> = lists
            .watchlist
            .iter()
            .flat_map(|w| w.symbols.iter().map(String::as_str))
            .collect();
        if symbols.len() > MAX_SPECS {
            warnings.push(format!(
                "instrument specs unavailable: more than {MAX_SPECS} watchlist symbols"
            ));
            return (Self::empty(), warnings);
        }
        let mut specs = HashMap::new();
        let mut skipped = 0_usize;
        for sym in symbols {
            match spec_for(repo, sym).await {
                Some(spec) => {
                    specs.insert(sym.to_owned(), spec);
                }
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            warnings.push(format!(
                "{skipped} watchlist symbol(s) have no usable NSE equity spec and will be refused"
            ));
        }
        (Self { specs }, warnings)
    }
}

/// The spec of one symbol, or `None` for anything but a valid NSE `EQ` cash instrument.
async fn spec_for<R: InstrumentRepo>(repo: &R, symbol: &str) -> Option<InstrumentSpec> {
    let row = repo.get(&Exchange::Nse, symbol).await.ok().flatten()?;
    let kind = if repo.is_cash_fno(symbol).await.ok()? {
        InstrumentKind::CashFno
    } else {
        InstrumentKind::CashNonFno
    };
    map_row(&row, symbol, kind)
}

fn map_row(row: &Instrument, symbol: &str, kind: InstrumentKind) -> Option<InstrumentSpec> {
    let equity = row.exchange == Exchange::Nse
        && row.instrument_type == InstrumentType::Equity
        && row.tradingsymbol == symbol
        && row.expiry.is_none()
        && row.strike.is_none();
    if !(equity && row.lot_size > 0 && row.tick_size > Decimal::ZERO) {
        return None;
    }
    Some(InstrumentSpec {
        segment: Segment::Equity,
        kind,
        lot_size: row.lot_size,
        tick_size: row.tick_size,
        freeze_qty: None,
    })
}

impl SpecSource for InstrumentSnapshot {
    fn spec(&self, symbol: &str) -> Option<InstrumentSpec> {
        self.specs.get(symbol).cloned()
    }
}

#[cfg(test)]
mod tests;
