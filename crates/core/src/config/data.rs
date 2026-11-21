//! `data.toml`: Kite and NSE endpoints, host allowlists, timeouts, rate limits, interval
//! spans, staleness windows and history depth. Everything here is a fact about an outside
//! service, so it lives in the file with `source` and `verified_on`, never in code.
//!
//! The loader is the first line of defence for the network adapters: every URL must be
//! `https`, bare of userinfo and port, and its host must be on the section's allowlist.

use chrono::NaiveDate;
use serde::Deserialize;

use super::{ConfigError, ConfigFile, impl_header};
use crate::candle::Interval;

const FILE: &str = "data.toml";
/// Longest allowed timeout, in seconds (every outbound call needs a bounded one).
const MAX_TIMEOUT_SECS: u32 = 300;
/// Largest response body an adapter may be configured to read (64 MiB).
const MAX_RESPONSE_BYTES_CEILING: u64 = 64 * 1024 * 1024;
/// Most hosts in one allowlist.
const MAX_ALLOWED_HOSTS: usize = 8;

/// Maximum days of data per historical call, by interval (Kite protocol limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpanDays {
    pub min1: u32,
    pub min3: u32,
    pub min5: u32,
    pub min10: u32,
    pub min15: u32,
    pub min30: u32,
    pub min60: u32,
    pub day: u32,
}

impl SpanDays {
    /// Days one historical call may span for `interval`.
    #[must_use]
    pub const fn for_interval(&self, interval: Interval) -> u32 {
        match interval {
            Interval::Min1 => self.min1,
            Interval::Min3 => self.min3,
            Interval::Min5 => self.min5,
            Interval::Min10 => self.min10,
            Interval::Min15 => self.min15,
            Interval::Min30 => self.min30,
            Interval::Min60 => self.min60,
            Interval::Day => self.day,
        }
    }

    fn check(&self, file: &'static str) -> Result<(), ConfigError> {
        for (key, v) in [
            ("min1", self.min1),
            ("min3", self.min3),
            ("min5", self.min5),
            ("min10", self.min10),
            ("min15", self.min15),
            ("min30", self.min30),
            ("min60", self.min60),
            ("day", self.day),
        ] {
            positive(file, &format!("kite.span_days.{key}"), v, 10_000)?;
        }
        Ok(())
    }
}

/// `[kite]`: the broker's read-only API.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KiteSection {
    /// REST base, e.g. `https://api.kite.trade` (no trailing path).
    pub api_base: String,
    /// Login page the owner is sent to.
    pub login_url: String,
    /// Value of the `X-Kite-Version` header.
    pub api_version: String,
    /// Hosts this client may call or redirect to; nothing else is ever contacted.
    pub allowed_hosts: Vec<String>,
    pub connect_timeout_secs: u32,
    pub request_timeout_secs: u32,
    pub max_response_bytes: u64,
    pub historical_per_sec: u32,
    pub default_per_sec: u32,
    pub max_quote_batch: u32,
    /// The access token expires at this IST wall-clock time on the next day.
    pub token_expiry_hour_ist: u32,
    pub token_expiry_minute_ist: u32,
    pub span_days: SpanDays,
}

/// `[nse]`: exchange archive downloads. Templates hold `{yyyymmdd}` or `{ddmmyyyy}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NseSection {
    pub allowed_hosts: Vec<String>,
    pub bhavcopy_url_template: String,
    pub banlist_url_template: String,
    pub request_timeout_secs: u32,
    pub max_download_bytes: u64,
}

/// `[staleness]`: when fetched data stops being usable.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StalenessSection {
    pub bhavcopy_max_age_days: u32,
    pub ban_list_max_age_hours: u32,
    pub quote_max_age_secs: u32,
}

/// `[history]`: how much to backfill per interval.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistorySection {
    pub daily_years: u32,
    pub min15_years: u32,
}

/// `data.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub placeholder: bool,
    #[serde(default)]
    pub verified_on: Option<NaiveDate>,
    pub source: String,
    /// Key paths whose value could not be checked against the provider's docs.
    #[serde(default)]
    pub unverified: Vec<String>,
    pub kite: Option<KiteSection>,
    pub nse: Option<NseSection>,
    pub staleness: Option<StalenessSection>,
    pub history: Option<HistorySection>,
}

impl DataConfig {
    /// The `[kite]` table; a missing table is an error, never defaults.
    pub fn kite(&self) -> Result<&KiteSection, ConfigError> {
        self.kite
            .as_ref()
            .ok_or_else(|| ConfigError::invalid(FILE, "kite", "table is required"))
    }

    /// The `[nse]` table.
    pub fn nse(&self) -> Result<&NseSection, ConfigError> {
        self.nse
            .as_ref()
            .ok_or_else(|| ConfigError::invalid(FILE, "nse", "table is required"))
    }

    /// The `[staleness]` table.
    pub fn staleness(&self) -> Result<&StalenessSection, ConfigError> {
        self.staleness
            .as_ref()
            .ok_or_else(|| ConfigError::invalid(FILE, "staleness", "table is required"))
    }

    /// The `[history]` table.
    pub fn history(&self) -> Result<&HistorySection, ConfigError> {
        self.history
            .as_ref()
            .ok_or_else(|| ConfigError::invalid(FILE, "history", "table is required"))
    }
}

fn positive(file: &'static str, key: &str, v: u32, max: u32) -> Result<(), ConfigError> {
    if v == 0 || v > max {
        return Err(ConfigError::invalid(
            file,
            key,
            format!("must be within 1 and {max}"),
        ));
    }
    Ok(())
}

/// Sections and keys an `unverified` entry may name (a section name covers all its keys).
const KNOWN_KEYS: &[&str] = &[
    "kite",
    "nse",
    "staleness",
    "history",
    "kite.api_base",
    "kite.login_url",
    "kite.api_version",
    "kite.allowed_hosts",
    "kite.connect_timeout_secs",
    "kite.request_timeout_secs",
    "kite.max_response_bytes",
    "kite.historical_per_sec",
    "kite.default_per_sec",
    "kite.max_quote_batch",
    "kite.token_expiry_hour_ist",
    "kite.token_expiry_minute_ist",
    "kite.span_days",
    "nse.allowed_hosts",
    "nse.bhavcopy_url_template",
    "nse.banlist_url_template",
    "nse.request_timeout_secs",
    "nse.max_download_bytes",
    "staleness.bhavcopy_max_age_days",
    "staleness.ban_list_max_age_hours",
    "staleness.quote_max_age_secs",
    "history.daily_years",
    "history.min15_years",
];

fn known_key(k: &str) -> bool {
    KNOWN_KEYS.contains(&k)
        || k.strip_prefix("kite.span_days.").is_some_and(|d| {
            [
                "min1", "min3", "min5", "min10", "min15", "min30", "min60", "day",
            ]
            .contains(&d)
        })
}

fn valid_host(h: &str) -> bool {
    // An all-numeric last label is an IP literal; `localhost` has no dot. Both are refused.
    let last_is_numeric = h
        .rsplit('.')
        .next()
        .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()));
    !h.is_empty()
        && h.len() <= 253
        && h.contains('.')
        && !last_is_numeric
        && !h.ends_with(".localhost")
        && h.split('.').all(|l| {
            !l.is_empty()
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn check_hosts(file: &'static str, key: &str, hosts: &[String]) -> Result<(), ConfigError> {
    if hosts.is_empty() || hosts.len() > MAX_ALLOWED_HOSTS {
        return Err(ConfigError::invalid(
            file,
            key,
            format!("must list 1 to {MAX_ALLOWED_HOSTS} hosts"),
        ));
    }
    if let Some(h) = hosts.iter().find(|h| !valid_host(h)) {
        return Err(ConfigError::invalid(
            file,
            key,
            format!("{h:?} is not a lowercase bare host name"),
        ));
    }
    Ok(())
}

/// Require `https://<host>/...` with no userinfo or port and a host on `allowed`.
fn check_url(
    file: &'static str,
    key: &str,
    url: &str,
    allowed: &[String],
) -> Result<(), ConfigError> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(ConfigError::invalid(file, key, "must start with https://"));
    };
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ConfigError::invalid(
            file,
            key,
            "must not contain whitespace",
        ));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains(['@', ':', '\\']) {
        return Err(ConfigError::invalid(
            file,
            key,
            "host must not carry userinfo or a port",
        ));
    }
    if !allowed.iter().any(|h| h == authority) {
        return Err(ConfigError::invalid(
            file,
            key,
            "host is not on the allowlist",
        ));
    }
    Ok(())
}

fn check_bytes(file: &'static str, key: &str, v: u64) -> Result<(), ConfigError> {
    if v == 0 || v > MAX_RESPONSE_BYTES_CEILING {
        return Err(ConfigError::invalid(
            file,
            key,
            format!("must be within 1 and {MAX_RESPONSE_BYTES_CEILING}"),
        ));
    }
    Ok(())
}

impl KiteSection {
    fn check(&self, file: &'static str) -> Result<(), ConfigError> {
        check_hosts(file, "kite.allowed_hosts", &self.allowed_hosts)?;
        check_url(file, "kite.api_base", &self.api_base, &self.allowed_hosts)?;
        let after_scheme = self.api_base.strip_prefix("https://").unwrap_or_default();
        if after_scheme
            .split_once('/')
            .is_some_and(|(_, p)| !p.is_empty())
            || after_scheme.contains(['?', '#'])
        {
            return Err(ConfigError::invalid(
                file,
                "kite.api_base",
                "must end at the host (no path, query or fragment)",
            ));
        }
        check_url(file, "kite.login_url", &self.login_url, &self.allowed_hosts)?;
        if self.api_version.trim().is_empty() || self.api_version.len() > 10 {
            return Err(ConfigError::invalid(
                file,
                "kite.api_version",
                "must be 1 to 10 characters",
            ));
        }
        for (key, v) in [
            ("kite.connect_timeout_secs", self.connect_timeout_secs),
            ("kite.request_timeout_secs", self.request_timeout_secs),
        ] {
            positive(file, key, v, MAX_TIMEOUT_SECS)?;
        }
        check_bytes(file, "kite.max_response_bytes", self.max_response_bytes)?;
        positive(
            file,
            "kite.historical_per_sec",
            self.historical_per_sec,
            1000,
        )?;
        positive(file, "kite.default_per_sec", self.default_per_sec, 1000)?;
        positive(file, "kite.max_quote_batch", self.max_quote_batch, 10_000)?;
        if self.token_expiry_hour_ist > 23 {
            return Err(ConfigError::invalid(
                file,
                "kite.token_expiry_hour_ist",
                "must be within 0 and 23",
            ));
        }
        if self.token_expiry_minute_ist > 59 {
            return Err(ConfigError::invalid(
                file,
                "kite.token_expiry_minute_ist",
                "must be within 0 and 59",
            ));
        }
        if self.connect_timeout_secs > self.request_timeout_secs {
            return Err(ConfigError::invalid(
                file,
                "kite.connect_timeout_secs",
                "must not exceed request_timeout_secs",
            ));
        }
        if self.historical_per_sec > self.default_per_sec {
            return Err(ConfigError::invalid(
                file,
                "kite.historical_per_sec",
                "must not exceed default_per_sec",
            ));
        }
        self.span_days.check(file)
    }
}

impl NseSection {
    fn check(&self, file: &'static str) -> Result<(), ConfigError> {
        check_hosts(file, "nse.allowed_hosts", &self.allowed_hosts)?;
        check_url(
            file,
            "nse.bhavcopy_url_template",
            &self.bhavcopy_url_template,
            &self.allowed_hosts,
        )?;
        check_url(
            file,
            "nse.banlist_url_template",
            &self.banlist_url_template,
            &self.allowed_hosts,
        )?;
        for (key, url, placeholder) in [
            (
                "nse.bhavcopy_url_template",
                &self.bhavcopy_url_template,
                "{yyyymmdd}",
            ),
            (
                "nse.banlist_url_template",
                &self.banlist_url_template,
                "{ddmmyyyy}",
            ),
        ] {
            if !url.contains(placeholder) {
                return Err(ConfigError::invalid(
                    file,
                    key,
                    format!("must contain {placeholder}"),
                ));
            }
        }
        positive(
            file,
            "nse.request_timeout_secs",
            self.request_timeout_secs,
            MAX_TIMEOUT_SECS,
        )?;
        check_bytes(file, "nse.max_download_bytes", self.max_download_bytes)
    }
}

impl ConfigFile for DataConfig {
    const FILE: &'static str = FILE;
    impl_header!();

    fn validate(&self) -> Result<(), ConfigError> {
        if self.placeholder {
            return Ok(());
        }
        self.kite()?.check(FILE)?;
        self.nse()?.check(FILE)?;
        let s = self.staleness()?;
        positive(
            FILE,
            "staleness.bhavcopy_max_age_days",
            s.bhavcopy_max_age_days,
            3650,
        )?;
        positive(
            FILE,
            "staleness.ban_list_max_age_hours",
            s.ban_list_max_age_hours,
            24 * 30,
        )?;
        positive(
            FILE,
            "staleness.quote_max_age_secs",
            s.quote_max_age_secs,
            86_400,
        )?;
        let h = self.history()?;
        positive(FILE, "history.daily_years", h.daily_years, 30)?;
        positive(FILE, "history.min15_years", h.min15_years, 30)?;
        if let Some(k) = self.unverified.iter().find(|k| !known_key(k)) {
            return Err(ConfigError::invalid(
                FILE,
                "unverified",
                format!("{k:?} is not a key or section of this file"),
            ));
        }
        if self.unverified.len() > 200 || self.unverified.iter().any(|k| k.len() > 100) {
            return Err(ConfigError::invalid(
                FILE,
                "unverified",
                "list is too large",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_str;

    const GOOD: &str = r#"
schema_version = 1
verified_on = "2026-11-21"
source = "test"
unverified = ["nse.banlist_url_template"]
[kite]
api_base = "https://api.kite.trade"
login_url = "https://kite.zerodha.com/connect/login"
api_version = "3"
allowed_hosts = ["api.kite.trade", "kite.zerodha.com"]
connect_timeout_secs = 5
request_timeout_secs = 20
max_response_bytes = 8388608
historical_per_sec = 3
default_per_sec = 10
max_quote_batch = 500
token_expiry_hour_ist = 6
token_expiry_minute_ist = 0
[kite.span_days]
min1 = 60
min3 = 100
min5 = 100
min10 = 100
min15 = 200
min30 = 200
min60 = 400
day = 2000
[nse]
allowed_hosts = ["nsearchives.nseindia.com"]
bhavcopy_url_template = "https://nsearchives.nseindia.com/content/cm/x_{yyyymmdd}.zip"
banlist_url_template = "https://nsearchives.nseindia.com/content/fo/fo_secban_{ddmmyyyy}.csv"
request_timeout_secs = 30
max_download_bytes = 33554432
[staleness]
bhavcopy_max_age_days = 4
ban_list_max_age_hours = 24
quote_max_age_secs = 10
[history]
daily_years = 3
min15_years = 1
"#;

    fn with(from: &str, to: &str) -> String {
        assert!(GOOD.contains(from), "test bug: {from}");
        GOOD.replacen(from, to, 1)
    }

    fn err_of(text: &str) -> String {
        parse_str::<DataConfig>(text).unwrap_err().to_string()
    }

    #[test]
    fn good_file_loads_and_exposes_spans() {
        let c = parse_str::<DataConfig>(GOOD).unwrap();
        assert_eq!(
            c.kite().unwrap().span_days.for_interval(Interval::Day),
            2000
        );
        assert_eq!(c.kite().unwrap().span_days.for_interval(Interval::Min1), 60);
        assert_eq!(c.nse().unwrap().request_timeout_secs, 30);
        assert_eq!(c.history().unwrap().daily_years, 3);
        assert_eq!(c.staleness().unwrap().quote_max_age_secs, 10);
    }

    #[test]
    fn missing_verified_on_is_rejected() {
        let e = err_of(&with("verified_on = \"2026-11-21\"\n", ""));
        assert!(e.contains("verified_on"), "{e}");
    }

    #[test]
    fn non_https_urls_are_rejected() {
        let e = err_of(&with(
            "\"https://api.kite.trade\"",
            "\"http://api.kite.trade\"",
        ));
        assert!(e.contains("kite.api_base") && e.contains("https"), "{e}");
        let e = err_of(&with(
            "\"https://nsearchives.nseindia.com/content/fo",
            "\"ftp://nsearchives.nseindia.com/content/fo",
        ));
        assert!(e.contains("nse.banlist_url_template"), "{e}");
    }

    #[test]
    fn off_allowlist_hosts_are_rejected() {
        let e = err_of(&with(
            "\"https://api.kite.trade\"",
            "\"https://evil.example.com\"",
        ));
        assert!(e.contains("allowlist"), "{e}");
        let e = err_of(&with(
            "https://nsearchives.nseindia.com/content/cm",
            "https://nseindia.com/content/cm",
        ));
        assert!(e.contains("nse.bhavcopy_url_template"), "{e}");
    }

    #[test]
    fn host_tricks_are_rejected() {
        for bad in [
            "https://api.kite.trade@evil.com",
            "https://api.kite.trade:8443",
            "https://api.kite.trade.evil.com",
        ] {
            let e = err_of(&with("\"https://api.kite.trade\"", &format!("\"{bad}\"")));
            assert!(e.contains("kite.api_base"), "{bad}: {e}");
        }
        let e = err_of(&with("[\"api.kite.trade\", ", "[\"API.kite.trade\", "));
        assert!(e.contains("allowed_hosts"), "{e}");
        let e = err_of(&with("[\"api.kite.trade\", ", "[\"\", "));
        assert!(e.contains("allowed_hosts"), "{e}");
    }

    #[test]
    fn zero_timeout_or_limit_is_rejected() {
        for key in [
            "connect_timeout_secs = 5",
            "request_timeout_secs = 20",
            "historical_per_sec = 3",
            "default_per_sec = 10",
            "max_quote_batch = 500",
            "request_timeout_secs = 30",
            "bhavcopy_max_age_days = 4",
            "quote_max_age_secs = 10",
            "daily_years = 3",
        ] {
            let name = key.split(' ').next().unwrap();
            let e = err_of(&with(key, &format!("{name} = 0")));
            assert!(e.contains(name), "{key}: {e}");
        }
        let e = err_of(&with(
            "request_timeout_secs = 20",
            "request_timeout_secs = 301",
        ));
        assert!(e.contains("kite.request_timeout_secs"), "{e}");
    }

    #[test]
    fn zero_interval_span_is_rejected() {
        let e = err_of(&with("min15 = 200", "min15 = 0"));
        assert!(e.contains("kite.span_days.min15"), "{e}");
    }

    #[test]
    fn size_caps_and_time_of_day_are_bounded() {
        let e = err_of(&with(
            "max_response_bytes = 8388608",
            "max_response_bytes = 0",
        ));
        assert!(e.contains("max_response_bytes"), "{e}");
        let e = err_of(&with(
            "max_download_bytes = 33554432",
            "max_download_bytes = 99999999999",
        ));
        assert!(e.contains("max_download_bytes"), "{e}");
        let e = err_of(&with(
            "token_expiry_hour_ist = 6",
            "token_expiry_hour_ist = 24",
        ));
        assert!(e.contains("token_expiry"), "{e}");
    }

    #[test]
    fn missing_table_and_unknown_key_are_rejected() {
        let cut = GOOD.split("[history]").next().unwrap();
        assert!(err_of(cut).contains("history"));
        let e = err_of(&with(
            "api_version = \"3\"",
            "api_version = \"3\"\nextra = 1",
        ));
        assert!(e.contains("extra"), "{e}");
    }

    #[test]
    fn templates_need_their_date_placeholder_and_api_base_no_path() {
        let e = err_of(&with("x_{yyyymmdd}.zip", "x.zip"));
        assert!(e.contains("{yyyymmdd}"), "{e}");
        let e = err_of(&with("fo_secban_{ddmmyyyy}.csv", "fo_secban.csv"));
        assert!(e.contains("{ddmmyyyy}"), "{e}");
        let e = err_of(&with(
            "\"https://api.kite.trade\"",
            "\"https://api.kite.trade/v3\"",
        ));
        assert!(e.contains("kite.api_base"), "{e}");
        assert!(
            parse_str::<DataConfig>(&with(
                "\"https://api.kite.trade\"",
                "\"https://api.kite.trade/\""
            ))
            .is_ok()
        );
    }

    #[test]
    fn unverified_entries_must_be_real_keys() {
        let e = err_of(&with(
            "[\"nse.banlist_url_template\"]",
            "[\"kite.api_bse\"]",
        ));
        assert!(e.contains("kite.api_bse"), "{e}");
        let ok = with(
            "[\"nse.banlist_url_template\"]",
            "[\"kite\", \"kite.span_days.min1\"]",
        );
        assert!(parse_str::<DataConfig>(&ok).is_ok());
    }

    #[test]
    fn minute_key_is_named_and_cross_limits_hold() {
        let e = err_of(&with(
            "token_expiry_minute_ist = 0",
            "token_expiry_minute_ist = 60",
        ));
        assert!(e.contains("token_expiry_minute_ist"), "{e}");
        let e = err_of(&with(
            "connect_timeout_secs = 5",
            "connect_timeout_secs = 21",
        ));
        assert!(e.contains("connect_timeout_secs"), "{e}");
        let e = err_of(&with("historical_per_sec = 3", "historical_per_sec = 11"));
        assert!(e.contains("historical_per_sec"), "{e}");
    }

    #[test]
    fn ip_literals_and_localhost_are_not_hosts() {
        for bad in ["127.0.0.1", "10.0.0.1", "foo.localhost"] {
            let e = err_of(&with("[\"api.kite.trade\", ", &format!("[\"{bad}\", ")));
            assert!(e.contains("allowed_hosts"), "{bad}: {e}");
        }
    }

    #[test]
    fn placeholder_flag_with_verified_on_is_rejected() {
        let e = err_of(&with(
            "source = \"test\"",
            "source = \"test\"\nplaceholder = true",
        ));
        assert!(e.contains("verified_on"), "{e}");
    }
}
