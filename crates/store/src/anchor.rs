//! Audit anchor (ADR 0002): the latest audit `(seq, hash)` in a small file beside the database,
//! so a cut tail or an older copy of the database is detected at open. Detective only and
//! unkeyed: someone who can write both files defeats it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bazaar_core::ports::{AuditRow, ChainFn, RepoError};
use sqlx::{Row, SqlitePool};
use tokio::sync::Mutex;

use crate::audit::audit_read_verified;
use crate::sqlite::{SqliteStore, map_err};

const MAGIC: &str = "bazaar-audit-anchor v1";
/// The file is a few dozen bytes; anything over this is refused.
pub(crate) const MAX_ANCHOR_BYTES: u64 = 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Which anchor check refused. Mapped to `RepoError::Corrupt` (`Io` to `Unavailable`) at the
/// store boundary; kept apart so tests and callers can say which rule fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorFault {
    /// No anchor file although the log has rows.
    Missing,
    /// File larger than 1 KB.
    Oversized,
    /// Not the three-line v1 format.
    Unparsable,
    /// Anchor names a row beyond the log's tail (tail cut or older copy).
    Ahead,
    /// The row at `anchor.seq` has a different hash (edited or swapped log).
    HashMismatch,
    /// The file could not be read.
    Io,
}

impl From<AnchorFault> for RepoError {
    fn from(f: AnchorFault) -> Self {
        if f == AnchorFault::Io {
            Self::Unavailable
        } else {
            Self::Corrupt
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnchorRecord {
    pub seq: u64,
    pub hash: [u8; 32],
}

/// Anchor health for the status alarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorStatus {
    /// Highest seq written to the anchor file by this process (or found at open).
    pub last_seq: u64,
    /// The last anchor write failed; the next audit append or flush retries.
    pub lag: bool,
}

/// What `anchor_adopt` changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdoptReport {
    /// Seq of the anchor that was replaced (`None`: missing or unreadable).
    pub old_seq: Option<u64>,
    /// Seq now anchored (the log's tail; 0 = empty log, nothing written).
    pub new_seq: u64,
}

#[derive(Debug)]
pub(crate) struct AnchorState {
    path: PathBuf,
    inner: Mutex<Inner>,
    /// Highest seq on disk, checked inside the blocking write so a timed-out write that lands
    /// late can never move the anchor backwards.
    high_water: Arc<StdMutex<u64>>,
}

#[derive(Debug)]
struct Inner {
    last_seq: u64,
    lag: bool,
}

/// `<db>.anchor`.
pub(crate) fn anchor_path_for(db: &Path) -> PathBuf {
    let mut p = db.as_os_str().to_owned();
    p.push(".anchor");
    PathBuf::from(p)
}

fn to_hex(h: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for b in h {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

fn from_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0_u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

pub(crate) fn encode(r: &AnchorRecord) -> String {
    format!("{MAGIC}\nseq={}\nhash={}\n", r.seq, to_hex(&r.hash))
}

pub(crate) fn parse(bytes: &[u8]) -> Result<AnchorRecord, AnchorFault> {
    let bad = AnchorFault::Unparsable;
    let text = std::str::from_utf8(bytes).map_err(|_| bad)?;
    let mut lines = text.strip_suffix('\n').unwrap_or(text).split('\n');
    let (m, s, h, extra) = (lines.next(), lines.next(), lines.next(), lines.next());
    if m != Some(MAGIC) || extra.is_some() {
        return Err(bad);
    }
    let seq: u64 = s
        .and_then(|l| l.strip_prefix("seq="))
        .and_then(|v| v.parse().ok())
        .filter(|v| *v >= 1)
        .ok_or(bad)?;
    let hash = h
        .and_then(|l| l.strip_prefix("hash="))
        .and_then(from_hex)
        .ok_or(bad)?;
    Ok(AnchorRecord { seq, hash })
}

pub(crate) fn read_file(path: &Path) -> Result<AnchorRecord, AnchorFault> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(AnchorFault::Missing),
        Err(_) => return Err(AnchorFault::Io),
    };
    let mut buf = Vec::new();
    f.take(MAX_ANCHOR_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|_| AnchorFault::Io)?;
    if buf.len() as u64 > MAX_ANCHOR_BYTES {
        return Err(AnchorFault::Oversized);
    }
    parse(&buf)
}

/// Temp file in the same directory, fsync, rename, fsync the directory. A failure before the
/// rename leaves the old anchor untouched.
pub(crate) fn write_atomic(path: &Path, rec: &AnchorRecord) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp)?;
    f.write_all(encode(rec).as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?; // a leftover temp file is harmless: it is overwritten next time
    #[cfg(unix)]
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::File::open(dir)?.sync_all()?;
    }
    Ok(())
}

/// The check at open. `rows` already passed `verify_rows`; returns the anchored seq (0 for an
/// empty log with no file).
pub(crate) fn check_anchor(
    rows: &[AuditRow],
    read: Result<AnchorRecord, AnchorFault>,
) -> Result<u64, AnchorFault> {
    let rec = match read {
        Err(AnchorFault::Missing) if rows.is_empty() => return Ok(0),
        other => other?,
    };
    let idx = usize::try_from(rec.seq - 1).map_err(|_| AnchorFault::Ahead)?;
    let row = rows.get(idx).ok_or(AnchorFault::Ahead)?;
    if row.hash != rec.hash {
        return Err(AnchorFault::HashMismatch);
    }
    Ok(rec.seq)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, RepoError> {
    match tokio::time::timeout(WRITE_TIMEOUT, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(v)) => Ok(v),
        _ => Err(RepoError::Unavailable),
    }
}

impl AnchorState {
    fn new(path: PathBuf, last_seq: u64) -> Self {
        Self {
            path,
            inner: Mutex::new(Inner {
                last_seq,
                lag: false,
            }),
            high_water: Arc::new(StdMutex::new(last_seq)),
        }
    }

    /// Anchor the log's current tail if it is ahead of what is on disk. Serialised; never moves
    /// backwards; a failure sets `lag` and is returned.
    pub(crate) async fn sync(&self, pool: &SqlitePool) -> Result<(), RepoError> {
        let mut g = self.inner.lock().await;
        let res = self.write_tail(&mut g, pool).await;
        g.lag = res.is_err();
        res
    }

    async fn write_tail(&self, g: &mut Inner, pool: &SqlitePool) -> Result<(), RepoError> {
        let tail = sqlx::query("SELECT seq, hash FROM audit_log ORDER BY seq DESC LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(|e| map_err(&e))?;
        let Some(r) = tail else { return Ok(()) };
        let seq = u64::try_from(r.try_get::<i64, _>("seq").map_err(|e| map_err(&e))?)
            .map_err(|_| RepoError::Corrupt)?;
        if seq <= g.last_seq {
            return Ok(());
        }
        let h: Vec<u8> = r.try_get("hash").map_err(|e| map_err(&e))?;
        let hash = <[u8; 32]>::try_from(h.as_slice()).map_err(|_| RepoError::Corrupt)?;
        let (path, hw) = (self.path.clone(), Arc::clone(&self.high_water));
        let rec = AnchorRecord { seq, hash };
        blocking(move || {
            let mut hw = hw.lock().map_err(|_| RepoError::Other)?;
            if seq > *hw {
                write_atomic(&path, &rec).map_err(|_| RepoError::Other)?;
                *hw = seq;
            }
            Ok::<(), RepoError>(())
        })
        .await??;
        g.last_seq = seq;
        Ok(())
    }
}

impl SqliteStore {
    /// Open like `open`, verify the audit chain AND the anchor beside the database (ADR 0002).
    /// Refuses with `Corrupt` on a cut tail, an older copy, a different hash, a missing file on
    /// a non-empty log, or an unparsable or oversized file; nothing is appended then. The
    /// execution store is opened only through this (via `compose::open_store`).
    pub async fn open_anchored(path: &Path, chain: ChainFn) -> Result<Self, RepoError> {
        let mut store = Self::open(path).await?;
        let rows = audit_read_verified(&store, chain).await?;
        let ap = anchor_path_for(path);
        let p2 = ap.clone();
        let read = blocking(move || read_file(&p2)).await?;
        let seq = check_anchor(&rows, read)?;
        let state = Arc::new(AnchorState::new(ap, seq));
        store.anchor = Some(Arc::clone(&state));
        // Anchor behind the log is the normal crash window: catch up now.
        state.sync(&store.pool).await?;
        Ok(store)
    }

    /// Anchor health; `None` for a store opened without an anchor (tests, in-memory).
    pub async fn anchor_status(&self) -> Option<AnchorStatus> {
        let a = self.anchor.as_ref()?;
        let g = a.inner.lock().await;
        Some(AnchorStatus {
            last_seq: g.last_seq,
            lag: g.lag,
        })
    }

    /// Retry a lagging anchor write (call on close). `Ok` when the anchor is at the tail.
    ///
    /// # Errors
    /// The write error, when the anchor still cannot be written.
    pub async fn anchor_flush(&self) -> Result<(), RepoError> {
        match &self.anchor {
            Some(a) => a.sync(&self.pool).await,
            None => Ok(()),
        }
    }

    /// The one post-commit hook, called after every commit that appended an audit row. The
    /// commit stands whatever happens here: a failure is recorded as the `lag` flag (shown by
    /// `anchor_status`) and retried by the next append or `anchor_flush` (RISK-119-6).
    pub(crate) async fn anchor_after_commit(&self) {
        if let Some(a) = &self.anchor {
            a.sync(&self.pool).await.ok();
        }
    }

    /// Human-only recovery (`audit-anchor adopt`): verify the chain, then write the anchor for
    /// the current tail. If an existing anchor is ahead of the log or disagrees with it (older
    /// copy restored, edited log) this refuses with `Conflict` unless `accept_rollback`.
    ///
    /// # Errors
    /// `Corrupt` on a broken chain (nothing written), `Conflict` as above, store errors.
    pub async fn anchor_adopt(
        db: &Path,
        chain: ChainFn,
        accept_rollback: bool,
    ) -> Result<AdoptReport, RepoError> {
        let store = Self::open(db).await?;
        let rows = audit_read_verified(&store, chain).await?;
        let ap = anchor_path_for(db);
        let p2 = ap.clone();
        let old = blocking(move || read_file(&p2)).await?;
        if old == Err(AnchorFault::Io) {
            return Err(RepoError::Unavailable);
        }
        let old_seq = old.ok().map(|r| r.seq);
        if old.is_ok() && check_anchor(&rows, old).is_err() && !accept_rollback {
            return Err(RepoError::Conflict);
        }
        let Some(tail) = rows.last() else {
            return Ok(AdoptReport {
                old_seq,
                new_seq: 0,
            });
        };
        let rec = AnchorRecord {
            seq: tail.seq,
            hash: tail.hash,
        };
        blocking(move || write_atomic(&ap, &rec))
            .await?
            .map_err(|_| RepoError::Other)?;
        Ok(AdoptReport {
            old_seq,
            new_seq: rec.seq,
        })
    }
}
