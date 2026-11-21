-- Orders and their event log. Forward-only. state/event are stable text labels owned by
-- `execution`; payload is the serialised intent. version starts at 1 and is bumped by every
-- advance (optimistic check). order_events is append-only: UPDATE and DELETE are refused.
CREATE TABLE orders (
    order_id   TEXT    NOT NULL PRIMARY KEY,
    state      TEXT    NOT NULL CHECK (length(state) BETWEEN 1 AND 64),
    version    INTEGER NOT NULL CHECK (version >= 1),
    payload    TEXT    NOT NULL,
    created_ns INTEGER NOT NULL,
    updated_ns INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX orders_by_state ON orders (state, created_ns);

CREATE TABLE order_events (
    order_id   TEXT    NOT NULL REFERENCES orders (order_id),
    version    INTEGER NOT NULL,
    event      TEXT    NOT NULL,
    from_state TEXT,
    to_state   TEXT    NOT NULL,
    at_ns      INTEGER NOT NULL,
    PRIMARY KEY (order_id, version)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER order_events_no_update BEFORE UPDATE ON order_events
BEGIN SELECT RAISE(ABORT, 'order_events is append-only'); END;
CREATE TRIGGER order_events_no_delete BEFORE DELETE ON order_events
BEGIN SELECT RAISE(ABORT, 'order_events is append-only'); END;
