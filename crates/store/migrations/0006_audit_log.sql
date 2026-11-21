-- Hash-chained audit log (docs/adr/0001-audit-hash-chain.md). seq is 1-based and gapless
-- (the writer appends inside the same transaction as the change it describes); the first
-- row's prev_hash is 32 zero bytes. No UPDATE or DELETE path: triggers refuse both.
CREATE TABLE audit_log (
    seq       INTEGER NOT NULL PRIMARY KEY,
    ts_ist    TEXT    NOT NULL,
    actor     TEXT    NOT NULL,
    kind      TEXT    NOT NULL,
    payload   TEXT    NOT NULL,
    prev_hash BLOB    NOT NULL CHECK (length(prev_hash) = 32),
    hash      BLOB    NOT NULL CHECK (length(hash) = 32)
) STRICT;

CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
