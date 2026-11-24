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
- The write runs detached after the commit (coalesced, one task at a time), so an engage or an
  exit never waits for file I/O (the very first row of a new log is written inline, so a crash
  cannot leave a non-empty log with no file); `anchor_flush()` waits for it (use on close and in tests).
- A failed anchor write after a commit does not undo or fail the commit (the order or
  kill-switch change is real; exits never depend on audit availability). It sets a `lag` flag
  (`anchor_status()`), is logged by category only (`tracing::warn`, no path or row contents), and
  the next append or `anchor_flush()` retries.
- The catch-up at open (anchor behind the log) is best effort: the read check has passed, so a
  write failure there only sets `lag` and logs; startup continues, because refusing to start
  would block exits at a restart with open positions.

## Recovery
Human-only `bazaar-cli audit-anchor adopt` (built in slice 9b) verifies the chain and writes
the anchor for the current tail (`SqliteStore::anchor_adopt`). Needed once for an existing
database and after restoring a backup. Any existing anchor file (ahead, different hash, garbage,
oversized, or even consistent) is replaced only with the explicit acceptance (`accept_rollback`);
only a missing file adopts freely. `AdoptReport` names the old seq and the fault found; the CLI also prints the new tail seq and the
last row's timestamp so the owner can check them against their own records.

Stop the server first: adopt takes no lock, and a running server's anchor writer could race it
(single writer per database). A missing-file adopt trusts the current tail: an attacker who also
deleted the anchor and cut rows is not detected. Adopt writes no audit row; the printed report
(old seq, fault, new seq, tail time on stdout) is the only trail. An audit row for adopt is a
slice 9c follow-up.

## Limits (say so, never claim more)
- Single writer: the high-water mark is per process; a second process on the same database, or
  a stale one, could write an older tail over a newer anchor. Run one app instance per database.
- Detective only and unkeyed: someone who can write both files defeats it; it is no defence
  against a deleted `data/` folder (same disk).
- Window: rows appended after the last successful anchor write (a crash, or `lag`) can be
  truncated undetected.
- It proves the log is not older than the anchor, not that rows are genuine.
