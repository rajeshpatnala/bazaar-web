-- One decision per order: the unique index makes a second approval lose (first wins).
CREATE TABLE approvals (
    approval_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    order_id    TEXT    NOT NULL REFERENCES orders (order_id),
    decision    TEXT    NOT NULL CHECK (decision IN ('approved', 'declined', 'expired')),
    actor       TEXT    NOT NULL,
    decided_ns  INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX approvals_one_per_order ON approvals (order_id);

CREATE TRIGGER approvals_no_update BEFORE UPDATE ON approvals
BEGIN SELECT RAISE(ABORT, 'approvals is append-only'); END;
CREATE TRIGGER approvals_no_delete BEFORE DELETE ON approvals
BEGIN SELECT RAISE(ABORT, 'approvals is append-only'); END;
