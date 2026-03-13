//! Recency decay for memory scoring.
//!
//! Computes how many days have elapsed since an episode's timestamp and
//! applies a hyperbolic decay to scoring. Decision and Preference
//! neighborhoods are exempt from recency decay in the caller.

use crate::system::DAESystem;

/// Recency decay coefficient for non-decision memories.
/// score *= 1.0 / (1.0 + days_old * RECENCY_DECAY_RATE)
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
    parse_days_ago(timestamp)
}

/// Parse an ISO-8601 date prefix (YYYY-MM-DD) and return the number of
/// whole days between that date and today. Returns 0.0 for unparseable input.
pub(crate) fn parse_days_ago(timestamp: &str) -> f64 {
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

    // Simple Julian day number for comparison (good enough for decay)
    let jdn = |year: i64, month: i64, day: i64| -> i64 {
        let a = (14 - month) / 12;
        let y = year + 4800 - a;
        let m = month + 12 * a - 3;
        day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045
    };

    let now_days = (crate::time::now_unix_secs() / 86400) as i64;
    // Unix epoch is JDN 2440588
    let now_jdn = now_days + 2_440_588;
    let ep_jdn = jdn(y, m, d);
    let diff = now_jdn - ep_jdn;
    diff.max(0) as f64
}
