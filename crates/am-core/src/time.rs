//! Lightweight UTC date/time utilities (no chrono dependency).
//!
//! Uses Howard Hinnant's civil_from_days algorithm for Unix-to-date conversion.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current UTC time as Unix seconds.
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Current UTC timestamp in ISO-8601 format.
pub fn now_iso8601() -> String {
    unix_to_iso8601(now_unix_secs())
}

/// Convert Unix seconds to ISO-8601 UTC string.
pub fn unix_to_iso8601(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Howard Hinnant's civil_from_days: Unix epoch days → (year, month, day).
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_epoch() {
        assert_eq!(unix_to_iso8601(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn test_known_date() {
        // 2026-02-21T00:00:00Z = 1771632000
        assert_eq!(unix_to_iso8601(1771632000), "2026-02-21T00:00:00Z");
    }

    #[test]
    fn test_now_is_recent() {
        let ts = now_iso8601();
        assert!(ts.starts_with("202"), "timestamp should be in 2020s: {ts}");
    }
}
