//! SQLite implementation of [`InstrumentRepo`]: one writer pool, forward-only migrations,
//! runtime queries with explicit columns. Errors are mapped to safe categories only.
//!
//! Key normalisation: `tradingsymbol` is trimmed and ASCII-upper-cased on write and in `get`;
//! `exchange` is stored as its canonical code; `name` is stored verbatim and matched with
//! `UPPER(TRIM(name))` in queries. An import dated before the latest import is rejected
//! (`RepoError::Other`) so older data never overwrites newer rows.

use std::borrow::Cow;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use bazaar_core::instrument::{Exchange, Instrument, InstrumentType};
use bazaar_core::ports::{InstrumentRepo, RepoError, UpsertSummary};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::migrate::{Migration, MigrationType, Migrator};
use sqlx::query::Query;
use sqlx::sqlite::{
    Sqlite, SqliteArguments, SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow,
};
use sqlx::{Row, SqlStr, SqlitePool};

const DATE_FMT: &str = "%Y-%m-%d";

const UPSERT_INSTRUMENT: &str = "INSERT INTO instruments (instrument_token, exchange_token, \
    tradingsymbol, name, exchange, instrument_type, segment, expiry, strike, tick_size, \
    lot_size, fetched_on) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
    ON CONFLICT (exchange, tradingsymbol) DO UPDATE SET \
    instrument_token = excluded.instrument_token, exchange_token = excluded.exchange_token, \
    name = excluded.name, instrument_type = excluded.instrument_type, \
    segment = excluded.segment, expiry = excluded.expiry, strike = excluded.strike, \
    tick_size = excluded.tick_size, lot_size = excluded.lot_size, \
    fetched_on = excluded.fetched_on";

const UPSERT_SNAPSHOT: &str = "INSERT INTO instrument_snapshots (instrument_token, \
    exchange_token, tradingsymbol, name, exchange, instrument_type, segment, expiry, strike, \
    tick_size, lot_size, fetched_on) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
    ON CONFLICT (fetched_on, exchange, tradingsymbol) DO UPDATE SET \
    instrument_token = excluded.instrument_token, exchange_token = excluded.exchange_token, \
    name = excluded.name, instrument_type = excluded.instrument_type, \
    segment = excluded.segment, expiry = excluded.expiry, strike = excluded.strike, \
    tick_size = excluded.tick_size, lot_size = excluded.lot_size";

const GET_INSTRUMENT: &str = "SELECT instrument_token, exchange_token, tradingsymbol, name, \
    exchange, instrument_type, segment, expiry, strike, tick_size, lot_size \
    FROM instruments WHERE exchange = ? AND tradingsymbol = ?";

fn migrator() -> Migrator {
    Migrator::with_migrations(vec![Migration::new(
        1,
        Cow::Borrowed("instruments"),
        MigrationType::Simple,
        SqlStr::from_static(include_str!("../migrations/0001_instruments.sql")),
        false,
    )])
}

fn map_err(e: &sqlx::Error) -> RepoError {
    match e {
        sqlx::Error::Io(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed
        | sqlx::Error::Configuration(_) => RepoError::Unavailable,
        sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_) => RepoError::Corrupt,
        _ => RepoError::Other,
    }
}

fn fmt_date(d: NaiveDate) -> String {
    d.format(DATE_FMT).to_string()
}

fn parse_date(s: &str) -> Result<NaiveDate, RepoError> {
    NaiveDate::parse_from_str(s, DATE_FMT).map_err(|_| RepoError::Corrupt)
}

fn to_i64(v: u64) -> Result<i64, RepoError> {
    i64::try_from(v).map_err(|_| RepoError::Other)
}

fn norm(s: &str) -> String {
    s.trim().to_ascii_uppercase()
}

fn bind_row<'q>(
    sql: &'static str,
    r: &Instrument,
    day: &str,
) -> Result<Query<'q, Sqlite, SqliteArguments>, RepoError> {
    Ok(sqlx::query(sql)
        .bind(to_i64(r.instrument_token)?)
        .bind(to_i64(r.exchange_token)?)
        .bind(norm(&r.tradingsymbol))
        .bind(r.name.clone())
        .bind(r.exchange.as_str().to_owned())
        .bind(r.instrument_type.as_str().to_owned())
        .bind(r.segment.clone())
        .bind(r.expiry.map(fmt_date))
        // Decimal::to_string keeps the scale, so 0.050 stays "0.050".
        .bind(r.strike.map(|d| d.to_string()))
        .bind(r.tick_size.to_string())
        .bind(i64::from(r.lot_size))
        .bind(day.to_owned()))
}

fn to_instrument(row: &SqliteRow) -> Result<Instrument, RepoError> {
    let g = |e: sqlx::Error| map_err(&e);
    let token: i64 = row.try_get("instrument_token").map_err(g)?;
    let ex_token: i64 = row.try_get("exchange_token").map_err(g)?;
    let lot: i64 = row.try_get("lot_size").map_err(g)?;
    let expiry: Option<String> = row.try_get("expiry").map_err(g)?;
    let strike: Option<String> = row.try_get("strike").map_err(g)?;
    let tick: String = row.try_get("tick_size").map_err(g)?;
    let exchange: String = row.try_get("exchange").map_err(g)?;
    let itype: String = row.try_get("instrument_type").map_err(g)?;
    Ok(Instrument {
        instrument_token: u64::try_from(token).map_err(|_| RepoError::Corrupt)?,
        exchange_token: u64::try_from(ex_token).map_err(|_| RepoError::Corrupt)?,
        tradingsymbol: row.try_get("tradingsymbol").map_err(g)?,
        name: row.try_get("name").map_err(g)?,
        exchange: Exchange::parse(&exchange),
        instrument_type: InstrumentType::parse(&itype),
        segment: row.try_get("segment").map_err(g)?,
        expiry: expiry.as_deref().map(parse_date).transpose()?,
        strike: strike
            .as_deref()
            .map(Decimal::from_str)
            .transpose()
            .map_err(|_| RepoError::Corrupt)?,
        tick_size: Decimal::from_str(&tick).map_err(|_| RepoError::Corrupt)?,
        lot_size: u32::try_from(lot).map_err(|_| RepoError::Corrupt)?,
    })
}

/// The SQLite store. Holds a single-connection pool, so all access is serialised (one writer).
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

/// Best effort: owner-only access to the database and its WAL/SHM files (unix).
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for suffix in ["", "-wal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(suffix);
            let _ = std::fs::set_permissions(
                std::path::PathBuf::from(p),
                std::fs::Permissions::from_mode(0o600),
            );
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

impl SqliteStore {
    /// Open (creating if missing) the database at `path` and run migrations.
    pub async fn open(path: &Path) -> Result<Self, RepoError> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let store = Self::connect(opts).await?;
        restrict_permissions(path);
        Ok(store)
    }

    /// Open a private in-memory database (tests, dry runs) and run migrations.
    pub async fn open_in_memory() -> Result<Self, RepoError> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(|e| map_err(&e))?
            .foreign_keys(true);
        Self::connect(opts).await
    }

    async fn connect(opts: SqliteConnectOptions) -> Result<Self, RepoError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            // The single connection must never be recycled: an in-memory DB would vanish.
            .idle_timeout(None)
            .max_lifetime(None)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(opts)
            .await
            .map_err(|e| map_err(&e))?;
        migrator().run(&pool).await.map_err(|_| RepoError::Schema)?;
        Ok(Self { pool })
    }
}

impl InstrumentRepo for SqliteStore {
    async fn upsert_instruments(
        &self,
        rows: &[Instrument],
        fetched_on: NaiveDate,
    ) -> Result<UpsertSummary, RepoError> {
        let day = fmt_date(fetched_on);
        let mut tx = self.pool.begin().await.map_err(|e| map_err(&e))?;
        let latest: Option<String> =
            sqlx::query("SELECT MAX(fetched_on) AS latest FROM instrument_imports")
                .fetch_one(&mut *tx)
                .await
                .and_then(|r| r.try_get("latest"))
                .map_err(|e| map_err(&e))?;
        if latest.as_deref().is_some_and(|l| l > day.as_str()) {
            return Err(RepoError::Other); // stale import: never overwrite newer rows
        }
        let mut written = 0_u64;
        for r in rows {
            bind_row(UPSERT_INSTRUMENT, r, &day)?
                .execute(&mut *tx)
                .await
                .map_err(|e| map_err(&e))?;
            bind_row(UPSERT_SNAPSHOT, r, &day)?
                .execute(&mut *tx)
                .await
                .map_err(|e| map_err(&e))?;
            written += 1;
        }
        // The import log records the snapshot's size, so a re-import of the same day is stable.
        sqlx::query(
            "INSERT INTO instrument_imports (fetched_on, row_count) \
             VALUES (?1, (SELECT COUNT(*) FROM instrument_snapshots WHERE fetched_on = ?1)) \
             ON CONFLICT (fetched_on) DO UPDATE SET row_count = excluded.row_count",
        )
        .bind(day)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_err(&e))?;
        tx.commit().await.map_err(|e| map_err(&e))?;
        Ok(UpsertSummary { written })
    }

    async fn get(
        &self,
        exchange: &Exchange,
        tradingsymbol: &str,
    ) -> Result<Option<Instrument>, RepoError> {
        let row = sqlx::query(GET_INSTRUMENT)
            .bind(exchange.as_str().to_owned())
            .bind(norm(tradingsymbol))
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_err(&e))?;
        row.as_ref().map(to_instrument).transpose()
    }

    async fn next_expiry(
        &self,
        exchange: &Exchange,
        underlying: &str,
        kind: &InstrumentType,
        on_or_after: NaiveDate,
    ) -> Result<Option<NaiveDate>, RepoError> {
        let row = sqlx::query(
            "SELECT MIN(expiry) AS next FROM instruments \
             WHERE exchange = ? AND instrument_type = ? AND expiry IS NOT NULL AND expiry >= ? \
             AND UPPER(TRIM(name)) = ?",
        )
        .bind(exchange.as_str().to_owned())
        .bind(kind.as_str().to_owned())
        .bind(fmt_date(on_or_after))
        .bind(norm(underlying))
        .fetch_one(&self.pool)
        .await
        .map_err(|e| map_err(&e))?;
        let next: Option<String> = row.try_get("next").map_err(|e| map_err(&e))?;
        next.as_deref().map(parse_date).transpose()
    }

    async fn is_cash_fno(&self, symbol: &str) -> Result<bool, RepoError> {
        let row = sqlx::query(
            "SELECT (EXISTS (SELECT 1 FROM instruments WHERE instrument_type = 'EQ' \
                 AND UPPER(TRIM(tradingsymbol)) = ?1) \
             AND EXISTS (SELECT 1 FROM instruments WHERE instrument_type IN ('FUT', 'CE', 'PE') \
                 AND UPPER(TRIM(name)) = ?1)) AS eligible",
        )
        .bind(norm(symbol))
        .fetch_one(&self.pool)
        .await
        .map_err(|e| map_err(&e))?;
        let eligible: i64 = row.try_get("eligible").map_err(|e| map_err(&e))?;
        Ok(eligible != 0)
    }

    async fn last_fetched_on(&self) -> Result<Option<NaiveDate>, RepoError> {
        let row = sqlx::query("SELECT MAX(fetched_on) AS latest FROM instrument_imports")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| map_err(&e))?;
        let latest: Option<String> = row.try_get("latest").map_err(|e| map_err(&e))?;
        latest.as_deref().map(parse_date).transpose()
    }

    async fn ping(&self) -> Result<(), RepoError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| map_err(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).expect("decimal")
    }
    use sqlx::Row;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("valid date")
    }

    fn inst(
        sym: &str,
        name: &str,
        ex: Exchange,
        ty: InstrumentType,
        expiry: Option<NaiveDate>,
    ) -> Instrument {
        let is_opt = matches!(ty, InstrumentType::Call | InstrumentType::Put);
        Instrument {
            instrument_token: 1_000 + u64::try_from(sym.len()).unwrap_or(0),
            exchange_token: 7,
            tradingsymbol: sym.to_owned(),
            name: name.to_owned(),
            exchange: ex,
            instrument_type: ty,
            segment: "SEG".to_owned(),
            expiry,
            strike: is_opt.then(|| dec("100.50")),
            tick_size: dec("0.05"),
            lot_size: 25,
        }
    }

    async fn store() -> SqliteStore {
        SqliteStore::open_in_memory().await.expect("open")
    }

    async fn count(s: &SqliteStore, table: &'static str) -> i64 {
        let sql: &'static str = match table {
            "instruments" => "SELECT COUNT(*) AS n FROM instruments",
            "snapshots" => "SELECT COUNT(*) AS n FROM instrument_snapshots",
            _ => "SELECT COUNT(*) AS n FROM instrument_imports",
        };
        sqlx::query(sql)
            .fetch_one(&s.pool)
            .await
            .expect("count")
            .get("n")
    }

    #[tokio::test]
    async fn migrations_run_on_empty_db_and_ping_works() {
        let s = store().await;
        s.ping().await.expect("ping");
        assert_eq!(s.last_fetched_on().await.expect("q"), None);
        assert_eq!(count(&s, "instruments").await, 0);
    }

    #[tokio::test]
    async fn open_file_db_is_reopenable_and_migration_idempotent() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("t.db");
        let a = SqliteStore::open(&path).await.expect("open");
        a.upsert_instruments(
            &[inst(
                "RELIANCE",
                "RELIANCE",
                Exchange::Nse,
                InstrumentType::Equity,
                None,
            )],
            d(2026, 1, 5),
        )
        .await
        .expect("up");
        drop(a);
        let b = SqliteStore::open(&path).await.expect("reopen");
        assert!(
            b.get(&Exchange::Nse, "RELIANCE")
                .await
                .expect("g")
                .is_some()
        );
    }

    #[tokio::test]
    async fn upsert_twice_is_identical_and_records_fetched_on() {
        let s = store().await;
        let rows = vec![
            inst(
                "RELIANCE",
                "RELIANCE",
                Exchange::Nse,
                InstrumentType::Equity,
                None,
            ),
            inst(
                "RELFUT",
                "RELIANCE",
                Exchange::Nfo,
                InstrumentType::Future,
                Some(d(2026, 1, 29)),
            ),
        ];
        let day = d(2026, 1, 5);
        let first = s.upsert_instruments(&rows, day).await.expect("1");
        let a = s.get(&Exchange::Nse, "RELIANCE").await.expect("g");
        let second = s.upsert_instruments(&rows, day).await.expect("2");
        let b = s.get(&Exchange::Nse, "RELIANCE").await.expect("g");
        assert_eq!(first, second);
        assert_eq!(first.written, 2);
        assert_eq!(a, b);
        assert_eq!(a.as_ref(), rows.first());
        assert_eq!(count(&s, "instruments").await, 2);
        assert_eq!(count(&s, "snapshots").await, 2);
        assert_eq!(count(&s, "imports").await, 1);
        assert_eq!(s.last_fetched_on().await.expect("l"), Some(day));
    }

    #[tokio::test]
    async fn dated_snapshots_accumulate_and_master_updates() {
        let s = store().await;
        let mut r = inst(
            "RELIANCE",
            "RELIANCE",
            Exchange::Nse,
            InstrumentType::Equity,
            None,
        );
        s.upsert_instruments(std::slice::from_ref(&r), d(2026, 1, 5))
            .await
            .expect("1");
        r.lot_size = 50;
        s.upsert_instruments(std::slice::from_ref(&r), d(2026, 1, 6))
            .await
            .expect("2");
        assert_eq!(count(&s, "instruments").await, 1);
        assert_eq!(count(&s, "snapshots").await, 2);
        assert_eq!(count(&s, "imports").await, 2);
        let got = s
            .get(&Exchange::Nse, "RELIANCE")
            .await
            .expect("g")
            .expect("some");
        assert_eq!(got.lot_size, 50);
        assert_eq!(s.last_fetched_on().await.expect("l"), Some(d(2026, 1, 6)));
    }

    #[tokio::test]
    async fn get_missing_is_none_and_key_includes_exchange() {
        let s = store().await;
        s.upsert_instruments(
            &[inst("X", "X", Exchange::Nse, InstrumentType::Equity, None)],
            d(2026, 1, 5),
        )
        .await
        .expect("up");
        assert!(s.get(&Exchange::Bse, "X").await.expect("g").is_none());
        assert!(s.get(&Exchange::Nse, "Y").await.expect("g").is_none());
    }

    #[tokio::test]
    async fn decimal_scale_round_trips_exactly() {
        let s = store().await;
        let mut r = inst(
            "NIFTYCE",
            "NIFTY",
            Exchange::Nfo,
            InstrumentType::Call,
            Some(d(2026, 1, 29)),
        );
        r.tick_size = dec("0.050");
        r.strike = Some(dec("23500.00"));
        s.upsert_instruments(std::slice::from_ref(&r), d(2026, 1, 5))
            .await
            .expect("up");
        let got = s
            .get(&Exchange::Nfo, "NIFTYCE")
            .await
            .expect("g")
            .expect("some");
        assert_eq!(got.tick_size.scale(), 3);
        assert_eq!(got.tick_size.to_string(), "0.050");
        assert_eq!(
            got.strike.map(|k| k.to_string()).as_deref(),
            Some("23500.00")
        );
        let raw: String =
            sqlx::query("SELECT tick_size FROM instruments WHERE tradingsymbol = 'NIFTYCE'")
                .fetch_one(&s.pool)
                .await
                .expect("raw")
                .get("tick_size");
        assert_eq!(raw, "0.050");
        // 0.1 + 0.2 style values are exact, never routed through f64.
        let mut t = r.clone();
        t.tradingsymbol = "T".to_owned();
        t.tick_size = dec("0.30000000000000000001");
        s.upsert_instruments(&[t.clone()], d(2026, 1, 5))
            .await
            .expect("up");
        assert_eq!(s.get(&Exchange::Nfo, "T").await.expect("g"), Some(t));
    }

    #[tokio::test]
    async fn next_expiry_counts_the_boundary_day_and_filters_by_kind() {
        let s = store().await;
        let rows = vec![
            inst(
                "NIFTYA",
                "NIFTY",
                Exchange::Nfo,
                InstrumentType::Future,
                Some(d(2026, 1, 29)),
            ),
            inst(
                "NIFTYB",
                " nifty ",
                Exchange::Nfo,
                InstrumentType::Future,
                Some(d(2026, 2, 26)),
            ),
            inst(
                "NIFTYC",
                "NIFTY",
                Exchange::Nfo,
                InstrumentType::Call,
                Some(d(2026, 1, 8)),
            ),
            inst(
                "OTHER",
                "BANKNIFTY",
                Exchange::Nfo,
                InstrumentType::Future,
                Some(d(2026, 1, 28)),
            ),
        ];
        s.upsert_instruments(&rows, d(2026, 1, 5))
            .await
            .expect("up");
        let q = |from| s.next_expiry(&Exchange::Nfo, "Nifty", &InstrumentType::Future, from);
        // The expiry day itself counts (caller supplies the date).
        assert_eq!(q(d(2026, 1, 29)).await.expect("q"), Some(d(2026, 1, 29)));
        assert_eq!(q(d(2026, 1, 30)).await.expect("q"), Some(d(2026, 2, 26)));
        assert_eq!(q(d(2026, 1, 1)).await.expect("q"), Some(d(2026, 1, 29)));
        assert_eq!(q(d(2026, 2, 27)).await.expect("q"), None);
        let call = s
            .next_expiry(
                &Exchange::Nfo,
                "NIFTY",
                &InstrumentType::Call,
                d(2026, 1, 1),
            )
            .await
            .expect("q");
        assert_eq!(call, Some(d(2026, 1, 8)));
        let other_ex = s
            .next_expiry(
                &Exchange::Bfo,
                "NIFTY",
                &InstrumentType::Future,
                d(2026, 1, 1),
            )
            .await
            .expect("q");
        assert_eq!(other_ex, None);
    }

    #[tokio::test]
    async fn cash_fno_needs_equity_and_derivative() {
        let s = store().await;
        let rows = vec![
            inst(
                "RELIANCE",
                "RELIANCE INDUSTRIES",
                Exchange::Nse,
                InstrumentType::Equity,
                None,
            ),
            inst(
                "RELFUT",
                "RELIANCE IND",
                Exchange::Nfo,
                InstrumentType::Future,
                Some(d(2026, 1, 29)),
            ),
            inst(
                "INFY",
                "INFOSYS",
                Exchange::Nse,
                InstrumentType::Equity,
                None,
            ),
            inst(
                "NIFTYFUT",
                "NIFTY",
                Exchange::Nfo,
                InstrumentType::Future,
                Some(d(2026, 1, 29)),
            ),
            inst("TCS", "TCS", Exchange::Nse, InstrumentType::Equity, None),
            inst(
                "TCSFUT",
                "TCS",
                Exchange::Nfo,
                InstrumentType::Future,
                Some(d(2026, 1, 29)),
            ),
        ];
        s.upsert_instruments(&rows, d(2026, 1, 5))
            .await
            .expect("up");
        // Equity row exists but the derivative underlying name differs: not eligible.
        assert!(!s.is_cash_fno("RELIANCE").await.expect("q"));
        assert!(!s.is_cash_fno("INFY").await.expect("q"), "no derivative");
        assert!(
            !s.is_cash_fno("NIFTY").await.expect("q"),
            "index has no cash equity row"
        );
        assert!(s.is_cash_fno(" tcs ").await.expect("q"));
        assert!(!s.is_cash_fno("UNKNOWN").await.expect("q"));
    }

    #[tokio::test]
    async fn empty_upsert_is_ok() {
        let s = store().await;
        let r = s.upsert_instruments(&[], d(2026, 1, 5)).await.expect("up");
        assert_eq!(r.written, 0);
    }

    #[tokio::test]
    async fn oversize_token_is_an_error_not_a_panic() {
        let s = store().await;
        let mut r = inst("X", "X", Exchange::Nse, InstrumentType::Equity, None);
        r.instrument_token = u64::MAX;
        let e = s
            .upsert_instruments(&[r], d(2026, 1, 5))
            .await
            .expect_err("err");
        assert_eq!(e, RepoError::Other);
        assert_eq!(count(&s, "instruments").await, 0);
    }

    #[tokio::test]
    async fn corrupt_stored_value_maps_to_corrupt() {
        let s = store().await;
        s.upsert_instruments(
            &[inst("X", "X", Exchange::Nse, InstrumentType::Equity, None)],
            d(2026, 1, 5),
        )
        .await
        .expect("up");
        sqlx::query("UPDATE instruments SET tick_size = 'abc'")
            .execute(&s.pool)
            .await
            .expect("upd");
        assert_eq!(
            s.get(&Exchange::Nse, "X").await.expect_err("e"),
            RepoError::Corrupt
        );
    }

    #[tokio::test]
    async fn closed_pool_maps_to_unavailable() {
        let s = store().await;
        s.pool.close().await;
        assert_eq!(s.ping().await.expect_err("e"), RepoError::Unavailable);
    }

    #[tokio::test]
    async fn older_import_is_rejected_and_changes_nothing() {
        let s = store().await;
        let mut r = inst("X", "X", Exchange::Nse, InstrumentType::Equity, None);
        s.upsert_instruments(std::slice::from_ref(&r), d(2026, 1, 6))
            .await
            .expect("new");
        r.lot_size = 99;
        let e = s
            .upsert_instruments(&[r], d(2026, 1, 5))
            .await
            .expect_err("old");
        assert_eq!(e, RepoError::Other);
        let got = s.get(&Exchange::Nse, "X").await.expect("g").expect("some");
        assert_eq!(got.lot_size, 25);
        assert_eq!(s.last_fetched_on().await.expect("l"), Some(d(2026, 1, 6)));
        assert_eq!(count(&s, "snapshots").await, 1);
    }

    #[tokio::test]
    async fn tradingsymbol_key_is_trimmed_and_case_normalised() {
        let s = store().await;
        let r = inst(
            " reliance ",
            "RELIANCE",
            Exchange::Nse,
            InstrumentType::Equity,
            None,
        );
        s.upsert_instruments(&[r], d(2026, 1, 5)).await.expect("up");
        let got = s
            .get(&Exchange::Nse, "Reliance")
            .await
            .expect("g")
            .expect("some");
        assert_eq!(got.tradingsymbol, "RELIANCE");
        assert!(
            s.get(&Exchange::Nse, " RELIANCE ")
                .await
                .expect("g")
                .is_some()
        );
        assert_eq!(count(&s, "instruments").await, 1);
    }

    #[tokio::test]
    async fn pool_never_recycles_its_connection() {
        let s = store().await;
        assert_eq!(s.pool.options().get_idle_timeout(), None);
        assert_eq!(s.pool.options().get_max_lifetime(), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn database_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("p.db");
        let _s = SqliteStore::open(&path).await.expect("open");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
