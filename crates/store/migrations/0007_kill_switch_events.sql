-- Append-only history of kill-switch changes, written in the same transaction as each set.
-- Also: orders can never be deleted.
CREATE TABLE kill_switch_events (
    version    INTEGER NOT NULL PRIMARY KEY,
    engaged    INTEGER NOT NULL CHECK (engaged IN (0, 1)),
    actor      TEXT    NOT NULL,
    reason     TEXT    NOT NULL,
    changed_ns INTEGER NOT NULL
) STRICT;

CREATE TRIGGER kill_switch_events_no_update BEFORE UPDATE ON kill_switch_events
BEGIN SELECT RAISE(ABORT, 'kill_switch_events is append-only'); END;
CREATE TRIGGER kill_switch_events_no_delete BEFORE DELETE ON kill_switch_events
BEGIN SELECT RAISE(ABORT, 'kill_switch_events is append-only'); END;

CREATE TRIGGER orders_no_delete BEFORE DELETE ON orders
BEGIN SELECT RAISE(ABORT, 'orders cannot be deleted'); END;
