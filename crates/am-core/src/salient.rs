//! Salient content extraction and neighborhood type detection.
//!
//! Extracts `<salient>...</salient>` tagged content from text and adds it
//! to the conscious episode. Detects `DECISION:` and `PREFERENCE:` prefixes
//! to set neighborhood types automatically.

use std::sync::LazyLock;

use rand::Rng;
use regex::Regex;
use uuid::Uuid;

use crate::neighborhood::NeighborhoodType;
use crate::system::DAESystem;

static SALIENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<salient>(.*?)</salient>").unwrap());

/// Detect neighborhood type from text prefix (DECISION: / PREFERENCE:).
/// Returns the detected type and the text with the prefix stripped.
pub fn detect_neighborhood_type(text: &str) -> (NeighborhoodType, &str) {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("DECISION:") {
        (NeighborhoodType::Decision, rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix("PREFERENCE:") {
        (NeighborhoodType::Preference, rest.trim())
    } else {
        (NeighborhoodType::Insight, trimmed)
    }
}

/// Extract salient-tagged content and add to conscious episode.
/// Detects DECISION: and PREFERENCE: prefixes to set neighborhood type.
pub fn extract_salient(system: &mut DAESystem, text: &str, rng: &mut impl Rng) -> u32 {
    let mut count = 0u32;
    for cap in SALIENT_RE.captures_iter(text) {
        if let Some(content) = cap.get(1) {
            let (nbhd_type, clean_text) = detect_neighborhood_type(content.as_str());
            system.add_to_conscious_typed(clean_text, nbhd_type, rng);
            count += 1;
        }
    }
    count
}

/// Mark text as salient with automatic type detection from prefix.
/// Used by `am_salient` when no `<salient>` tags are present.
pub fn mark_salient_typed(system: &mut DAESystem, text: &str, rng: &mut impl Rng) -> Uuid {
    let (nbhd_type, clean_text) = detect_neighborhood_type(text);
    system.add_to_conscious_typed(clean_text, nbhd_type, rng)
}
