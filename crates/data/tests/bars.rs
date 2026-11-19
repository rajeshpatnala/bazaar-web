//! Bar builder over the synthetic 1-minute session fixture and the fixture calendar
//! (session 10:00-14:00 IST, fictional). Expectations are computed independently of the builder.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use bazaar_core::calendar::MarketCalendar;
use bazaar_core::candle::{Candle, Interval};
use bazaar_core::config::{CalendarConfig, load};
use bazaar_core::segment::{InstrumentKind, Segment};
use bazaar_data::bars::{BarBuilder, BarError, BarStats, MAX_INPUT_BARS, aggregate};
use chrono::{DateTime, Duration, TimeZone, Timelike};
use chrono_tz::{Asia::Kolkata, Tz};
use rust_decimal::Decimal;

const SEG: Segment = Segment::Equity;
const KIND: InstrumentKind = InstrumentKind::CashNonFno;

fn cal() -> MarketCalendar {
    let cfg: CalendarConfig = load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/config"
    )))
    .unwrap();
    MarketCalendar::from_config(&cfg)
}

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

type Row = (DateTime<Tz>, [Decimal; 4], u64);

fn rows() -> Vec<Row> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/data/candles_1m_session.csv"
    );
    let mut rdr = csv::Reader::from_path(path).unwrap();
    rdr.records()
        .map(|r| {
            let r = r.unwrap();
            let t = DateTime::parse_from_rfc3339(&r[0])
                .unwrap()
                .with_timezone(&Kolkata);
            (
                t,
                [d(&r[1]), d(&r[2]), d(&r[3]), d(&r[4])],
                r[5].parse().unwrap(),
            )
        })
        .collect()
}

fn mk(row: &Row, interval: Interval) -> Candle {
    Candle::new(row.0, interval, row.1, row.2, false).unwrap()
}

fn candles(rows: &[Row]) -> Vec<Candle> {
    rows.iter().map(|r| mk(r, Interval::Min1)).collect()
}

fn run(target: Interval, input: &[Candle]) -> (Vec<Candle>, BarStats) {
    aggregate(&cal(), SEG, KIND, target, input).unwrap()
}

/// Naive reference: group rows by `(minute index) / n` and fold.
fn naive(rows: &[Row], n: usize) -> Vec<(DateTime<Tz>, [Decimal; 4], u64)> {
    rows.chunks(n)
        .map(|ch| {
            let hi = ch.iter().map(|r| r.1[1]).max().unwrap();
            let lo = ch.iter().map(|r| r.1[2]).min().unwrap();
            let vol = ch.iter().map(|r| r.2).sum();
            (
                ch[ch.len() - 1].0,
                [ch[0].1[0], hi, lo, ch[ch.len() - 1].1[3]],
                vol,
            )
        })
        .collect()
}

fn check_against_naive(target: Interval, n: usize) {
    let rows = rows();
    let (bars, stats) = run(target, &candles(&rows));
    let want = naive(&rows, n);
    assert_eq!(bars.len(), want.len());
    for (b, w) in bars.iter().zip(&want) {
        assert_eq!(b.close_time(), w.0);
        assert_eq!(b.interval(), target);
        assert_eq!([b.open(), b.high(), b.low(), b.close()], w.1);
        assert_eq!(b.volume(), w.2);
    }
    assert_eq!(stats.emitted, want.len() as u64);
    assert_eq!(stats.accepted, 240);
    assert_eq!(stats.incomplete_buckets, 0);
}

#[test]
fn fixture_is_one_full_session() {
    let r = rows();
    assert_eq!(r.len(), 240);
    assert_eq!((r[0].0.hour(), r[0].0.minute()), (10, 1));
    assert_eq!((r[239].0.hour(), r[239].0.minute()), (14, 0));
}

#[test]
fn five_minute_first_bar_hand_computed() {
    let (bars, _) = run(Interval::Min5, &candles(&rows()));
    assert_eq!(bars.len(), 48);
    let b = &bars[0];
    // Minutes 10:01..10:05: open of first = 100.00; highs 100.05,100.35,100.65,100.50,100.45 -> 100.65;
    // lows 100.00,100.10,100.20,99.95,100.05 -> 99.95; close of last = 100.35;
    // volume 100+107+114+121+128 = 570. Bar closes at 10:05, aligned to the 10:00 session open.
    assert_eq!(b.open(), d("100.00"));
    assert_eq!(b.high(), d("100.65"));
    assert_eq!(b.low(), d("99.95"));
    assert_eq!(b.close(), d("100.35"));
    assert_eq!(b.volume(), 570);
    assert_eq!((b.close_time().hour(), b.close_time().minute()), (10, 5));
}

#[test]
fn matches_naive_reference_for_each_interval() {
    check_against_naive(Interval::Min3, 3);
    check_against_naive(Interval::Min5, 5);
    check_against_naive(Interval::Min10, 10);
    check_against_naive(Interval::Min15, 15);
    check_against_naive(Interval::Min30, 30);
    check_against_naive(Interval::Min60, 60);
    check_against_naive(Interval::Day, 240);
}

#[test]
fn daily_bar_spans_session() {
    let rows = rows();
    let (bars, _) = run(Interval::Day, &candles(&rows));
    assert_eq!(bars.len(), 1);
    assert_eq!(bars[0].open(), rows[0].1[0]);
    assert_eq!(bars[0].close(), rows[239].1[3]);
    assert_eq!(bars[0].volume(), rows.iter().map(|r| r.2).sum::<u64>());
    assert_eq!(bars[0].close_time().hour(), 14);
}

#[test]
fn forming_bucket_is_never_emitted() {
    let all = candles(&rows());
    let cal = cal();
    let mut b = BarBuilder::new(&cal, SEG, KIND, Interval::Min5).unwrap();
    let emitted: Vec<_> = all[..7].iter().filter_map(|c| b.push(c)).collect();
    assert_eq!(emitted.len(), 1);
    assert_eq!(b.stats().accepted, 7);
    // Day bar: nothing until the final minute.
    let mut b = BarBuilder::new(&cal, SEG, KIND, Interval::Day).unwrap();
    assert!(all[..239].iter().all(|c| b.push(c).is_none()));
    assert!(b.push(&all[239]).is_some());
}

#[test]
fn gap_drops_bucket_and_counts() {
    let mut r = rows();
    r.remove(2); // 10:03 missing
    let (bars, s) = run(Interval::Min5, &candles(&r));
    assert_eq!(bars.len(), 47);
    assert_eq!(s.incomplete_buckets, 1);
    assert_eq!(s.gap_minutes, 1);
    assert_eq!(s.accepted, 239);
}

#[test]
fn out_of_order_and_duplicates_are_skipped() {
    let r = rows();
    let mut input = candles(&r);
    input.insert(3, input[1].clone()); // late repeat of 10:02
    input.insert(6, input[4].clone()); // duplicate 10:04 after 10:05
    let (bars, s) = run(Interval::Min5, &input);
    assert_eq!(s.out_of_order, 2);
    assert_eq!(bars.len(), 48);
    assert_eq!(bars[0].volume(), 570);
}

#[test]
fn outside_session_and_non_trading_days_are_skipped() {
    let r = rows();
    let mut pre = r[0];
    pre.0 -= Duration::minutes(1); // closes 10:00: minute 09:59 is before the open
    let mut post = r[239];
    post.0 += Duration::minutes(1); // 14:01 is after the normal session
    let mut sat = r[0];
    sat.0 += Duration::days(5); // 2026-03-07, a Saturday
    let mut input = vec![mk(&pre, Interval::Min1)];
    input.extend(candles(&r));
    input.push(mk(&post, Interval::Min1));
    input.push(mk(&sat, Interval::Min1));
    let (bars, s) = run(Interval::Min60, &input);
    assert_eq!(s.out_of_session, 3);
    assert_eq!(bars.len(), 4);
}

#[test]
fn malformed_input_is_counted() {
    let r = rows();
    let wrong_interval = mk(&r[0], Interval::Min5);
    let mut off = r[0];
    off.0 += Duration::seconds(30);
    let off = mk(&off, Interval::Min1);
    let (bars, s) = run(Interval::Min5, &[wrong_interval, off]);
    assert_eq!(bars.len(), 0);
    assert_eq!(s.malformed, 2);
    assert_eq!(s.accepted, 0);
}

#[test]
fn two_sessions_make_two_daily_bars() {
    let r = rows();
    let mut input = candles(&r);
    let next: Vec<Row> = r
        .iter()
        .map(|x| (x.0 + Duration::days(1), x.1, x.2))
        .collect();
    input.extend(candles(&next));
    let (bars, s) = run(Interval::Day, &input);
    assert_eq!(bars.len(), 2);
    assert_eq!(s.emitted, 2);
    assert_eq!(
        bars[1].close_time(),
        bars[0].close_time() + Duration::days(1)
    );
}

#[test]
fn volume_overflow_drops_bucket() {
    let r = rows();
    let mut a = r[0];
    a.2 = u64::MAX;
    let mut b = r[1];
    b.2 = 1;
    let input = vec![
        mk(&a, Interval::Min1),
        mk(&b, Interval::Min1),
        mk(&r[2], Interval::Min1),
    ];
    let (bars, s) = run(Interval::Min3, &input);
    assert_eq!(bars.len(), 0);
    assert_eq!(s.overflow_buckets, 1);
}

#[test]
fn rejects_bad_target_and_oversized_input() {
    let cal = cal();
    assert_eq!(
        aggregate(&cal, SEG, KIND, Interval::Min1, &[]).err(),
        Some(BarError::BadTarget)
    );
    let one = candles(&rows()[..1]);
    let big = vec![one[0].clone(); MAX_INPUT_BARS + 1];
    assert_eq!(
        aggregate(&cal, SEG, KIND, Interval::Min5, &big).err(),
        Some(BarError::TooManyBars(MAX_INPUT_BARS + 1))
    );
}

fn real_cal() -> MarketCalendar {
    let cfg: CalendarConfig = load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/config_real_hours"
    )))
    .unwrap();
    MarketCalendar::from_config(&cfg)
}

/// One minute bar per minute of 09:15-15:30 on 2026-03-02 (375 minutes). Minute k (0-based)
/// closes at 09:16 + k: open 100+k, high 200+k, low 50+k, close 100+k+0.5, volume k+1.
fn real_session() -> Vec<Candle> {
    let first = Kolkata.with_ymd_and_hms(2026, 3, 2, 9, 16, 0).unwrap();
    (0..375i64)
        .map(|k| {
            let o = Decimal::from(100 + k);
            Candle::new(
                first + Duration::minutes(k),
                Interval::Min1,
                [
                    o,
                    Decimal::from(200 + k),
                    Decimal::from(50 + k),
                    o + d("0.5"),
                ],
                u64::try_from(k + 1).unwrap(),
                false,
            )
            .unwrap()
        })
        .collect()
}

fn real_run(target: Interval, input: &[Candle]) -> (Vec<Candle>, BarStats) {
    aggregate(&real_cal(), SEG, KIND, target, input).unwrap()
}

#[test]
fn short_final_bar_min10_on_real_hours() {
    let (bars, s) = real_run(Interval::Min10, &real_session());
    // 37 full bars plus a 5-minute bar 15:25-15:30.
    assert_eq!(bars.len(), 38);
    let last = bars.last().unwrap();
    assert_eq!(
        last.close_time(),
        Kolkata.with_ymd_and_hms(2026, 3, 2, 15, 30, 0).unwrap()
    );
    // Minutes k = 370..=374: open 470, high 574 (200+374), low 420 (50+370), close 474.5,
    // volume 371+372+373+374+375 = 1865.
    assert_eq!(
        [last.open(), last.high(), last.low(), last.close()],
        [d("470"), d("574"), d("420"), d("474.5")]
    );
    assert_eq!(last.volume(), 1865);
    assert_eq!(s.incomplete_buckets, 0);
    assert_eq!(s.gap_minutes, 0);
    assert_eq!(s.accepted, 375);
}

#[test]
fn short_final_bar_min60_on_real_hours() {
    let (bars, s) = real_run(Interval::Min60, &real_session());
    // 6 full hours 09:15-15:15 plus 15:15-15:30.
    assert_eq!(bars.len(), 7);
    let last = bars.last().unwrap();
    assert_eq!(
        last.close_time(),
        Kolkata.with_ymd_and_hms(2026, 3, 2, 15, 30, 0).unwrap()
    );
    // Minutes k = 360..=374: open 460, high 574, low 410, close 474.5, volume sum(361..=375).
    assert_eq!(
        [last.open(), last.high(), last.low(), last.close()],
        [d("460"), d("574"), d("410"), d("474.5")]
    );
    assert_eq!(last.volume(), (361..=375).sum::<u64>());
    assert_eq!(s.incomplete_buckets, 0);
    assert_eq!(s.emitted, 7);
}

#[test]
fn day_bar_on_real_hours_ends_at_session_end() {
    let (bars, s) = real_run(Interval::Day, &real_session());
    assert_eq!(bars.len(), 1);
    let b = &bars[0];
    assert_eq!(
        b.close_time(),
        Kolkata.with_ymd_and_hms(2026, 3, 2, 15, 30, 0).unwrap()
    );
    assert_eq!(
        [b.open(), b.high(), b.low(), b.close()],
        [d("100"), d("574"), d("50"), d("474.5")]
    );
    assert_eq!(b.volume(), (1..=375).sum::<u64>());
    assert_eq!(s.incomplete_buckets, 0);
}

#[test]
fn year_outside_calendar_counts_calendar_errors() {
    let r = rows();
    let mut late = r[0];
    late.0 += Duration::days(365 * 2); // 2028: not in covered_years
    let (bars, s) = run(Interval::Min5, &[mk(&late, Interval::Min1)]);
    assert_eq!(bars.len(), 0);
    assert_eq!(s.calendar_errors, 1);
    assert_eq!(s.out_of_session, 0);
}

#[test]
fn special_session_minutes_are_not_aggregated() {
    let r = rows();
    let mut muhurat = r[0];
    muhurat.0 = Kolkata.with_ymd_and_hms(2026, 11, 7, 18, 1, 0).unwrap();
    let mut input = candles(&r);
    input.push(mk(&muhurat, Interval::Min1));
    let (bars, s) = run(Interval::Min5, &input);
    assert_eq!(bars.len(), 48);
    assert_eq!(s.out_of_session, 1);
    assert_eq!(s.accepted, 240);
}

#[test]
fn mixed_adjusted_bucket_is_dropped() {
    let r = rows();
    let mut input = candles(&r);
    input[2] = Candle::new(r[2].0, Interval::Min1, r[2].1, r[2].2, true).unwrap();
    let (bars, s) = run(Interval::Min5, &input);
    assert_eq!(bars.len(), 47);
    assert_eq!(s.mixed_adjusted_buckets, 1);
    // A bucket wholly adjusted keeps its flag.
    let all: Vec<Candle> = r[..5]
        .iter()
        .map(|x| Candle::new(x.0, Interval::Min1, x.1, x.2, true).unwrap())
        .collect();
    let (bars, _) = run(Interval::Min5, &all);
    assert!(bars[0].adjusted());
}

#[test]
fn min_timestamp_does_not_panic() {
    let r = rows();
    let c = Candle::new(
        DateTime::<chrono::Utc>::MIN_UTC.with_timezone(&Kolkata),
        Interval::Min1,
        r[0].1,
        r[0].2,
        false,
    );
    if let Ok(c) = c {
        let (bars, s) = run(Interval::Min5, &[c]);
        assert_eq!(bars.len(), 0);
        assert_eq!(s.accepted, 0);
    }
}

#[test]
fn rejected_minutes_cannot_force_repeated_probes() {
    // Alternating out-of-order dates: the second and later are out of order, not re-probed.
    let r = rows();
    let day2 = (r[0].0 + Duration::days(1), r[0].1, r[0].2);
    let mut input = Vec::new();
    for _ in 0..2000 {
        input.push(mk(&day2, Interval::Min1));
        input.push(mk(&r[0], Interval::Min1));
    }
    let (_, s) = run(Interval::Min5, &input);
    assert_eq!(s.accepted, 1);
    assert_eq!(s.out_of_order, 3999);
}

#[test]
fn prefix_of_input_gives_prefix_of_output() {
    let input = candles(&rows());
    let (full, _) = run(Interval::Min15, &input);
    for cut in (0..=input.len()).step_by(7) {
        let (part, _) = run(Interval::Min15, &input[..cut]);
        assert!(part.len() <= full.len());
        assert_eq!(part[..], full[..part.len()]);
    }
}
