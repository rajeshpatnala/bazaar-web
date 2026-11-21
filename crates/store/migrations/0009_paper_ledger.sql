-- Append-only paper ledger: balances are derived by replay. Amounts are exact decimal text.
-- A reset starts a new epoch; old entries stay.
CREATE TABLE paper_ledger (
    id     INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    epoch  INTEGER NOT NULL CHECK (epoch >= 1),
    kind   TEXT    NOT NULL CHECK (length(kind) BETWEEN 1 AND 64),
    amount TEXT    NOT NULL CHECK (length(amount) BETWEEN 1 AND 64),
    ref_id TEXT,
    ts_ist TEXT    NOT NULL CHECK (length(ts_ist) <= 64)
) STRICT;
CREATE TRIGGER paper_ledger_no_update BEFORE UPDATE ON paper_ledger
BEGIN SELECT RAISE(ABORT, 'paper_ledger is append-only'); END;
CREATE TRIGGER paper_ledger_no_delete BEFORE DELETE ON paper_ledger
BEGIN SELECT RAISE(ABORT, 'paper_ledger is append-only'); END;
