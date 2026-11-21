-- Single-row kill switch state. A missing row is read as Corrupt (fail closed by the caller).
CREATE TABLE kill_switch (
    id         INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    engaged    INTEGER NOT NULL CHECK (engaged IN (0, 1)),
    version    INTEGER NOT NULL CHECK (version >= 1),
    actor      TEXT    NOT NULL,
    reason     TEXT    NOT NULL,
    changed_ns INTEGER NOT NULL
) STRICT;

INSERT INTO kill_switch (id, engaged, version, actor, reason, changed_ns)
VALUES (1, 0, 1, 'system', 'initial', 0);

CREATE TRIGGER kill_switch_no_delete BEFORE DELETE ON kill_switch
BEGIN SELECT RAISE(ABORT, 'kill_switch row cannot be deleted'); END;
