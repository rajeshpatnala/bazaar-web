//! In-transaction audit append. The only writer of `audit_log` in this slice: it runs inside
//! the transaction of the change it describes, so the row and the change commit or roll back
//! together. The hash function is passed in (store depends on core only).

use bazaar_core::ports::{AuditDraft, AuditRow, ChainFn, RepoError};
use sqlx::{Row, Sqlite, Transaction};

use crate::sqlite::{SqliteStore, map_err};

const GENESIS: [u8; 32] = [0; 32];
/// Caps on draft fields, enforced before any SQL (SEC-123-4).
const MAX_TS: usize = 64;
const MAX_ACTOR: usize = 500;
const MAX_KIND: usize = 64;
const MAX_PAYLOAD: usize = 64 * 1024;

fn check_caps(d: &AuditDraft) -> Result<(), RepoError> {
    if d.ts_ist_rfc3339.len() > MAX_TS
        || d.actor.len() > MAX_ACTOR
        || d.kind.len() > MAX_KIND
        || d.payload.len() > MAX_PAYLOAD
    {
        return Err(RepoError::TooLarge);
    }
    Ok(())
}

fn to_arr(v: &[u8]) -> Result<[u8; 32], RepoError> {
    <[u8; 32]>::try_from(v).map_err(|_| RepoError::Corrupt)
}

/// Append one row after the current last row; returns its `seq`.
pub(crate) async fn append_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    draft: &AuditDraft,
    chain: ChainFn,
) -> Result<u64, RepoError> {
    check_caps(draft)?;
    let last = sqlx::query("SELECT seq, hash FROM audit_log ORDER BY seq DESC LIMIT 1")
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| map_err(&e))?;
    let (seq, prev) = match last {
        None => (1_u64, GENESIS),
        Some(r) => {
            let s: i64 = r.try_get("seq").map_err(|e| map_err(&e))?;
            let h: Vec<u8> = r.try_get("hash").map_err(|e| map_err(&e))?;
            let next = u64::try_from(s)
                .map_err(|_| RepoError::Corrupt)?
                .checked_add(1)
                .ok_or(RepoError::Other)?;
            (next, to_arr(&h)?)
        }
    };
    let hash = chain(&prev, seq, draft);
    sqlx::query(
        "INSERT INTO audit_log (seq, ts_ist, actor, kind, payload, prev_hash, hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(i64::try_from(seq).map_err(|_| RepoError::Other)?)
    .bind(&draft.ts_ist_rfc3339)
    .bind(&draft.actor)
    .bind(&draft.kind)
    .bind(&draft.payload)
    .bind(prev.to_vec())
    .bind(hash.to_vec())
    .execute(&mut **tx)
    .await
    .map_err(|e| map_err(&e))?;
    Ok(seq)
}

/// Check rows in order: gapless 1-based `seq`, `prev_hash` links, and every hash recomputed with
/// `chain`. A mismatch is `Corrupt` (category only; no row contents).
pub(crate) fn verify_rows(rows: &[AuditRow], chain: ChainFn) -> Result<(), RepoError> {
    let mut prev = GENESIS;
    for (i, r) in rows.iter().enumerate() {
        let want_seq = u64::try_from(i).map_err(|_| RepoError::Other)? + 1;
        if r.seq != want_seq || r.prev_hash != prev || r.hash != chain(&prev, r.seq, &r.draft) {
            return Err(RepoError::Corrupt);
        }
        prev = r.hash;
    }
    Ok(())
}

impl SqliteStore {
    /// All audit rows in `seq` order.
    pub async fn audit_read_all(&self) -> Result<Vec<AuditRow>, RepoError> {
        let rows = sqlx::query(
            "SELECT seq, ts_ist, actor, kind, payload, prev_hash, hash FROM audit_log \
             ORDER BY seq",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_err(&e))?;
        let g = |e: sqlx::Error| map_err(&e);
        rows.iter()
            .map(|r| {
                let seq: i64 = r.try_get("seq").map_err(g)?;
                let prev: Vec<u8> = r.try_get("prev_hash").map_err(g)?;
                let hash: Vec<u8> = r.try_get("hash").map_err(g)?;
                Ok(AuditRow {
                    seq: u64::try_from(seq).map_err(|_| RepoError::Corrupt)?,
                    draft: AuditDraft {
                        ts_ist_rfc3339: r.try_get("ts_ist").map_err(g)?,
                        actor: r.try_get("actor").map_err(g)?,
                        kind: r.try_get("kind").map_err(g)?,
                        payload: r.try_get("payload").map_err(g)?,
                    },
                    prev_hash: to_arr(&prev)?,
                    hash: to_arr(&hash)?,
                })
            })
            .collect()
    }

    /// Re-verify the whole chain with `chain`; `Corrupt` when any row was altered, removed or
    /// reordered.
    pub async fn audit_verify(&self, chain: ChainFn) -> Result<(), RepoError> {
        verify_rows(&self.audit_read_all().await?, chain)
    }

    /// Open like [`SqliteStore::open`], then verify the audit chain: a broken chain refuses to
    /// open (`Corrupt`), so a tampered log is never silently appended to.
    pub async fn open_verified(path: &std::path::Path, chain: ChainFn) -> Result<Self, RepoError> {
        let store = Self::open(path).await?;
        store.audit_verify(chain).await?;
        Ok(store)
    }
}

#[cfg(test)]
pub(crate) fn test_chain(prev: &[u8; 32], seq: u64, d: &AuditDraft) -> [u8; 32] {
    let mut h = *prev;
    h[0] = h[0].wrapping_add(1);
    h[1] = u8::try_from(seq % 251).unwrap_or(0);
    h[2] = u8::try_from(d.payload.len() % 251).unwrap_or(0);
    h
}

#[cfg(test)]
pub(crate) fn test_draft(kind: &str) -> AuditDraft {
    AuditDraft {
        ts_ist_rfc3339: "2026-11-22T09:15:00+05:30".to_owned(),
        actor: "tester".to_owned(),
        kind: kind.to_owned(),
        payload: "{}".to_owned(),
    }
}
