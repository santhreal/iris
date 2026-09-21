//! Local wall-clock formatting without a date crate.
//!
//! iris needs two things from a clock: a millisecond timestamp for
//! library ordering and a `YYYY-MM-DD`/`HH-MM-SS` stamp for file
//! names. chrono carried iana-time-zone, num-traits, libm and autocfg
//! for that; libc's localtime_r and SystemTime cover both.

/// Local time as (year, month, day, hour, minute, second), via the
/// C library's timezone database. Falls back to UTC fields when
/// localtime_r fails (no /etc/localtime, embedded target).
pub fn local_now() -> (i32, u32, u32, u32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            // UTC fallback: civil-from-days over the raw epoch.
            return utc_fields(secs);
        }
        (
            tm.tm_year + 1900,
            (tm.tm_mon + 1) as u32,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
            tm.tm_sec as u32,
        )
    }
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

/// Howard Hinnant's civil-from-days: epoch seconds to UTC calendar
/// fields, used only when localtime_r cannot resolve a zone.
fn utc_fields(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (y + (m <= 2) as i64) as i32;
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
        assert!(y >= 2020 && y < 2200, "year {y}");
        assert!((1..=12).contains(&mo));
        assert!((1..=31).contains(&d));
        assert!(h < 24 && mi < 60 && s < 61);
    }
}
