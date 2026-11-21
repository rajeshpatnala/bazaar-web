//! In-transaction audit append. The only writer of `audit_log` in this slice: it runs inside
//! the transaction of the change it describes, so the row and the change commit or roll back
//! together. The hash function is passed in (store depends on core only).

use bazaar_core::ports::{AuditDraft, ChainFn, RepoError};
use sqlx::{Row, Sqlite, Transaction};

use crate::sqlite::map_err;

const GENESIS: [u8; 32] = [0; 32];

fn to_arr(v: &[u8]) -> Result<[u8; 32], RepoError> {
    <[u8; 32]>::try_from(v).map_err(|_| RepoError::Corrupt)
}

/// Append one row after the current last row; returns its `seq`.
pub(crate) async fn append_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    draft: &AuditDraft,
    chain: ChainFn,
) -> Result<u64, RepoError> {
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
