use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use rand::Rng;
use regex::Regex;
use uuid::Uuid;

use crate::query::{InterferenceResult, QueryResult};
use crate::surface::SurfaceResult;
use crate::system::{DAESystem, OccurrenceRef};
use crate::tokenizer::token_count;

/// Category of recalled content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallCategory {
    Conscious,
    Subconscious,
    Novel,
}

/// Metrics about the composed context.
pub struct ContextMetrics {
    pub conscious: u32,
    pub subconscious: u32,
    pub novel: u32,
}

/// Neighborhood IDs categorized by recall type — for feedback tracking.
pub struct CategorizedIds {
    pub conscious: Vec<Uuid>,
    pub subconscious: Vec<Uuid>,
    pub novel: Vec<Uuid>,
}

/// Result of context composition.
pub struct ContextResult {
    pub context: String,
    pub metrics: ContextMetrics,
    /// Neighborhood IDs included in this result (for session dedup tracking).
    pub included_ids: Vec<Uuid>,
    /// Neighborhood IDs categorized by recall type (for am_feedback).
    pub recalled_ids: CategorizedIds,
}

/// Configuration for budget-constrained context composition.
pub struct BudgetConfig {
    /// Maximum token budget for the composed context.
    pub max_tokens: usize,
    /// Minimum conscious recall entries to include (if available).
    pub min_conscious: usize,
    /// Minimum subconscious recall entries to include (if available).
    pub min_subconscious: usize,
    /// Minimum novel connection entries to include (if available).
    pub min_novel: usize,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            min_conscious: 1,
            min_subconscious: 1,
            min_novel: 0,
        }
    }
}

/// A single fragment included in the budgeted result.
#[derive(Debug)]
pub struct IncludedFragment {
    pub neighborhood_id: Uuid,
    pub episode_name: String,
    pub category: RecallCategory,
    pub score: f64,
    pub tokens: usize,
    pub text: String,
    pub neighborhood_type: NeighborhoodType,
}

/// Result of budget-constrained context composition.
pub struct BudgetedContextResult {
    pub context: String,
    pub metrics: ContextMetrics,
    pub included: Vec<IncludedFragment>,
    pub excluded_count: usize,
    pub tokens_used: usize,
    pub tokens_budget: usize,
}

// -- Shared internals --

struct RankedCandidate {
    neighborhood_id: Uuid,
    episode_idx: usize,
    category: RecallCategory,
    score: f64,
    text: String,
    tokens: usize,
    neighborhood_type: NeighborhoodType,
}

/// Score and categorize all activated neighborhoods into ranked candidates.
/// Conscious neighborhoods scored by IDF-weighted activation.
/// Subconscious neighborhoods scored by IDF-weighted activation with project affinity.
/// Novel candidates: subconscious with activated_count <= 2, no words in common
/// with conscious, scored by max_word_weight * max_plasticity / activated_count.
fn rank_candidates(
    system: &mut DAESystem,
    query_result: &QueryResult,
    project_id: Option<&str>,
) -> Vec<RankedCandidate> {
    let conscious_words: HashSet<String> = query_result
        .activation
        .conscious
        .iter()
        .map(|r| system.get_occurrence(*r).word.to_lowercase())
        .collect();

    let con_scored = score_neighborhoods(system, &query_result.activation.conscious, true, None);
    let sub_scored = score_neighborhoods(
        system,
        &query_result.activation.subconscious,
        false,
        project_id,
    );

    let mut candidates = Vec::new();
    let mut selected_for_novel: HashSet<Uuid> = HashSet::new();

    // Conscious candidates
    for sn in con_scored.values() {
        let text = get_neighborhood_text(system, sn.neighborhood_id, sn.episode_idx);
        let tokens = token_count(&text);
        candidates.push(RankedCandidate {
            neighborhood_id: sn.neighborhood_id,
            episode_idx: sn.episode_idx,
            category: RecallCategory::Conscious,
            score: sn.score,
            text,
            tokens,
            neighborhood_type: sn.neighborhood_type,
        });
    }

    // Subconscious candidates
    for sn in sub_scored.values() {
        let text = get_neighborhood_text(system, sn.neighborhood_id, sn.episode_idx);
        let tokens = token_count(&text);
        candidates.push(RankedCandidate {
            neighborhood_id: sn.neighborhood_id,
            episode_idx: sn.episode_idx,
            category: RecallCategory::Subconscious,
            score: sn.score,
            text,
            tokens,
            neighborhood_type: sn.neighborhood_type,
        });

        // Check if this is also a novel candidate
        if sn.activated_count <= 2 && !sn.words.iter().any(|w| conscious_words.contains(w)) {
            selected_for_novel.insert(sn.neighborhood_id);
        }
    }

    // Add novel candidates (these are subconscious neighborhoods that qualify)
    for sn in sub_scored.values() {
        if !selected_for_novel.contains(&sn.neighborhood_id) {
            continue;
        }
        let novelty_score =
            sn.max_word_weight * sn.max_plasticity / sn.activated_count.max(1) as f64;
        let text = get_neighborhood_text(system, sn.neighborhood_id, sn.episode_idx);
        let tokens = token_count(&text);
        candidates.push(RankedCandidate {
            neighborhood_id: sn.neighborhood_id,
            episode_idx: sn.episode_idx,
            category: RecallCategory::Novel,
            score: novelty_score,
            text,
            tokens,
            neighborhood_type: sn.neighborhood_type,
        });
    }

    candidates
}

/// Format a single entry for the composed context string.
fn format_entry(
    category: RecallCategory,
    index: usize,
    ep_name: &str,
    text: &str,
    nbhd_type: NeighborhoodType,
) -> Vec<String> {
    let mut lines = Vec::new();
    match category {
        RecallCategory::Conscious => {
            lines.push("CONSCIOUS RECALL:".to_string());
            lines.push("[Source: Previously marked salient]".to_string());
        }
        RecallCategory::Subconscious => {
            lines.push(format!("SUBCONSCIOUS RECALL {}:", index));
            lines.push(format!("[Source: {}]", ep_name));
        }
        RecallCategory::Novel => {
            lines.push("NOVEL CONNECTION:".to_string());
            lines.push(format!("[Source: {}]", ep_name));
        }
    }
    // Decisions get [DECIDED] prefix so the AI knows not to re-litigate
    let formatted_text = if nbhd_type == NeighborhoodType::Decision {
        format!("[DECIDED] {}", text)
    } else if nbhd_type == NeighborhoodType::Preference {
        format!("[PREFERENCE] {}", text)
    } else {
        text.to_string()
    };
    lines.push(format!("\"{}\"", formatted_text));
    lines
}

const ENTRY_HEADER_OVERHEAD_TOKENS: usize = 20;

/// Compose human-readable context from surface and activation results.
///
/// `project_id` scopes subconscious recall to the current project (boosted)
/// while keeping conscious recall global. Pass `None` to disable scoping.
///
/// `session_recalled` tracks neighborhood IDs already returned this session.
/// Non-decision neighborhoods in this set are skipped (dedup). Decision
/// neighborhoods are always included but marked with `[DECIDED]` prefix.
///
/// `_surface` and `_interference` are part of the pipeline API and reserved
/// for future use (e.g. vivid filtering, interference-weighted scoring).
pub fn compose_context(
    system: &mut DAESystem,
    _surface: &SurfaceResult,
    query_result: &QueryResult,
    _interference: &[InterferenceResult],
    project_id: Option<&str>,
    session_recalled: Option<&HashSet<Uuid>>,
) -> ContextResult {
    let candidates = rank_candidates(system, query_result, project_id);

    let empty_set = HashSet::new();
    let recalled = session_recalled.unwrap_or(&empty_set);

    let mut selected_ids: HashSet<Uuid> = HashSet::new();
    let mut parts: Vec<String> = Vec::new();
    let mut metrics = ContextMetrics {
        conscious: 0,
        subconscious: 0,
        novel: 0,
    };
    let mut conscious_ids: Vec<Uuid> = Vec::new();
    let mut subconscious_ids: Vec<Uuid> = Vec::new();
    let mut novel_ids: Vec<Uuid> = Vec::new();

    // Helper: should this candidate be skipped due to session dedup?
    let should_skip = |c: &RankedCandidate| -> bool {
        if recalled.contains(&c.neighborhood_id) {
            // Decisions are never skipped — they always surface
            c.neighborhood_type != NeighborhoodType::Decision
                && c.neighborhood_type != NeighborhoodType::Preference
        } else {
            false
        }
    };

    // Conscious: top 1
    let mut con: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Conscious && !should_skip(c))
        .collect();
    con.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    if let Some(best) = con.first() {
        selected_ids.insert(best.neighborhood_id);
        conscious_ids.push(best.neighborhood_id);
        let entry = format_entry(
            RecallCategory::Conscious,
            0,
            "",
            &best.text,
            best.neighborhood_type,
        );
        parts.extend(entry);
        metrics.conscious = 1;
    }

    // Subconscious: top 2 (excluding already selected and deduped)
    let mut sub: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| {
            c.category == RecallCategory::Subconscious
                && !selected_ids.contains(&c.neighborhood_id)
                && !should_skip(c)
        })
        .collect();
    sub.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    for (i, entry) in sub.iter().take(2).enumerate() {
        selected_ids.insert(entry.neighborhood_id);
        subconscious_ids.push(entry.neighborhood_id);
        let ep_name = get_episode_name(system, entry.episode_idx);
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let lines = format_entry(
            RecallCategory::Subconscious,
            i + 1,
            &ep_name,
            &entry.text,
            entry.neighborhood_type,
        );
        parts.extend(lines);
        metrics.subconscious += 1;
    }

    // Novel: top 1 (excluding already selected and deduped)
    let mut novel: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| {
            c.category == RecallCategory::Novel
                && !selected_ids.contains(&c.neighborhood_id)
                && !should_skip(c)
        })
        .collect();
    novel.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    if let Some(best) = novel.first() {
        selected_ids.insert(best.neighborhood_id);
        novel_ids.push(best.neighborhood_id);
        let ep_name = get_episode_name(system, best.episode_idx);
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let lines = format_entry(
            RecallCategory::Novel,
            0,
            &ep_name,
            &best.text,
            best.neighborhood_type,
        );
        parts.extend(lines);
        metrics.novel = 1;
    }

    ContextResult {
        context: parts.join("\n"),
        metrics,
        recalled_ids: CategorizedIds {
            conscious: conscious_ids,
            subconscious: subconscious_ids,
            novel: novel_ids,
        },
        included_ids: selected_ids.into_iter().collect(),
    }
}

/// Budget-constrained context composition.
///
/// Fills guaranteed minimums first (highest-scored per category), then greedily
/// fills remaining budget by score across all categories.
///
/// `project_id` scopes subconscious recall to the current project (boosted)
/// while keeping conscious recall global. Pass `None` to disable scoping.
///
/// `session_recalled` tracks neighborhood IDs already returned this session.
/// Non-decision neighborhoods in this set are skipped (dedup). Decision
/// neighborhoods are always included but marked with `[DECIDED]` prefix.
///
/// `_surface` and `_interference` are part of the pipeline API and reserved
/// for future use (e.g. vivid filtering, interference-weighted scoring).
pub fn compose_context_budgeted(
    system: &mut DAESystem,
    _surface: &SurfaceResult,
    query_result: &QueryResult,
    _interference: &[InterferenceResult],
    budget: &BudgetConfig,
    project_id: Option<&str>,
    session_recalled: Option<&HashSet<Uuid>>,
) -> BudgetedContextResult {
    let candidates = rank_candidates(system, query_result, project_id);

    let empty_set = HashSet::new();
    let recalled = session_recalled.unwrap_or(&empty_set);

    // Filter out session-deduped non-decision candidates
    let candidates: Vec<RankedCandidate> = candidates
        .into_iter()
        .filter(|c| {
            if recalled.contains(&c.neighborhood_id) {
                // Decisions and preferences always pass through
                c.neighborhood_type == NeighborhoodType::Decision
                    || c.neighborhood_type == NeighborhoodType::Preference
            } else {
                true
            }
        })
        .collect();

    // Split candidates by category, sorted by score desc
    let mut conscious: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Conscious)
        .collect();
    conscious.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    let mut subconscious: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Subconscious)
        .collect();
    subconscious.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    let mut novel: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Novel)
        .collect();
    novel.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    // Deduplicate: a neighborhood can appear as both Subconscious and Novel.
    // Track which neighborhood_ids are included to avoid duplicates.
    let mut selected_ids: HashSet<Uuid> = HashSet::new();
    let mut included: Vec<IncludedFragment> = Vec::new();
    let mut tokens_used: usize = 0;
    // Count unique neighborhoods across all categories (a neighborhood may appear as both Subconscious and Novel)
    let unique_candidate_ids: HashSet<Uuid> =
        candidates.iter().map(|c| c.neighborhood_id).collect();
    let total_unique_candidates = unique_candidate_ids.len();

    let try_add = |candidate: &RankedCandidate,
                   selected_ids: &mut HashSet<Uuid>,
                   included: &mut Vec<IncludedFragment>,
                   tokens_used: &mut usize,
                   budget_limit: usize,
                   system: &DAESystem|
     -> bool {
        if selected_ids.contains(&candidate.neighborhood_id) {
            return false;
        }
        let cost = candidate.tokens + ENTRY_HEADER_OVERHEAD_TOKENS;
        if *tokens_used + cost > budget_limit {
            return false;
        }
        selected_ids.insert(candidate.neighborhood_id);
        *tokens_used += cost;
        let ep_name = get_episode_name(system, candidate.episode_idx);
        included.push(IncludedFragment {
            neighborhood_id: candidate.neighborhood_id,
            episode_name: ep_name,
            category: candidate.category,
            score: candidate.score,
            tokens: cost,
            text: candidate.text.clone(),
            neighborhood_type: candidate.neighborhood_type,
        });
        true
    };

    // Phase 1: Fill guaranteed minimums
    let mut con_filled = 0usize;
    for c in &conscious {
        if con_filled >= budget.min_conscious {
            break;
        }
        if try_add(
            c,
            &mut selected_ids,
            &mut included,
            &mut tokens_used,
            budget.max_tokens,
            system,
        ) {
            con_filled += 1;
        }
    }

    let mut sub_filled = 0usize;
    for c in &subconscious {
        if sub_filled >= budget.min_subconscious {
            break;
        }
        if try_add(
            c,
            &mut selected_ids,
            &mut included,
            &mut tokens_used,
            budget.max_tokens,
            system,
        ) {
            sub_filled += 1;
        }
    }

    let mut novel_filled = 0usize;
    for c in &novel {
        if novel_filled >= budget.min_novel {
            break;
        }
        if try_add(
            c,
            &mut selected_ids,
            &mut included,
            &mut tokens_used,
            budget.max_tokens,
            system,
        ) {
            novel_filled += 1;
        }
    }

    // Phase 2: Greedily fill remaining budget by score across all categories
    let mut remaining: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| !selected_ids.contains(&c.neighborhood_id))
        .collect();
    remaining.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    for c in &remaining {
        if tokens_used >= budget.max_tokens {
            break;
        }
        try_add(
            c,
            &mut selected_ids,
            &mut included,
            &mut tokens_used,
            budget.max_tokens,
            system,
        );
    }

    let excluded_count = total_unique_candidates.saturating_sub(included.len());

    // Format output, grouping by category in standard order
    let mut parts: Vec<String> = Vec::new();
    let mut metrics = ContextMetrics {
        conscious: 0,
        subconscious: 0,
        novel: 0,
    };

    // Conscious entries
    let con_entries: Vec<&IncludedFragment> = included
        .iter()
        .filter(|f| f.category == RecallCategory::Conscious)
        .collect();
    for entry in &con_entries {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let lines = format_entry(
            RecallCategory::Conscious,
            0,
            "",
            &entry.text,
            entry.neighborhood_type,
        );
        parts.extend(lines);
        metrics.conscious += 1;
    }

    // Subconscious entries
    let sub_entries: Vec<&IncludedFragment> = included
        .iter()
        .filter(|f| f.category == RecallCategory::Subconscious)
        .collect();
    for (i, entry) in sub_entries.iter().enumerate() {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let lines = format_entry(
            RecallCategory::Subconscious,
            i + 1,
            &entry.episode_name,
            &entry.text,
            entry.neighborhood_type,
        );
        parts.extend(lines);
        metrics.subconscious += 1;
    }

    // Novel entries
    let novel_entries: Vec<&IncludedFragment> = included
        .iter()
        .filter(|f| f.category == RecallCategory::Novel)
        .collect();
    for entry in &novel_entries {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let lines = format_entry(
            RecallCategory::Novel,
            0,
            &entry.episode_name,
            &entry.text,
            entry.neighborhood_type,
        );
        parts.extend(lines);
        metrics.novel += 1;
    }

    BudgetedContextResult {
        context: parts.join("\n"),
        metrics,
        included,
        excluded_count,
        tokens_used,
        tokens_budget: budget.max_tokens,
    }
}

// -- Scoring internals --

/// Flat score for decision neighborhoods — always high so they surface when relevant.
const DECISION_FLAT_SCORE: f64 = 100.0;

/// Recency decay coefficient for non-decision memories.
/// score *= 1.0 / (1.0 + days_old * RECENCY_DECAY_RATE)
const RECENCY_DECAY_RATE: f64 = 0.01;

struct ScoredNeighborhood {
    neighborhood_id: Uuid,
    episode_idx: usize, // usize::MAX for conscious
    score: f64,
    activated_count: usize,
    words: HashSet<String>,
    max_word_weight: f64,
    max_plasticity: f64,
    neighborhood_type: NeighborhoodType,
}

/// Compute days since an episode's timestamp (empty or unparseable → 0.0).
fn days_since_episode(system: &DAESystem, episode_idx: usize) -> f64 {
    let timestamp = if episode_idx == usize::MAX {
        &system.conscious_episode.timestamp
    } else {
        &system.episodes[episode_idx].timestamp
    };
    if timestamp.is_empty() {
        return 0.0;
    }
    // Parse ISO-8601 timestamps like "2026-02-19T12:00:00Z" or "2026-02-19"
    // Fall back to 0.0 if unparseable (no external chrono dep — simple parse).
    parse_days_ago(timestamp)
}

fn parse_days_ago(timestamp: &str) -> f64 {
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
    let now_jdn = now_days + 2440588;
    let ep_jdn = jdn(y, m, d);
    let diff = now_jdn - ep_jdn;
    diff.max(0) as f64
}

fn score_neighborhoods(
    system: &mut DAESystem,
    refs: &[OccurrenceRef],
    _is_conscious: bool,
    project_id: Option<&str>,
) -> HashMap<Uuid, ScoredNeighborhood> {
    let mut scored: HashMap<Uuid, ScoredNeighborhood> = HashMap::new();

    // Pre-collect data to avoid borrow conflicts
    let data: Vec<(Uuid, usize, String, u32, f64, NeighborhoodType)> = refs
        .iter()
        .map(|r| {
            let occ = system.get_occurrence(*r);
            let nbhd = system.get_neighborhood_for_occurrence(*r);
            (
                nbhd.id,
                if r.is_conscious() {
                    usize::MAX
                } else {
                    r.episode_idx
                },
                occ.word.to_lowercase(),
                occ.activation_count,
                occ.plasticity(),
                nbhd.neighborhood_type,
            )
        })
        .collect();

    // Pre-collect project affinity per episode_idx to avoid repeated lookups
    let affinity_cache: HashMap<usize, f64> = data
        .iter()
        .map(|(_, ep_idx, _, _, _, _)| *ep_idx)
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|ep_idx| {
            let affinity = if ep_idx == usize::MAX {
                // Conscious — always full weight
                1.0
            } else if let Some(pid) = project_id {
                let ep_pid = &system.episodes[ep_idx].project_id;
                if ep_pid == pid {
                    2.0 // Current project: strong boost
                } else if ep_pid.is_empty() {
                    1.0 // No project tag: neutral
                } else {
                    0.3 // Different project: suppressed
                }
            } else {
                1.0 // No project context: neutral
            };
            (ep_idx, affinity)
        })
        .collect();

    // Pre-collect recency decay per episode_idx
    let recency_cache: HashMap<usize, f64> = data
        .iter()
        .map(|(_, ep_idx, _, _, _, _)| *ep_idx)
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|ep_idx| {
            let days = days_since_episode(system, ep_idx);
            let decay = 1.0 / (1.0 + days * RECENCY_DECAY_RATE);
            (ep_idx, decay)
        })
        .collect();

    // For conscious neighborhoods, compute recency boost based on position.
    // Later neighborhoods (higher index) were added more recently.
    let conscious_count = if data
        .iter()
        .any(|(_, ep_idx, _, _, _, _)| *ep_idx == usize::MAX)
    {
        system.conscious_episode.neighborhoods.len() as f64
    } else {
        1.0
    };
    let conscious_recency: HashMap<Uuid, f64> = if conscious_count > 1.0 {
        system
            .conscious_episode
            .neighborhoods
            .iter()
            .enumerate()
            .map(|(i, nbhd)| {
                // Newest neighborhood (last) gets boost 2.0, oldest gets 1.0
                let recency = 1.0 + (i as f64 / conscious_count);
                (nbhd.id, recency)
            })
            .collect()
    } else {
        HashMap::new()
    };

    for (nbhd_id, ep_idx, word, activation_count, plasticity, nbhd_type) in &data {
        let weight = system.get_word_weight(word);
        let affinity = affinity_cache.get(ep_idx).copied().unwrap_or(1.0);

        let entry = scored
            .entry(*nbhd_id)
            .or_insert_with(|| ScoredNeighborhood {
                neighborhood_id: *nbhd_id,
                episode_idx: *ep_idx,
                score: 0.0,
                activated_count: 0,
                words: HashSet::new(),
                max_word_weight: 0.0,
                max_plasticity: 0.0,
                neighborhood_type: *nbhd_type,
            });

        entry.score += weight * *activation_count as f64 * affinity;
        entry.words.insert(word.clone());
        entry.activated_count += 1;
        if weight > entry.max_word_weight {
            entry.max_word_weight = weight;
        }
        if *plasticity > entry.max_plasticity {
            entry.max_plasticity = *plasticity;
        }
    }

    // Post-process: decisions get flat score, non-decisions get recency decay
    for sn in scored.values_mut() {
        match sn.neighborhood_type {
            NeighborhoodType::Decision => {
                sn.score = DECISION_FLAT_SCORE;
            }
            _ => {
                let decay = recency_cache.get(&sn.episode_idx).copied().unwrap_or(1.0);
                sn.score *= decay;
                // For conscious neighborhoods, apply recency boost (newer = higher score)
                if sn.episode_idx == usize::MAX {
                    let boost = conscious_recency
                        .get(&sn.neighborhood_id)
                        .copied()
                        .unwrap_or(1.0);
                    sn.score *= boost;
                }
            }
        }
    }

    scored
}

fn get_neighborhood_text(system: &DAESystem, neighborhood_id: Uuid, episode_idx: usize) -> String {
    let episode = if episode_idx == usize::MAX {
        &system.conscious_episode
    } else {
        &system.episodes[episode_idx]
    };

    for nbhd in &episode.neighborhoods {
        if nbhd.id == neighborhood_id {
            if !nbhd.source_text.is_empty() {
                return nbhd.source_text.clone();
            }
            return nbhd
                .occurrences
                .iter()
                .map(|o| o.word.as_str())
                .collect::<Vec<_>>()
                .join(" ");
        }
    }

    String::new()
}

fn get_episode_name(system: &DAESystem, episode_idx: usize) -> String {
    if episode_idx == usize::MAX {
        "Previously marked salient".to_string()
    } else {
        let ep = &system.episodes[episode_idx];
        if ep.name.is_empty() {
            "Memory".to_string()
        } else {
            ep.name.clone()
        }
    }
}

static SALIENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<salient>(.*?)</salient>").unwrap());

use crate::neighborhood::NeighborhoodType;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::Episode;
    use crate::neighborhood::Neighborhood;
    use crate::query::QueryEngine;
    use crate::surface::compute_surface;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn make_full_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Subconscious memories
        let mut ep = Episode::new("Science memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics", "particle", "wave"]),
            None,
            "quantum physics particle wave",
            &mut rng,
        ));
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["neural", "network", "deep", "learning"]),
            None,
            "neural network deep learning",
            &mut rng,
        ));
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["biology", "cell", "membrane", "protein"]),
            None,
            "biology cell membrane protein",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Conscious
        sys.add_to_conscious("quantum computing research", &mut rng);

        sys
    }

    #[test]
    fn test_compose_includes_recall_types() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics neural");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        assert!(ctx.context.contains("CONSCIOUS RECALL:"));
        assert!(ctx.context.contains("SUBCONSCIOUS RECALL"));
    }

    #[test]
    fn test_absent_types_omitted() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Only subconscious, no conscious overlap
        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta"]),
            None,
            "alpha beta",
            &mut rng,
        ));
        sys.add_episode(ep);

        let result = QueryEngine::process_query(&mut sys, "alpha");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        // No conscious recall since no conscious content matches
        assert!(!ctx.context.contains("CONSCIOUS RECALL:"));
    }

    #[test]
    fn test_metrics() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        assert!(ctx.metrics.conscious <= 1);
        assert!(ctx.metrics.subconscious <= 2);
        assert!(ctx.metrics.novel <= 1);
    }

    #[test]
    fn test_extract_salient_basic() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let count = extract_salient(
            &mut sys,
            "Normal text <salient>important insight</salient> more text",
            &mut rng,
        );
        assert_eq!(count, 1);
        assert_eq!(sys.conscious_episode.neighborhoods.len(), 1);
    }

    #[test]
    fn test_extract_salient_multiline() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let count = extract_salient(
            &mut sys,
            "Text <salient>line one\nline two</salient> rest",
            &mut rng,
        );
        assert_eq!(count, 1);
    }

    #[test]
    fn test_extract_salient_multiple() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let count = extract_salient(
            &mut sys,
            "<salient>first</salient> gap <salient>second</salient>",
            &mut rng,
        );
        assert_eq!(count, 2);
        assert_eq!(sys.conscious_episode.neighborhoods.len(), 2);
    }

    #[test]
    fn test_deterministic_scoring() {
        let mut sys1 = make_full_system();
        let result1 = QueryEngine::process_query(&mut sys1, "quantum");
        let surface1 = compute_surface(&sys1, &result1);
        let ctx1 = compose_context(
            &mut sys1,
            &surface1,
            &result1,
            &result1.interference,
            None,
            None,
        );

        let mut sys2 = make_full_system();
        let result2 = QueryEngine::process_query(&mut sys2, "quantum");
        let surface2 = compute_surface(&sys2, &result2);
        let ctx2 = compose_context(
            &mut sys2,
            &surface2,
            &result2,
            &result2.interference,
            None,
            None,
        );

        assert_eq!(ctx1.context, ctx2.context);
    }

    // -- Budgeted query tests --

    #[test]
    fn test_budgeted_respects_token_limit() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics neural");
        let surface = compute_surface(&sys, &result);

        let budget = BudgetConfig {
            max_tokens: 50,
            min_conscious: 0,
            min_subconscious: 0,
            min_novel: 0,
        };
        let ctx = compose_context_budgeted(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            &budget,
            None,
            None,
        );

        assert!(
            ctx.tokens_used <= ctx.tokens_budget,
            "tokens_used ({}) exceeded budget ({})",
            ctx.tokens_used,
            ctx.tokens_budget
        );
    }

    #[test]
    fn test_budgeted_includes_minimums() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics neural");
        let surface = compute_surface(&sys, &result);

        let budget = BudgetConfig {
            max_tokens: 4096,
            min_conscious: 1,
            min_subconscious: 1,
            min_novel: 0,
        };
        let ctx = compose_context_budgeted(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            &budget,
            None,
            None,
        );

        assert!(
            ctx.metrics.conscious >= 1,
            "expected at least 1 conscious, got {}",
            ctx.metrics.conscious
        );
        assert!(
            ctx.metrics.subconscious >= 1,
            "expected at least 1 subconscious, got {}",
            ctx.metrics.subconscious
        );
    }

    #[test]
    fn test_budgeted_tracks_excluded() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics neural");
        let surface = compute_surface(&sys, &result);

        // Very tight budget should exclude some
        let budget = BudgetConfig {
            max_tokens: 30,
            min_conscious: 0,
            min_subconscious: 0,
            min_novel: 0,
        };
        let ctx = compose_context_budgeted(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            &budget,
            None,
            None,
        );

        assert!(
            ctx.excluded_count > 0,
            "expected some excluded candidates with tight budget"
        );
    }

    #[test]
    fn test_budgeted_full_budget() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics neural");
        let surface = compute_surface(&sys, &result);

        let budget = BudgetConfig {
            max_tokens: 100000,
            min_conscious: 1,
            min_subconscious: 1,
            min_novel: 0,
        };
        let ctx = compose_context_budgeted(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            &budget,
            None,
            None,
        );

        // With huge budget, everything should be included
        assert_eq!(
            ctx.excluded_count, 0,
            "expected no exclusions with huge budget, got {}",
            ctx.excluded_count
        );
    }

    #[test]
    fn test_compose_context_unchanged() {
        // Regression: verify compose_context still produces the same output as before refactor
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics neural");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        // Must contain expected sections
        assert!(ctx.context.contains("CONSCIOUS RECALL:"));
        assert!(ctx.context.contains("[Source: Previously marked salient]"));
        assert!(ctx.context.contains("SUBCONSCIOUS RECALL 1:"));
        assert!(ctx.context.contains("[Source: Science memories]"));

        // Metrics within original limits
        assert_eq!(ctx.metrics.conscious, 1);
        assert!(ctx.metrics.subconscious >= 1 && ctx.metrics.subconscious <= 2);
        assert!(ctx.metrics.novel <= 1);

        // Deterministic: run again on fresh system, same output
        let mut sys2 = make_full_system();
        let result2 = QueryEngine::process_query(&mut sys2, "quantum physics neural");
        let surface2 = compute_surface(&sys2, &result2);
        let ctx2 = compose_context(
            &mut sys2,
            &surface2,
            &result2,
            &result2.interference,
            None,
            None,
        );
        assert_eq!(ctx.context, ctx2.context);
    }

    // =====================================================================
    // Decision-aware tests
    // =====================================================================

    #[test]
    fn test_detect_neighborhood_type_decision() {
        let (typ, text) = detect_neighborhood_type("DECISION: We use Postgres not SQLite");
        assert_eq!(typ, NeighborhoodType::Decision);
        assert_eq!(text, "We use Postgres not SQLite");
    }

    #[test]
    fn test_detect_neighborhood_type_preference() {
        let (typ, text) = detect_neighborhood_type("PREFERENCE: User prefers dark mode");
        assert_eq!(typ, NeighborhoodType::Preference);
        assert_eq!(text, "User prefers dark mode");
    }

    #[test]
    fn test_detect_neighborhood_type_plain() {
        let (typ, text) = detect_neighborhood_type("Just a regular insight");
        assert_eq!(typ, NeighborhoodType::Insight);
        assert_eq!(text, "Just a regular insight");
    }

    #[test]
    fn test_extract_salient_decision_prefix() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let count = extract_salient(
            &mut sys,
            "Text <salient>DECISION: use JWT not sessions</salient> more",
            &mut rng,
        );
        assert_eq!(count, 1);
        assert_eq!(sys.conscious_episode.neighborhoods.len(), 1);
        assert_eq!(
            sys.conscious_episode.neighborhoods[0].neighborhood_type,
            NeighborhoodType::Decision
        );
    }

    #[test]
    fn test_extract_salient_preference_prefix() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        extract_salient(
            &mut sys,
            "<salient>PREFERENCE: dark mode always</salient>",
            &mut rng,
        );
        assert_eq!(
            sys.conscious_episode.neighborhoods[0].neighborhood_type,
            NeighborhoodType::Preference
        );
    }

    #[test]
    fn test_mark_salient_typed_decision() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let id = mark_salient_typed(&mut sys, "DECISION: architecture is event-driven", &mut rng);

        let nbhd = sys
            .conscious_episode
            .neighborhoods
            .iter()
            .find(|n| n.id == id)
            .unwrap();
        assert_eq!(nbhd.neighborhood_type, NeighborhoodType::Decision);
        // Prefix should be stripped from the source text used to build tokens
        assert!(!nbhd.source_text.contains("DECISION:"));
    }

    #[test]
    fn test_decision_flat_score() {
        // Decisions should get DECISION_FLAT_SCORE regardless of activation count
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Add a subconscious episode with "architecture"
        let mut ep = Episode::new("Architecture notes");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["architecture", "event", "driven", "design"]),
            None,
            "architecture event driven design",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Mark a decision about architecture
        sys.add_to_conscious_typed(
            "architecture is event-driven",
            NeighborhoodType::Decision,
            &mut rng,
        );

        let result = QueryEngine::process_query(&mut sys, "architecture event");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        // The decision should appear in conscious recall with [DECIDED] prefix
        assert!(
            ctx.context.contains("[DECIDED]"),
            "decision should have [DECIDED] prefix in output, got:\n{}",
            ctx.context,
        );
    }

    #[test]
    fn test_session_dedup_skips_non_decisions() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Add regular conscious memory
        let nbhd_id = sys.add_to_conscious("quantum computing research", &mut rng);

        // Add subconscious
        let mut ep = Episode::new("Science");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics", "wave"]),
            None,
            "quantum physics wave",
            &mut rng,
        ));
        sys.add_episode(ep);

        // First query — no session recall set
        let result = QueryEngine::process_query(&mut sys, "quantum");
        let surface = compute_surface(&sys, &result);
        let ctx1 = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        assert!(ctx1.metrics.conscious > 0 || ctx1.metrics.subconscious > 0);
        assert!(!ctx1.included_ids.is_empty());

        // Second query — pass the IDs from first query as session_recalled
        let mut recalled: HashSet<Uuid> = ctx1.included_ids.iter().copied().collect();
        recalled.insert(nbhd_id); // Ensure the conscious neighborhood is in the set

        let result2 = QueryEngine::process_query(&mut sys, "quantum");
        let surface2 = compute_surface(&sys, &result2);
        let ctx2 = compose_context(
            &mut sys,
            &surface2,
            &result2,
            &result2.interference,
            None,
            Some(&recalled),
        );

        // Non-decision neighborhoods should be skipped
        // The conscious neighborhood was an Insight (default), so it should be deduped
        assert!(
            ctx2.metrics.conscious == 0,
            "non-decision conscious recall should be deduped on second query, got {}",
            ctx2.metrics.conscious,
        );
    }

    #[test]
    fn test_session_dedup_keeps_decisions() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Mark a decision
        let decision_id =
            sys.add_to_conscious_typed("always use Postgres", NeighborhoodType::Decision, &mut rng);

        // Add subconscious context that matches
        let mut ep = Episode::new("DB notes");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["postgres", "database", "sql"]),
            None,
            "postgres database sql",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Simulate session recall set containing the decision ID
        let mut recalled: HashSet<Uuid> = HashSet::new();
        recalled.insert(decision_id);

        let result = QueryEngine::process_query(&mut sys, "postgres database");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            Some(&recalled),
        );

        // Decision should still appear despite being in recalled set
        assert!(
            ctx.context.contains("[DECIDED]"),
            "decisions should survive session dedup, got:\n{}",
            ctx.context,
        );
    }

    #[test]
    fn test_recency_decay_reduces_old_scores() {
        // Two episodes with the same words but different timestamps.
        // The older one should score lower due to recency decay.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Recent episode (empty timestamp = no decay)
        let mut ep_recent = Episode::new("Recent");
        ep_recent.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta"]),
            None,
            "alpha beta recent",
            &mut rng,
        ));
        sys.add_episode(ep_recent);

        // Old episode (simulate 365 days ago via timestamp)
        let mut ep_old = Episode::new("Old");
        // Use a date 365 days in the past (rough Julian day approach)
        // Timestamp format is whatever the system uses; we set it directly
        ep_old.timestamp = "2025-02-19T00:00:00Z".to_string(); // ~365 days ago from 2026-02-19
        ep_old.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta"]),
            None,
            "alpha beta old",
            &mut rng,
        ));
        sys.add_episode(ep_old);

        let result = QueryEngine::process_query(&mut sys, "alpha beta");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        // Should have subconscious recall from at least one episode
        assert!(
            ctx.metrics.subconscious > 0,
            "should recall at least one episode",
        );

        // The context should include the recent one (higher score after decay)
        // We can't check exact scores from compose_context, but we can verify
        // the more recent one appears first in SUBCONSCIOUS RECALL 1
        if ctx.context.contains("SUBCONSCIOUS RECALL 1:") {
            let recall1_idx = ctx.context.find("SUBCONSCIOUS RECALL 1:").unwrap();
            // The first recall should reference recent content
            let after_recall1 = &ctx.context[recall1_idx..];
            assert!(
                after_recall1.contains("Recent"),
                "recent episode should rank higher than old one in first recall slot,\ngot:\n{}",
                ctx.context,
            );
        }
    }

    #[test]
    fn test_included_ids_populated() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        // included_ids should contain the neighborhood IDs that were included
        assert!(
            !ctx.included_ids.is_empty(),
            "included_ids should be populated after compose",
        );
        // Number of included IDs should match total metrics
        let total = (ctx.metrics.conscious + ctx.metrics.subconscious + ctx.metrics.novel) as usize;
        assert_eq!(
            ctx.included_ids.len(),
            total,
            "included_ids count should match metrics total",
        );
    }

    #[test]
    fn test_preference_prefix_in_output() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        sys.add_to_conscious_typed(
            "user prefers dark mode",
            NeighborhoodType::Preference,
            &mut rng,
        );

        let mut ep = Episode::new("Settings");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["user", "prefers", "dark", "mode"]),
            None,
            "user prefers dark mode",
            &mut rng,
        ));
        sys.add_episode(ep);

        let result = QueryEngine::process_query(&mut sys, "user prefers dark");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            None,
            None,
        );

        assert!(
            ctx.context.contains("[PREFERENCE]"),
            "preference type should have [PREFERENCE] prefix in output, got:\n{}",
            ctx.context,
        );
    }
}
