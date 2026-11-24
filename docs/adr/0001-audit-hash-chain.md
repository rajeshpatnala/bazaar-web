# ADR 0001: audit log hash chain format

Status: accepted for F10 slice 0 (2026-11-20). Stored forever: changing it breaks `verify_chain` on old rows.

## Decision
Each audit row has `seq` (1-based, gapless), `prev_hash`, `hash` and the draft fields
`ts_ist_rfc3339`, `actor`, `kind`, `payload` (canonical JSON: keys sorted, no whitespace).

```
hash = SHA-256( prev_hash (32 bytes)
              || seq (u64, big-endian, 8 bytes)
              || for each of ts_ist_rfc3339, actor, kind, payload:
                   len (u64, big-endian) || UTF-8 bytes )
```

`prev_hash` of row 1 is 32 zero bytes (`GENESIS`).

## Why length prefixes
The plan's plain concatenation lets bytes move between adjacent fields without changing the
hash (`actor="ab", kind="c"` equals `actor="a", kind="bc"`). Length prefixes remove that.

## Where it lives
`bazaar_execution::audit::{chain_hash, verify_chain}`. The store never depends on `execution`:
callers pass `chain_hash` as a `ChainFn` to `AuditRepo::append`, and the store applies it inside
the append transaction against the last row. A pinned-hash unit test guards the format.

## Limits
The chain is unkeyed SHA-256: it is tamper-evident against accidents and partial edits, not
tamper-proof. Anyone with write access to the database can recompute every hash.
Rows alone detect altered, removed (middle) and reordered rows. Truncating the tail is not
detectable from the rows alone; the anchor file of ADR 0003 detects it at open (detective, not preventive).
The log has no UPDATE/DELETE path; SQLite triggers refuse both (store slice).
