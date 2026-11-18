//! Instrument master: parse and clean a Kite-format instruments CSV (bytes in, no I/O).

mod parse;

pub use parse::{ParseError, ParseLimits, ParsedInstruments, parse_instruments};
