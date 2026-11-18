//! Bounded, panic-free parser for the Kite instruments CSV.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use bazaar_core::error::DomainError;
use bazaar_core::instrument::{CleaningReport, Exchange, Instrument, InstrumentType, RejectReason};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use thiserror::Error;

const REQUIRED: [&str; 11] = [
    "instrument_token",
    "exchange_token",
    "tradingsymbol",
    "name",
    "expiry",
    "strike",
    "tick_size",
    "lot_size",
    "instrument_type",
    "segment",
    "exchange",
];

/// Bounds applied while parsing untrusted input. The defaults leave generous headroom over
/// the Kite master (on the order of 100k rows); change them deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLimits {
    /// Maximum input size; checked on the slice, so the caller must bound the read too.
    pub max_bytes: usize,
    /// Maximum number of data rows (header excluded).
    pub max_rows: usize,
    /// Maximum length of one field in bytes; longer rows are rejected.
    pub max_field_len: usize,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_rows: 500_000,
            max_field_len: 256,
        }
    }
}

/// Failures that stop the whole parse (row-level problems are counted, not raised).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    /// Input is larger than `ParseLimits::max_bytes`.
    #[error("input is {size} bytes, over the {max} byte limit")]
    TooLarge { size: usize, max: usize },
    /// More data rows than `ParseLimits::max_rows`.
    #[error("input has more than {max} rows")]
    TooManyRows { max: usize },
    /// A required header column is absent.
    #[error("required column `{0}` is missing")]
    MissingColumn(&'static str),
    /// Empty input.
    #[error("input has no header row")]
    NoHeader,
    /// The header record could not be read.
    #[error("csv header could not be read")]
    Csv,
}

/// Result of one parse: the clean rows and what was dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInstruments {
    pub instruments: Vec<Instrument>,
    pub report: CleaningReport,
}

type Columns = HashMap<&'static str, usize>;

/// Parse and clean `bytes`. Extra columns are ignored; bad rows are counted in the report.
/// A later row repeating an earlier `(exchange, tradingsymbol)` or instrument token is
/// rejected as `Duplicate` (first wins). If the input does not end in a newline the last
/// record is rejected as `Truncated`, since it may be cut inside any column. Never panics.
pub fn parse_instruments(
    bytes: &[u8],
    limits: &ParseLimits,
) -> Result<ParsedInstruments, ParseError> {
    if bytes.len() > limits.max_bytes {
        return Err(ParseError::TooLarge {
            size: bytes.len(),
            max: limits.max_bytes,
        });
    }
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes);
    let mut records = rdr.byte_records();
    let header = match records.next() {
        Some(Ok(h)) => h,
        Some(Err(_)) => return Err(ParseError::Csv),
        None => return Err(ParseError::NoHeader),
    };
    let mut named: HashMap<String, usize> = HashMap::new();
    for (i, f) in header.iter().enumerate() {
        let name = String::from_utf8_lossy(f)
            .trim_start_matches('\u{feff}')
            .trim()
            .to_ascii_lowercase();
        named.entry(name).or_insert(i);
    }
    let mut idx: Columns = HashMap::new();
    for name in REQUIRED {
        let i = named.get(name).ok_or(ParseError::MissingColumn(name))?;
        idx.insert(name, *i);
    }

    let complete = bytes.ends_with(b"\n");
    let mut seen_keys: HashSet<(Exchange, String)> = HashSet::new();
    let mut seen_tokens: HashSet<u64> = HashSet::new();
    let mut records = records.peekable();
    let mut instruments = Vec::new();
    let mut report = CleaningReport::default();
    let mut rows = 0usize;
    while let Some(rec) = records.next() {
        let is_last = records.peek().is_none();
        rows += 1;
        if rows > limits.max_rows {
            return Err(ParseError::TooManyRows {
                max: limits.max_rows,
            });
        }
        let Ok(rec) = rec else {
            report.reject(RejectReason::InvalidEncoding);
            continue;
        };
        if is_last && !complete {
            report.reject(RejectReason::Truncated);
            continue;
        }
        match parse_row(&rec, &idx, limits) {
            Ok(ins) => {
                let key = (ins.exchange.clone(), ins.tradingsymbol.clone());
                if seen_keys.contains(&key) || seen_tokens.contains(&ins.instrument_token) {
                    report.reject(RejectReason::Duplicate);
                    continue;
                }
                seen_keys.insert(key);
                seen_tokens.insert(ins.instrument_token);
                instruments.push(ins);
                report.accept();
            }
            Err(reason) => report.reject(reason),
        }
    }
    Ok(ParsedInstruments {
        instruments,
        report,
    })
}

fn field<'a>(
    rec: &'a csv::ByteRecord,
    idx: &Columns,
    name: &str,
    limits: &ParseLimits,
) -> Result<&'a str, RejectReason> {
    let i = idx.get(name).copied().ok_or(RejectReason::MissingField)?;
    let raw = rec.get(i).ok_or(RejectReason::MissingField)?;
    if raw.len() > limits.max_field_len {
        return Err(RejectReason::FieldTooLong);
    }
    let s = std::str::from_utf8(raw).map_err(|_| RejectReason::InvalidEncoding)?;
    Ok(s.trim())
}

fn parse_row(
    rec: &csv::ByteRecord,
    idx: &Columns,
    limits: &ParseLimits,
) -> Result<Instrument, RejectReason> {
    let f = |name: &str| field(rec, idx, name, limits);
    let tradingsymbol = f("tradingsymbol")?;
    if tradingsymbol.is_empty() {
        return Err(RejectReason::MissingField);
    }
    let exchange = f("exchange")?;
    let type_code = f("instrument_type")?;
    if exchange.is_empty() || type_code.is_empty() {
        return Err(RejectReason::MissingField);
    }
    let instrument_token =
        u64::from_str(f("instrument_token")?).map_err(|_| RejectReason::MalformedNumber)?;
    let exchange_token =
        u64::from_str(f("exchange_token")?).map_err(|_| RejectReason::MalformedNumber)?;
    let tick_size =
        Decimal::from_str(f("tick_size")?).map_err(|_| RejectReason::MalformedNumber)?;
    let lot = i64::from_str(f("lot_size")?).map_err(|_| RejectReason::MalformedNumber)?;
    let lot_size = match u32::try_from(lot) {
        Ok(n) => n,
        Err(_) if lot <= 0 => 0,
        Err(_) => return Err(RejectReason::MalformedNumber),
    };
    let expiry = match f("expiry")? {
        "" => None,
        s => Some(
            NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| RejectReason::MalformedDate)?,
        ),
    };
    let strike = match f("strike")? {
        "" => None,
        s => Some(Decimal::from_str(s).map_err(|_| RejectReason::MalformedNumber)?),
    };
    let mut ins = Instrument {
        instrument_token,
        exchange_token,
        tradingsymbol: tradingsymbol.to_owned(),
        name: f("name")?.to_owned(),
        exchange: Exchange::parse(exchange),
        instrument_type: InstrumentType::parse(type_code),
        segment: f("segment")?.to_owned(),
        expiry,
        strike,
        tick_size,
        lot_size,
    };
    ins.normalise();
    ins.validate().map_err(|e| match e {
        DomainError::InvalidTick => RejectReason::NonPositiveTick,
        DomainError::InvalidLotSize => RejectReason::NonPositiveLot,
        DomainError::InvalidExpiry => RejectReason::InvalidExpiry,
        DomainError::InvalidStrike => RejectReason::InvalidStrike,
        _ => RejectReason::MissingField,
    })?;
    Ok(ins)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn load(name: &str) -> Vec<u8> {
        let p = format!("{}/../../fixtures/data/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(p).unwrap()
    }
    fn parse(name: &str) -> Result<ParsedInstruments, ParseError> {
        parse_instruments(&load(name), &ParseLimits::default())
    }
    fn count(r: &CleaningReport, reason: RejectReason) -> u64 {
        r.reasons
            .iter()
            .find(|(x, _)| *x == reason)
            .map_or(0, |(_, n)| *n)
    }
    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn good_file_parses_with_types() {
        let p = parse("instruments_good.csv").unwrap();
        assert_eq!(p.report.accepted, 5);
        assert_eq!(p.report.rejected, 0);
        let eq = &p.instruments[0];
        assert_eq!(eq.tick_size, dec("0.05"));
        assert_eq!(eq.strike, None);
        assert_eq!(eq.expiry, None);
        let fut = &p.instruments[1];
        assert_eq!(fut.strike, None, "strike 0 on a future is None");
        assert_eq!(fut.expiry, NaiveDate::from_ymd_opt(2031, 1, 29));
        let ce = &p.instruments[2];
        assert_eq!(ce.strike, Some(dec("100")));
        assert_eq!(ce.instrument_type, InstrumentType::Call);
        let gb = &p.instruments[4];
        assert_eq!(gb.instrument_type, InstrumentType::Other("GB".into()));
        assert_eq!(gb.tick_size, dec("0.0025"));
    }

    #[test]
    fn extra_columns_tolerated() {
        let p = parse("instruments_extra_column.csv").unwrap();
        assert_eq!((p.report.accepted, p.report.rejected), (2, 0));
    }

    #[test]
    fn bad_rows_are_counted_by_reason() {
        let p = parse("instruments_bad_rows.csv").unwrap();
        assert_eq!(p.report.accepted, 1);
        assert_eq!(p.report.rejected, 8);
        assert_eq!(count(&p.report, RejectReason::NonPositiveTick), 1);
        assert_eq!(count(&p.report, RejectReason::NonPositiveLot), 1);
        assert_eq!(count(&p.report, RejectReason::MalformedNumber), 1);
        assert_eq!(count(&p.report, RejectReason::MalformedDate), 1);
        assert_eq!(count(&p.report, RejectReason::InvalidExpiry), 1);
        assert_eq!(count(&p.report, RejectReason::InvalidStrike), 1);
        assert_eq!(count(&p.report, RejectReason::MissingField), 2);
        assert_eq!(p.report.total(), 9);
    }

    #[test]
    fn truncated_input_does_not_panic() {
        let p = parse("instruments_truncated.csv").unwrap();
        assert_eq!(p.report.accepted, 1);
        assert_eq!(p.report.rejected, 1);
        let all = load("instruments_good.csv");
        let full = parse_instruments(&all, &ParseLimits::default()).unwrap();
        for n in 0..all.len() {
            if let Ok(part) = parse_instruments(&all[..n], &ParseLimits::default()) {
                for ins in &part.instruments {
                    assert!(full.instruments.contains(ins), "prefix {n} made a row");
                }
            }
        }
        assert_eq!(count(&p.report, RejectReason::Truncated), 1);
    }

    #[test]
    fn missing_required_column_is_clear_error() {
        let e = parse_instruments(b"instrument_token,name\n1,x\n", &ParseLimits::default())
            .unwrap_err();
        assert_eq!(e, ParseError::MissingColumn("exchange_token"));
        assert!(e.to_string().contains("exchange_token"));
    }

    #[test]
    fn empty_input_has_no_header() {
        assert_eq!(
            parse_instruments(b"", &ParseLimits::default()).unwrap_err(),
            ParseError::NoHeader
        );
    }

    #[test]
    fn oversize_input_is_rejected() {
        let l = ParseLimits {
            max_bytes: 10,
            ..ParseLimits::default()
        };
        assert!(matches!(
            parse_instruments(&load("instruments_good.csv"), &l),
            Err(ParseError::TooLarge { .. })
        ));
    }

    #[test]
    fn too_many_rows_is_rejected() {
        let l = ParseLimits {
            max_rows: 2,
            ..ParseLimits::default()
        };
        assert_eq!(
            parse_instruments(&load("instruments_good.csv"), &l).unwrap_err(),
            ParseError::TooManyRows { max: 2 }
        );
    }

    #[test]
    fn long_field_row_is_rejected_and_invalid_utf8_counted() {
        let l = ParseLimits {
            max_field_len: 8,
            ..ParseLimits::default()
        };
        let p = parse_instruments(&load("instruments_good.csv"), &l).unwrap();
        assert!(count(&p.report, RejectReason::FieldTooLong) > 0);
        let mut b = load("instruments_good.csv");
        b.extend_from_slice(b"9,9,\xff\xfe,N,0,,0,0.05,1,EQ,NSE,NSE\n");
        let p = parse_instruments(&b, &ParseLimits::default()).unwrap();
        assert_eq!(count(&p.report, RejectReason::InvalidEncoding), 1);
    }

    #[test]
    fn duplicates_are_rejected_first_wins() {
        let mut b = load("instruments_good.csv");
        // same key, new token; new key, same token
        b.extend_from_slice(b"2001,4,TESTEQ,OTHER,0,,0,0.05,1,EQ,NSE,NSE\n");
        b.extend_from_slice(b"1001,4,NEWSYM,OTHER,0,,0,0.05,1,EQ,NSE,NSE\n");
        let p = parse_instruments(&b, &ParseLimits::default()).unwrap();
        assert_eq!((p.report.accepted, p.report.rejected), (5, 2));
        assert_eq!(count(&p.report, RejectReason::Duplicate), 2);
        assert_eq!(p.instruments[0].name, "TEST EQUITY");
    }

    #[test]
    fn same_symbol_on_another_exchange_is_not_a_duplicate() {
        let mut b = load("instruments_good.csv");
        b.extend_from_slice(b"2001,4,TESTEQ,TEST EQUITY,0,,0,0.05,1,EQ,BSE,BSE\n");
        let p = parse_instruments(&b, &ParseLimits::default()).unwrap();
        assert_eq!(p.report.accepted, 6);
    }

    #[test]
    fn empty_exchange_or_type_is_missing_field() {
        let mut b = load("instruments_good.csv");
        b.extend_from_slice(b"2001,4,A,A,0,,0,0.05,1,EQ,NSE,\n");
        b.extend_from_slice(b"2002,5,B,B,0,,0,0.05,1,,NSE,NSE\n");
        let p = parse_instruments(&b, &ParseLimits::default()).unwrap();
        assert_eq!(count(&p.report, RejectReason::MissingField), 2);
    }

    #[test]
    fn missing_trailing_newline_rejects_last_row_cut_in_last_column() {
        let all = load("instruments_good.csv");
        let cut = &all[..all.len() - 2]; // "...,NSE" -> "...,NS", no newline
        let p = parse_instruments(cut, &ParseLimits::default()).unwrap();
        assert_eq!((p.report.accepted, p.report.rejected), (4, 1));
        assert_eq!(count(&p.report, RejectReason::Truncated), 1);
    }

    #[test]
    fn negative_lot_is_non_positive_not_malformed() {
        let b = b"instrument_token,exchange_token,tradingsymbol,name,expiry,strike,tick_size,lot_size,instrument_type,segment,exchange\n1,1,A,A,,0,0.05,-3,EQ,NSE,NSE\n";
        let p = parse_instruments(b, &ParseLimits::default()).unwrap();
        assert_eq!(count(&p.report, RejectReason::NonPositiveLot), 1);
    }
}
