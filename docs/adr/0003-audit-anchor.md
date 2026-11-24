# ADR 0003: audit anchor file

Status: accepted for F10 slice 9a (2026-11-25). (Number 0002 was taken by Kite token storage.)
Closes the "tail truncation is undetectable" limit of ADR 0001 (SEC-96-2, SEC-123-4).

## Decision
A small text file `<db>.anchor` beside the SQLite database holds the latest audit row:

```
bazaar-audit-anchor v1
seq=<n>          (decimal, >= 1)
hash=<64 lowercase hex>
```

- Written after every commit that appended an audit row, from one store function
  (`anchor_after_commit`), serialised by a mutex, never moving backwards (the blocking write
  re-checks a high-water mark, so a timed-out write that lands late cannot regress it).
- Atomic: temp file in the same directory (mode 0600), fsync, rename, fsync the directory.
  A failure before the rename leaves the old anchor intact. Reads are capped at 1 KB.
- Checked at open (`SqliteStore::open_anchored`, used by `compose::open_store`): the chain
  verifies AND row `anchor.seq` exists with `anchor.hash`. The anchor behind the log is normal
  (crash between commit and anchor write) and is caught up at open. The anchor ahead of the log,
  a different hash, a missing file on a non-empty log, an unparsable or oversized file: `Corrupt`,
  startup aborts, nothing is appended. Empty log and no file: fine, created on first append.
- A failed anchor write after a commit does not undo or fail the commit (the order or
  kill-switch change is real; exits never depend on audit availability). It sets a `lag` flag
  (`anchor_status()`), and the next append or `anchor_flush()` retries.

## Recovery
Human-only `bazaar-cli audit-anchor adopt` (built in slice 9b) verifies the chain and writes
the anchor for the current tail (`SqliteStore::anchor_adopt`). Needed once for an existing
database and after restoring a backup. An existing anchor that is ahead of, or disagrees with,
the log is replaced only with the explicit rollback acceptance (`accept_rollback`).

## Limits (say so, never claim more)
- Detective only and unkeyed: someone who can write both files defeats it; it is no defence
  against a deleted `data/` folder (same disk).
- Window: rows appended after the last successful anchor write (a crash, or `lag`) can be
  truncated undetected.
- It proves the log is not older than the anchor, not that rows are genuine.
