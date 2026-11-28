//! Pure day and week P&L over the ledger lines of all paper sessions (BZ-F10-S10 slice 0b; unused
//! until the P&L source is wired).
//!
//! Figures are computed per session by [`session_figures`], then added up by [`shared_figures`].
//! For a window (today, or since the IST Monday):
//! - P&L = results of the sales dated in the window + the open losses of the pools held now.
//!   Opening lines are never P&L, and charges count only through those two terms.
//! - A session's equity = its cash at the window start (opening line, even if dated inside the
//!   window, plus earlier lines) + the share cost of the pools open then, without their unsold
//!   buy charges.
//! - The shared base = the smallest of the starting cash and the equity of the active session and
//!   of every session with lines in the window. So a session that is not active and has no line
//!   in the window does not lower that window's base.
//!
//! Every failure is a [`PnlError`]; no figure is ever replaced by zero. The lines must be the
//! whole ledger in id order.

use std::collections::BTreeMap;

use chrono::{Datelike, Days, NaiveDate};
use rust_decimal::Decimal;
use thiserror::Error;

use crate::account::LedgerKind;
use crate::cost_pool::replay::{LineFault, ReplayError, ReplayLine, date_of, replay_pools};
use crate::cost_pool::{PaperSessionId, PoolError};

/// Largest magnitude any sum may reach, in rupees (1e15: 10,000 lines of 1e11). With at most
/// four fraction digits per term, sums within it are exact.
pub const MAX_SUM: Decimal = Decimal::from_parts(2_764_472_320, 232_830, 0, false, 0);

/// Why there are no figures. `line` is the 0-based position of the offending line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PnlError {
    #[error(transparent)]
    Replay(#[from] ReplayError),
    #[error("ledger line {line}: a session does not start with its opening line")]
    NoOpening { line: usize },
    #[error("ledger line {line}: second opening line of a session")]
    SecondOpening { line: usize },
    #[error("ledger line {line}: dated before the line ahead of it in its session")]
    DateOrder { line: usize },
    #[error("the active session has no ledger lines")]
    NoActiveSession,
    #[error("{0} is not active but holds shares")]
    OpenOutsideActive(PaperSessionId),
    #[error("no usable mark for an open position")]
    MissingMark,
    #[error("open loss: {0}")]
    Pool(#[from] PoolError),
    #[error("sum above its bound")]
    SumTooLarge,
    #[error("week start is out of the calendar range")]
    Calendar,
    #[error("loss base is not positive")]
    BaseNotPositive,
}

/// One session's figures for one window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionWindow {
    /// Results of the sales dated in the window.
    pub realised: Decimal,
    /// Open losses of the pools held now; zero or less.
    pub open_loss: Decimal,
    /// Equity at the window start.
    pub equity: Decimal,
    /// Whether the session has any line dated in the window.
    pub has_lines: bool,
}

/// One session's day and week.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionFigures {
    pub day: SessionWindow,
    pub week: SessionWindow,
}

/// The figures of every session in the ledger. Only [`session_figures`] builds it, so the shared
/// figures cannot be taken over a chosen subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sessions {
    active: PaperSessionId,
    by_session: BTreeMap<PaperSessionId, SessionFigures>,
}

impl Sessions {
    /// One session's figures, if it has lines.
    #[must_use]
    pub fn get(&self, session: PaperSessionId) -> Option<&SessionFigures> {
        self.by_session.get(&session)
    }
}

/// P&L of all sessions together for one window, and the base its loss limit is measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPnl {
    pub pnl: Decimal,
    pub equity_base: Decimal,
}

/// The shared day and week figures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedFigures {
    pub day: WindowPnl,
    pub week: WindowPnl,
}

/// `a + b`, refused above [`MAX_SUM`].
fn sum(a: Decimal, b: Decimal) -> Result<Decimal, PnlError> {
    a.checked_add(b)
        .filter(|v| v.abs() <= MAX_SUM)
        .ok_or(PnlError::SumTooLarge)
}

impl SessionWindow {
    /// Counts one ledger line dated `date` in a window beginning on `start`.
    fn line(
        &mut self,
        kind: LedgerKind,
        amount: Decimal,
        date: NaiveDate,
        start: NaiveDate,
    ) -> Result<(), PnlError> {
        let inside = date >= start;
        self.has_lines |= inside;
        if kind == LedgerKind::Opening || !inside {
            // Cash, plus the share cost a buy puts into its pool (its amount is negative).
            self.equity = sum(self.equity, amount)?;
            if kind == LedgerKind::Buy {
                self.equity = sum(self.equity, -amount)?;
            }
        }
        Ok(())
    }

    /// Adds the open loss of one pool held now.
    fn holding(&mut self, loss: Decimal) -> Result<(), PnlError> {
        self.open_loss = sum(self.open_loss, loss)?;
        Ok(())
    }

    /// Counts one sale: in the window it is P&L; before it, its cost has left the pool.
    fn sale(
        &mut self,
        result: Decimal,
        cost_out: Decimal,
        date: NaiveDate,
        start: NaiveDate,
    ) -> Result<(), PnlError> {
        if date >= start {
            self.realised = sum(self.realised, result)?;
        } else {
            self.equity = sum(self.equity, -cost_out)?;
        }
        Ok(())
    }
}

/// Computes every session's day and week figures. `lines` is the whole ledger in id order,
/// `today` the IST date from the `Clock`. `mark` gives a symbol's usable price: `None` when it is
/// missing or stale. It is asked only for pools that hold shares.
///
/// # Errors
/// [`PnlError`] for the first bad line, a broken session or a missing or bad mark.
pub fn session_figures(
    lines: &[ReplayLine<'_>],
    today: NaiveDate,
    active: PaperSessionId,
    mark: impl Fn(&str) -> Option<Decimal>,
) -> Result<Sessions, PnlError> {
    let replayed = replay_pools(lines.iter().copied(), today)?;
    let monday = today
        .checked_sub_days(Days::new(today.weekday().num_days_from_monday().into()))
        .ok_or(PnlError::Calendar)?;
    // Each session's figures and the date of its latest line.
    let mut seen: BTreeMap<PaperSessionId, (SessionFigures, NaiveDate)> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        // Replay has accepted every line, so none of these three can fail here.
        let bad = |fault| ReplayError { line: index, fault };
        let session =
            PaperSessionId::new(line.session).ok_or_else(|| bad(LineFault::BadSession))?;
        let kind = LedgerKind::parse(line.kind).ok_or_else(|| bad(LineFault::UnknownKind))?;
        let date = date_of(line.ts_ist_rfc3339, today).map_err(bad)?;
        let opening = kind == LedgerKind::Opening;
        if seen.contains_key(&session) {
            if opening {
                return Err(PnlError::SecondOpening { line: index });
            }
        } else if !opening {
            return Err(PnlError::NoOpening { line: index });
        }
        let (figures, latest) = seen
            .entry(session)
            .or_insert((SessionFigures::default(), date));
        // A backdated line would make "cash at the window start" ambiguous.
        if date < *latest {
            return Err(PnlError::DateOrder { line: index });
        }
        *latest = date;
        figures.day.line(kind, line.amount, date, today)?;
        figures.week.line(kind, line.amount, date, monday)?;
    }
    let mut by_session: BTreeMap<_, _> = seen.into_iter().map(|(id, (f, _))| (id, f)).collect();
    if !by_session.contains_key(&active) {
        return Err(PnlError::NoActiveSession);
    }
    for s in &replayed.sales {
        // Every sale's session has lines, so the entry exists.
        let f = by_session.entry(s.key.session).or_default();
        let (result, cost_out) = (s.split.result, s.split.cost_out);
        f.day.sale(result, cost_out, s.date, today)?;
        f.week.sale(result, cost_out, s.date, monday)?;
    }
    let mut held: Vec<_> = replayed.pools.iter().filter(|(_, p)| p.qty() > 0).collect();
    // A fixed order, so one ledger with several bad pools always gives the same error.
    held.sort_by_key(|(k, _)| (k.session, k.symbol.as_str(), k.product.as_str()));
    for (key, pool) in held {
        if key.session != active {
            return Err(PnlError::OpenOutsideActive(key.session));
        }
        let loss = pool.open_loss(mark(&key.symbol).ok_or(PnlError::MissingMark)?)?;
        let f = by_session.entry(key.session).or_default();
        f.day.holding(loss)?;
        f.week.holding(loss)?;
    }
    Ok(Sessions { active, by_session })
}

/// Adds one window up over all sessions.
fn shared_window(
    sessions: &Sessions,
    starting_cash: Decimal,
    pick: impl Fn(&SessionFigures) -> SessionWindow,
) -> Result<WindowPnl, PnlError> {
    let (mut pnl, mut equity_base) = (Decimal::ZERO, starting_cash);
    for (id, figures) in &sessions.by_session {
        let w = pick(figures);
        pnl = sum(sum(pnl, w.realised)?, w.open_loss)?;
        if *id == sessions.active || w.has_lines {
            equity_base = equity_base.min(w.equity);
        }
    }
    if equity_base <= Decimal::ZERO {
        return Err(PnlError::BaseNotPositive);
    }
    Ok(WindowPnl { pnl, equity_base })
}

/// The shared day and week figures. `starting_cash` is `paper_starting_cash_inr`, the cap on
/// both bases.
///
/// # Errors
/// `SumTooLarge`; `BaseNotPositive`.
pub fn shared_figures(
    sessions: &Sessions,
    starting_cash: Decimal,
) -> Result<SharedFigures, PnlError> {
    Ok(SharedFigures {
        day: shared_window(sessions, starting_cash, |f| f.day)?,
        week: shared_window(sessions, starting_cash, |f| f.week)?,
    })
}

#[cfg(test)]
mod tests;
