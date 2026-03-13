//! Recency decay for memory scoring.
//!
//! Computes how many days have elapsed since an episode's timestamp and
//! applies a hyperbolic decay to scoring. Decision and Preference
//! neighborhoods are exempt from recency decay in the caller.

use crate::system::DAESystem;

/// Recency decay coefficient for non-decision memories.
/// score *= 1.0 / (1.0 + `days_old` * `RECENCY_DECAY_RATE`)
pub(crate) const RECENCY_DECAY_RATE: f64 = 0.01;

/// Compute days since an episode's timestamp (empty or unparseable returns 0.0).
pub(crate) fn days_since_episode(system: &DAESystem, episode_idx: usize) -> f64 {
    let timestamp = if episode_idx == usize::MAX {
        &system.conscious_episode.timestamp
    } else {
        &system.episodes[episode_idx].timestamp
    };
    if timestamp.is_empty() {
        return 0.0;
    }
    // Parse ISO-8601 timestamps like "2026-02-19T12:00:00Z" or "2026-02-19"
    // Fall back to 0.0 if unparseable (no external chrono dep - simple parse).
    parse_days_ago(timestamp, crate::time::now_unix_secs())
}

/// Parse an ISO-8601 date prefix (YYYY-MM-DD) and return the number of
/// whole days between that date and today. Returns 0.0 for unparseable input.
///
/// `now_secs` is the current time as Unix seconds, passed explicitly for
/// testability.
pub(crate) fn parse_days_ago(timestamp: &str, now_secs: u64) -> f64 {
    // Extract YYYY-MM-DD from start of timestamp
    if timestamp.len() < 10 {
        return 0.0;
    }
    let parts: Vec<&str> = timestamp[..10].split('-').collect();
    if parts.len() != 3 {
        return 0.0;
    }
    let Ok(y) = parts[0].parse::<i64>() else {
        return 0.0;
    };
    let Ok(m) = parts[1].parse::<i64>() else {
        return 0.0;
    };
    let Ok(d) = parts[2].parse::<i64>() else {
        return 0.0;
    };

    // Simple Julian Day Number for comparison (good enough for decay)
    let jdn = |year: i64, month: i64, day: i64| -> i64 {
        let a = (14 - month) / 12;
        let y = year + 4800 - a;
        let m = month + 12 * a - 3;
        day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045
    };

    let now_days = (now_secs / 86400) as i64;
    // Unix epoch is JDN 2440588
    let now_jdn = now_days + 2_440_588;
    let ep_jdn = jdn(y, m, d);
    let diff = now_jdn - ep_jdn;
    diff.max(0) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-03-13T00:00:00Z as Unix seconds (midnight UTC)
    const NOW_2026_03_13: u64 = 1_773_360_000;

    #[test]
    fn same_day_returns_zero() {
        let days = parse_days_ago("2026-03-13T10:00:00Z", NOW_2026_03_13);
        assert!((days - 0.0).abs() < f64::EPSILON, "expected 0, got {days}");
    }

    #[test]
    fn one_day_ago() {
        let days = parse_days_ago("2026-03-12T10:00:00Z", NOW_2026_03_13);
        assert!((days - 1.0).abs() < f64::EPSILON, "expected 1, got {days}");
    }

    #[test]
    fn unix_epoch_large_positive() {
        let days = parse_days_ago("1970-01-01T00:00:00Z", NOW_2026_03_13);
        // 2026-03-13 is ~20525 days after 1970-01-01
        assert!(days > 20_000.0, "expected >20000, got {days}");
        assert!(days < 21_000.0, "expected <21000, got {days}");
    }

    #[test]
    fn year_boundary() {
        // 2026-01-01T00:00:00Z as Unix seconds
        let now_jan1 = 1_767_225_600;
        let days = parse_days_ago("2025-12-31T23:59:59Z", now_jan1);
        assert!((days - 1.0).abs() < f64::EPSILON, "expected 1, got {days}");
    }

    #[test]
    fn malformed_input_returns_zero() {
        assert!((parse_days_ago("not-a-date", NOW_2026_03_13) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn too_short_returns_zero() {
        assert!((parse_days_ago("2026", NOW_2026_03_13) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_string_returns_zero() {
        assert!((parse_days_ago("", NOW_2026_03_13) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn date_only_no_time_suffix() {
        let days = parse_days_ago("2026-03-12", NOW_2026_03_13);
        assert!((days - 1.0).abs() < f64::EPSILON, "expected 1, got {days}");
    }

    #[test]
    fn future_date_clamped_to_zero() {
        let days = parse_days_ago("2027-01-01T00:00:00Z", NOW_2026_03_13);
        assert!(
            (days - 0.0).abs() < f64::EPSILON,
            "future date should clamp to 0, got {days}"
        );
    }

    #[test]
    fn seven_days_ago() {
        let days = parse_days_ago("2026-03-06T10:00:00Z", NOW_2026_03_13);
        assert!((days - 7.0).abs() < f64::EPSILON, "expected 7, got {days}");
    }

    #[test]
    fn fractional_day_now_midday() {
        // now_secs at 2026-03-13T12:00:00Z (noon) = midnight + 43200
        let now_midday = NOW_2026_03_13 + 43_200;
        // Episode from yesterday: still 1 full day because parse_days_ago
        // uses integer division (now_secs / 86400) for the current day.
        let days = parse_days_ago("2026-03-12T18:00:00Z", now_midday);
        assert!((days - 1.0).abs() < f64::EPSILON, "expected 1, got {days}");
        // Same day: 0 even though now is at noon
        let days = parse_days_ago("2026-03-13T00:00:00Z", now_midday);
        assert!((days - 0.0).abs() < f64::EPSILON, "expected 0, got {days}");
    }

    #[test]
    fn leap_year_feb29() {
        // 2024 was a leap year; 2024-02-29 is valid
        // 2024-03-01T00:00:00Z = 1709251200
        let now_mar1_2024 = 1_709_251_200;
        let days = parse_days_ago("2024-02-29T00:00:00Z", now_mar1_2024);
        assert!(
            (days - 1.0).abs() < f64::EPSILON,
            "expected 1 day, got {days}"
        );
    }
}
