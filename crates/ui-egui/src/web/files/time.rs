//! Dates as the file list shows them ("Today 09:10", "Yesterday 16:05", "11 Nov 2026") and version
//! stamps, without a date library: the server sends RFC 3339 in UTC, and the browser says its
//! offset.

use crate::backend::WallClock;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(year, month 1–12, day)` of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

/// `2026-11-13T22:17:27.932757Z` (or with `+01:00`) → seconds since the epoch.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r).filter(|t| t.bytes().all(|c| c.is_ascii_digit()))?.parse().ok() };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) => {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            sign * (o.get(1..3)?.parse::<i64>().ok()? * 3600 + o.get(4..6)?.parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// When something changed, for a list: "Today 09:10", "Yesterday 16:05", "11 Nov 2026"; with no
/// clock, "13 Nov 2026 22:17" in UTC.
pub fn when(rfc3339: &str, clock: Option<WallClock>) -> String {
    local(rfc3339, clock, false)
}

/// As [`when`], but older dates keep their time too ("11 Nov 2026 09:10"), for telling apart
/// things that happened on the same day (the trash's DELETED column).
pub fn when_exact(rfc3339: &str, clock: Option<WallClock>) -> String {
    local(rfc3339, clock, true)
}

fn local(rfc3339: &str, clock: Option<WallClock>, exact: bool) -> String {
    let Some(t) = parse_rfc3339(rfc3339) else { return String::new() };
    let offset = clock.map_or(0, |c| i64::from(c.utc_offset_minutes) * 60);
    let local = t + offset;
    let day = local.div_euclid(86_400);
    let (y, m, d) = civil_from_days(day);
    let secs = local.rem_euclid(86_400);
    let hm = format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60);
    let date = format!("{d} {} {y}", MONTHS[(m - 1) as usize]);
    match clock {
        Some(c) => {
            let today = (c.unix as i64 + offset).div_euclid(86_400);
            match today - day {
                0 => format!("Today {hm}"),
                1 => format!("Yesterday {hm}"),
                _ if exact => format!("{date} {hm}"),
                _ => date,
            }
        }
        None => format!("{date} {hm}"),
    }
}

/// A version stamp for `unix` seconds (UTC): `20261113T151013.123Z`, as `versions::STAMP_FORMAT`.
pub fn stamp(unix: f64) -> String {
    let millis = (unix * 1000.0).round() as i64;
    let secs = millis.div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}.{:03}Z", s / 3600, s % 3600 / 60, s % 60, millis.rem_euclid(1000))
}

/// A version stamp as a time to show: `20261113T151013.123Z` → like [`when`].
#[allow(dead_code)] // For the actions that follow.
pub fn stamp_when(stamp: &str, clock: Option<WallClock>) -> Option<String> {
    let b = stamp.as_bytes();
    if b.len() < 16 || b[8] != b'T' {
        return None;
    }
    let iso = format!("{}-{}-{}T{}:{}:{}Z", &stamp[0..4], &stamp[4..6], &stamp[6..8], &stamp[9..11], &stamp[11..13], &stamp[13..15]);
    parse_rfc3339(&iso).map(|_| when(&iso, clock))
}

#[cfg(test)]
mod tests {
    use super::{parse_rfc3339, stamp, stamp_when, when, when_exact};
    use crate::backend::WallClock;

    #[test]
    fn reads_the_servers_times() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-13T22:17:27.932757Z"), Some(1_794_608_247));
        assert_eq!(parse_rfc3339("2026-11-13T23:17:27+01:00"), Some(1_794_608_247));
        assert_eq!(parse_rfc3339("yesterday"), None);
    }

    #[test]
    fn lists_say_today_and_yesterday() {
        // 2026-11-13 22:17:27 UTC; the viewer is at UTC+1, where it is already the 14th.
        let clock = Some(WallClock { unix: 1_794_608_247.0, utc_offset_minutes: 60 });
        assert_eq!(when("2026-11-13T22:17:27Z", clock), "Today 23:17");
        assert_eq!(when("2026-11-12T23:30:00Z", clock), "Today 00:30");
        assert_eq!(when("2026-11-12T22:00:00Z", clock), "Yesterday 23:00");
        assert_eq!(when("2026-11-02T10:00:00Z", clock), "2 Nov 2026");
        assert_eq!(when("2026-11-02T10:00:00Z", None), "2 Nov 2026 10:00");
        assert_eq!(when("", clock), "");
        assert_eq!(when_exact("2026-11-02T10:00:00Z", clock), "2 Nov 2026 11:00", "local time, with the time");
        assert_eq!(when_exact("2026-11-13T22:17:27Z", clock), "Today 23:17");
    }

    #[test]
    fn stamps_sort_and_read_back() {
        assert_eq!(stamp(1_794_608_247.123), "20261113T221727.123Z");
        assert!(stamp(1_794_608_247.0) < stamp(1_794_608_248.0));
        assert_eq!(stamp_when("20261113T221727.123Z", None).as_deref(), Some("13 Nov 2026 22:17"));
        assert_eq!(stamp_when("notes.md", None), None);
    }
}
