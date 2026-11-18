-- Instrument master. Forward-only. Decimals are canonical TEXT (scale-preserving), dates ISO-8601 TEXT.
CREATE TABLE instruments (
    exchange         TEXT    NOT NULL,
    tradingsymbol    TEXT    NOT NULL,
    instrument_token INTEGER NOT NULL,
    exchange_token   INTEGER NOT NULL,
    name             TEXT    NOT NULL,
    instrument_type  TEXT    NOT NULL,
    segment          TEXT    NOT NULL,
    expiry           TEXT,
    strike           TEXT,
    tick_size        TEXT    NOT NULL,
    lot_size         INTEGER NOT NULL,
    fetched_on       TEXT    NOT NULL,
    PRIMARY KEY (exchange, tradingsymbol)
) STRICT;

CREATE INDEX instruments_expiry_idx ON instruments (exchange, instrument_type, expiry);
CREATE INDEX instruments_name_idx ON instruments (name);

-- One dated snapshot per fetch date; re-importing the same date replaces it.
CREATE TABLE instrument_snapshots (
    fetched_on       TEXT    NOT NULL,
    exchange         TEXT    NOT NULL,
    tradingsymbol    TEXT    NOT NULL,
    instrument_token INTEGER NOT NULL,
    exchange_token   INTEGER NOT NULL,
    name             TEXT    NOT NULL,
    instrument_type  TEXT    NOT NULL,
    segment          TEXT    NOT NULL,
    expiry           TEXT,
    strike           TEXT,
    tick_size        TEXT    NOT NULL,
    lot_size         INTEGER NOT NULL,
    PRIMARY KEY (fetched_on, exchange, tradingsymbol)
) STRICT;

CREATE TABLE instrument_imports (
    fetched_on TEXT    NOT NULL PRIMARY KEY,
    row_count  INTEGER NOT NULL
) STRICT;
