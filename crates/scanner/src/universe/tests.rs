use super::*;
use bazaar_core::candle::Interval;
use chrono::TimeZone;
use chrono_tz::Asia::Kolkata;

fn at(d: u32, h: u32) -> DateTime<Tz> {
    Kolkata.with_ymd_and_hms(2026, 3, d, h, 0, 0).unwrap()
}

fn list(as_of: Option<DateTime<Tz>>) -> BanListStatus {
    BanListStatus::Available {
        symbols: vec![" abc ".into(), "Xyz".into()],
        as_of,
    }
}

#[test]
fn unavailable_ban_list_fails_closed() {
    let s = BanListStatus::Unavailable {
        reason: "down".into(),
    };
    assert_eq!(
        banned_set(&s, at(5, 9), 24),
        Err(DropReason::BanListUnavailable("down".into()))
    );
}

#[test]
fn undated_ban_list_fails_closed() {
    assert_eq!(
        banned_set(&list(None), at(5, 9), 24),
        Err(DropReason::BanListUndated)
    );
}

#[test]
fn ban_list_25_hours_old_is_stale() {
    // 2026-03-04 08:00 -> 2026-03-05 09:00 is 25 h.
    assert_eq!(
        banned_set(&list(Some(at(4, 8))), at(5, 9), 24),
        Err(DropReason::BanListStale { age_hours: 25 })
    );
}

#[test]
fn ban_list_24_hours_old_is_fresh_and_names_are_case_normalised() {
    let set = banned_set(&list(Some(at(4, 9))), at(5, 9), 24).unwrap();
    assert!(set.contains("ABC") && set.contains("XYZ"));
    assert_eq!(set.len(), 2);
}

#[test]
fn ban_list_dated_in_the_future_fails_closed() {
    assert!(matches!(
        banned_set(&list(Some(at(6, 9))), at(5, 9), 24),
        Err(DropReason::BanListStale { .. })
    ));
}

fn bar(d: u32, close: i64, vol: u64) -> Candle {
    let p = Decimal::from(close);
    Candle::new(at(d, 15), Interval::Day, [p, p, p, p], vol, true).unwrap()
}

#[test]
fn median_traded_value_odd_even_and_window() {
    assert_eq!(median_traded_value(&[], 20), None);
    let odd = [bar(2, 10, 1), bar(3, 10, 5), bar(4, 10, 3)];
    assert_eq!(median_traded_value(&odd, 3), Some(Decimal::from(30)));
    let even = [bar(2, 10, 1), bar(3, 10, 3)];
    assert_eq!(median_traded_value(&even, 2), Some(Decimal::from(20)));
    // Only the last 2 bars count: 50 and 30 -> 40.
    let win = [bar(2, 10, 900), bar(3, 10, 5), bar(4, 10, 3)];
    assert_eq!(median_traded_value(&win, 2), Some(Decimal::from(40)));
}

#[test]
fn median_needs_a_full_window_excludes_older_spikes_and_survives_overflow() {
    // 25 bars: an old spike in the first 5 is outside the 20-bar window.
    let mut bars: Vec<Candle> = (0..25).map(|i| bar(1 + (i % 28), 10, 1)).collect();
    bars[0] = bar(1, 10, 1_000_000);
    assert_eq!(median_traded_value(&bars, 20), Some(Decimal::from(10)));
    assert_eq!(median_traded_value(&bars[..19], 20), None);
    let big = Decimal::MAX / Decimal::TWO;
    let c = Candle::new(at(2, 15), Interval::Day, [big, big, big, big], 10, true).unwrap();
    assert_eq!(median_traded_value(&[c], 1), None);
}
