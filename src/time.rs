//! Local wall-clock formatting without a date crate.
//!
//! iris needs two things from a clock: a millisecond timestamp for
//! library ordering and a `YYYY-MM-DD`/`HH-MM-SS` stamp for file
//! names. chrono carried iana-time-zone, num-traits, libm and autocfg
//! for that; libc's localtime_r and SystemTime cover both.

/// Local time as (year, month, day, hour, minute, second), via the
/// C library's timezone database. Falls back to UTC fields when the
/// platform localtime call fails (no zone database, embedded target).
pub fn local_now() -> (i32, u32, u32, u32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    local_fields(secs)
}

/// Epoch `secs` as local (year, month, day, hour, minute, second), with
/// the UTC fallback of [`local_now`].
pub fn local_fields(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let Some(tm) = crate::sys::time::localtime(secs) else {
        // UTC fallback: civil-from-days over the raw epoch.
        return utc_fields(secs);
    };
    (
        tm.tm_year + 1900,
        (tm.tm_mon + 1) as u32,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
        tm.tm_sec as u32,
    )
}

/// Milliseconds since the Unix epoch, for ordering and dedup.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD` for the {date} template placeholder.
pub fn date_stamp() -> String {
    let (y, mo, d, ..) = local_now();
    format!("{y:04}-{mo:02}-{d:02}")
}

/// `HH-MM-SS` for the {time} template placeholder.
pub fn time_stamp() -> String {
    let (.., h, mi, s) = local_now();
    format!("{h:02}-{mi:02}-{s:02}")
}

/// `HH:MM:SS.mmm` for log lines.
pub fn log_stamp() -> String {
    let millis = now_millis();
    let (.., h, mi, s) = local_now();
    format!("{h:02}:{mi:02}:{s:02}.{:03}", millis.rem_euclid(1000))
}

/// Howard Hinnant's civil-from-days: days since 1970-01-01 to the
/// proleptic Gregorian (year, month, day).
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m, d)
}

/// The inverse of [`civil_from_days`]: the proleptic Gregorian date
/// `y`-`m`-`d` as days since 1970-01-01.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Epoch seconds to UTC calendar fields, used only when localtime_r
/// cannot resolve a zone.
fn utc_fields(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let (year, m, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    (
        year,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

// WHY: the class closed here is "the stamp drifts from the platform
// clock": a hand-rolled formatter that mis-reads localtime fields or
// botches the UTC fallback writes wrong capture names. The fallback
// is pinned against known epochs; the localtime path is verified by
// shape (fields in range), since the zone is the machine's own.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_fields_matches_known_epochs() {
        // 1970-01-01 00:00:00 UTC.
        assert_eq!(utc_fields(0), (1970, 1, 1, 0, 0, 0));
        // 2000-02-29 12:34:56 UTC (leap day).
        assert_eq!(utc_fields(951_827_696), (2000, 2, 29, 12, 34, 56));
        // 2038-01-19 03:14:07 UTC (i32 edge).
        assert_eq!(utc_fields(2_147_483_647), (2038, 1, 19, 3, 14, 7));
        // Pre-epoch: 1969-12-31 23:59:59 UTC.
        assert_eq!(utc_fields(-1), (1969, 12, 31, 23, 59, 59));
    }

    // WHY: the library groups captures by local day through these two
    // conversions. Closed here: a day that maps to a date that does not
    // map back (a leap day, a century, a year boundary, a pre-epoch day),
    // which would split one day's captures or merge two days'. Not
    // covered: the zone lookup, which is the platform's.
    #[test]
    fn civil_days_round_trip_across_every_day_of_four_centuries() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        let (lo, hi) = (days_from_civil(1800, 1, 1), days_from_civil(2200, 1, 1));
        let mut prev = civil_from_days(lo - 1);
        for day in lo..hi {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day, "{y}-{m}-{d}");
            // Consecutive days are consecutive dates.
            let next_of_prev = if d == 1 {
                (m == 1 && prev.0 + 1 == y && prev.1 == 12) || (prev.0 == y && prev.1 + 1 == m)
            } else {
                prev.0 == y && prev.1 == m && prev.2 + 1 == d
            };
            assert!(next_of_prev, "{prev:?} then {y}-{m}-{d}");
            prev = (y, m, d);
        }
        // 1900 is not a leap year, 2000 is.
        assert_eq!(
            civil_from_days(days_from_civil(1900, 2, 28) + 1),
            (1900, 3, 1)
        );
        assert_eq!(
            civil_from_days(days_from_civil(2000, 2, 28) + 1),
            (2000, 2, 29)
        );
    }

    #[test]
    fn stamps_have_fixed_shape() {
        let d = date_stamp();
        assert_eq!(d.len(), 10, "date {d:?}");
        assert_eq!(&d[4..5], "-");
        let t = time_stamp();
        assert_eq!(t.len(), 8, "time {t:?}");
        let l = log_stamp();
        assert_eq!(l.len(), 12, "log {l:?}");
    }

    #[test]
    fn local_now_fields_in_range() {
        let (y, mo, d, h, mi, s) = local_now();
        assert!((2020..2200).contains(&y), "year {y}");
        assert!((1..=12).contains(&mo));
        assert!((1..=31).contains(&d));
        assert!(h < 24 && mi < 60 && s < 61);
    }
}
