-- Closed candles. Forward-only. Prices are canonical TEXT decimals (scale-preserving);
-- close_ns is UTC epoch nanoseconds (sortable, exact); adjusted is part of the key.
CREATE TABLE candles (
    symbol    TEXT    NOT NULL,
    interval  TEXT    NOT NULL,
    adjusted  INTEGER NOT NULL CHECK (adjusted IN (0, 1)),
    close_ns  INTEGER NOT NULL,
    open      TEXT    NOT NULL,
    high      TEXT    NOT NULL,
    low       TEXT    NOT NULL,
    close     TEXT    NOT NULL,
    volume    INTEGER NOT NULL,
    PRIMARY KEY (symbol, interval, adjusted, close_ns)
) STRICT, WITHOUT ROWID;
