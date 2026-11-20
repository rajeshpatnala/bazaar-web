//! Audit chain hash and verification. Format fixed by `docs/adr/0001-audit-hash-chain.md`.

use sha2::{Digest, Sha256};

use bazaar_core::ports::{AuditDraft, AuditRow};

/// `prev_hash` of the first row.
pub const GENESIS: [u8; 32] = [0; 32];

fn put(h: &mut Sha256, field: &str) {
    // Length-prefixed so field boundaries cannot be shifted.
    h.update((field.len() as u64).to_be_bytes());
    h.update(field.as_bytes());
}

/// `SHA-256(prev || seq_be8 || len+ts || len+actor || len+kind || len+payload)`.
/// Matches `bazaar_core::ports::ChainFn`.
#[must_use]
pub fn chain_hash(prev: &[u8; 32], seq: u64, d: &AuditDraft) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(seq.to_be_bytes());
    for f in [&d.ts_ist_rfc3339, &d.actor, &d.kind, &d.payload] {
        put(&mut h, f);
    }
    h.finalize().into()
}

/// Where a chain first fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainBreak {
    /// Index in the slice of the first bad row.
    pub index: usize,
}

/// Check seq (1-based, gapless), `prev_hash` links and each hash. Detects altered, removed
/// (a gap or a wrong link) and reordered rows. A removed tail is not detectable from rows alone.
///
/// # Errors
/// `ChainBreak` at the first row that does not verify.
pub fn verify_chain(rows: &[AuditRow]) -> Result<(), ChainBreak> {
    let mut prev = GENESIS;
    for (index, r) in rows.iter().enumerate() {
        let ok = r.seq == index as u64 + 1
            && r.prev_hash == prev
            && r.hash == chain_hash(&prev, r.seq, &r.draft);
        if !ok {
            return Err(ChainBreak { index });
        }
        prev = r.hash;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    fn chain(n: u64) -> Vec<AuditRow> {
        let mut prev = GENESIS;
        (1..=n)
            .map(|seq| {
                let draft = AuditDraft {
                    ts_ist_rfc3339: format!("2026-01-05T10:00:0{seq}+05:30"),
                    actor: "user".into(),
                    kind: "approve".into(),
                    payload: format!("{{\"n\":{seq}}}"),
                };
                let hash = chain_hash(&prev, seq, &draft);
                let row = AuditRow {
                    seq,
                    draft,
                    prev_hash: prev,
                    hash,
                };
                prev = hash;
                row
            })
            .collect()
    }

    #[test]
    fn valid_chain_verifies_and_empty_is_valid() {
        assert_eq!(verify_chain(&chain(4)), Ok(()));
        assert_eq!(verify_chain(&[]), Ok(()));
    }

    #[test]
    fn altered_payload_is_detected() {
        let mut c = chain(4);
        c[2].draft.payload = "{\"n\":99}".into();
        assert_eq!(verify_chain(&c), Err(ChainBreak { index: 2 }));
    }

    #[test]
    fn removed_row_is_detected() {
        let mut c = chain(4);
        c.remove(1);
        assert_eq!(verify_chain(&c), Err(ChainBreak { index: 1 }));
    }

    #[test]
    fn reordered_rows_are_detected() {
        let mut c = chain(4);
        c.swap(1, 2);
        assert_eq!(verify_chain(&c), Err(ChainBreak { index: 1 }));
    }

    #[test]
    fn field_boundaries_cannot_be_shifted() {
        let a = AuditDraft {
            ts_ist_rfc3339: "t".into(),
            actor: "ab".into(),
            kind: "c".into(),
            payload: "{}".into(),
        };
        let b = AuditDraft {
            actor: "a".into(),
            kind: "bc".into(),
            ..a.clone()
        };
        assert_ne!(chain_hash(&GENESIS, 1, &a), chain_hash(&GENESIS, 1, &b));
    }

    #[test]
    fn hash_is_pinned_so_the_stored_format_cannot_drift() {
        let d = AuditDraft {
            ts_ist_rfc3339: "2026-01-05T10:00:00+05:30".into(),
            actor: "system".into(),
            kind: "kill_switch_engaged".into(),
            payload: "{}".into(),
        };
        let hex = chain_hash(&GENESIS, 1, &d)
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            });
        assert_eq!(
            hex,
            "b6a2f20c7a74e8980d6185dd6cd1457992d5056190afbf2b5e45d2bc43b47a0c"
        );
    }
}
