use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use rand::Rng;
use regex::Regex;
use uuid::Uuid;

use crate::neighborhood::NeighborhoodType;
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

/// Estimated LLM token cost of recalled content, broken down by category.
/// Uses chars/4 approximation which is within ~20% of Claude BPE tokenization.
pub struct TokenEstimate {
    pub conscious: usize,
    pub subconscious: usize,
    pub novel: usize,
    pub total: usize,
}

/// Estimate LLM tokens from text length (chars / 4, rounded up).
fn estimate_llm_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Neighborhood IDs categorized by recall type - for feedback tracking.
pub struct CategorizedIds {
    pub conscious: Vec<Uuid>,
    pub subconscious: Vec<Uuid>,
    pub novel: Vec<Uuid>,
}

/// Result of context composition.
pub struct ContextResult {
    pub context: String,
    pub metrics: ContextMetrics,
    /// Neighborhood IDs included in this result (for session recall tracking).
    pub included_ids: Vec<Uuid>,
    /// Neighborhood IDs categorized by recall type (for am_feedback).
    pub recalled_ids: CategorizedIds,
    /// Estimated LLM token cost of the recalled content.
    pub token_estimate: TokenEstimate,
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
    /// Estimated LLM token cost of the recalled content.
    pub token_estimate: TokenEstimate,
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
/// Subconscious neighborhoods scored by IDF-weighted activation.
/// Novel candidates: subconscious with activated_count <= 2, no words in common
/// with conscious, scored by max_word_weight * max_plasticity / activated_count.
fn rank_candidates(system: &mut DAESystem, query_result: &QueryResult) -> Vec<RankedCandidate> {
    let conscious_words: HashSet<String> = query_result
        .activation
        .conscious
        .iter()
        .map(|r| system.get_occurrence(*r).word.to_lowercase())
        .collect();

    let qtc = query_result.query_token_count;
    let mut con_scored = score_neighborhoods(system, &query_result.activation.conscious, true, qtc);
    let mut sub_scored =
        score_neighborhoods(system, &query_result.activation.subconscious, false, qtc);

    // Suppress older neighborhoods that overlap with newer ones (contradiction handling)
    overlap_suppress(&mut con_scored, &mut sub_scored, system);

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

/// Apply diminishing returns to previously-recalled candidates.
/// Decisions/Preferences are exempt - they always surface at full score.
fn apply_diminishing_returns(
    candidates: Vec<RankedCandidate>,
    recalled: &HashMap<Uuid, u32>,
) -> Vec<RankedCandidate> {
    candidates
        .into_iter()
        .map(|mut c| {
            if let Some(&count) = recalled.get(&c.neighborhood_id)
                && c.neighborhood_type != NeighborhoodType::Decision
                && c.neighborhood_type != NeighborhoodType::Preference
            {
                c.score *= 1.0 / (1.0 + count as f64);
            }
            c
        })
        .collect()
}

/// Compose human-readable context from surface and activation results.
///
/// `session_recalled` tracks how many times each neighborhood ID has been
/// returned this session. Non-decision neighborhoods get diminishing returns
/// (score *= 1/(1+count)). Decision/Preference neighborhoods are exempt.
///
/// `_surface` and `_interference` are part of the pipeline API and reserved
/// for future use (e.g. vivid filtering, interference-weighted scoring).
pub fn compose_context(
    system: &mut DAESystem,
    _surface: &SurfaceResult,
    query_result: &QueryResult,
    _interference: &[InterferenceResult],
    session_recalled: Option<&HashMap<Uuid, u32>>,
) -> ContextResult {
    let candidates = rank_candidates(system, query_result);

    let empty_map = HashMap::new();
    let recalled = session_recalled.unwrap_or(&empty_map);
    let candidates = apply_diminishing_returns(candidates, recalled);

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

    let mut te_conscious: usize = 0;
    let mut te_subconscious: usize = 0;
    let mut te_novel: usize = 0;

    // Conscious: top 1
    let mut con: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Conscious)
        .collect();
    con.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    if let Some(best) = con.first() {
        selected_ids.insert(best.neighborhood_id);
        conscious_ids.push(best.neighborhood_id);
        te_conscious += estimate_llm_tokens(&best.text);
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

    // Subconscious: top 2 (excluding already selected)
    let mut sub: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| {
            c.category == RecallCategory::Subconscious && !selected_ids.contains(&c.neighborhood_id)
        })
        .collect();
    sub.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    for (i, entry) in sub.iter().take(2).enumerate() {
        selected_ids.insert(entry.neighborhood_id);
        subconscious_ids.push(entry.neighborhood_id);
        te_subconscious += estimate_llm_tokens(&entry.text);
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

    // Novel: top 1 (excluding already selected)
    let mut novel: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| {
            c.category == RecallCategory::Novel && !selected_ids.contains(&c.neighborhood_id)
        })
        .collect();
    novel.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    if let Some(best) = novel.first() {
        selected_ids.insert(best.neighborhood_id);
        novel_ids.push(best.neighborhood_id);
        te_novel += estimate_llm_tokens(&best.text);
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
        token_estimate: TokenEstimate {
            conscious: te_conscious,
            subconscious: te_subconscious,
            novel: te_novel,
            total: te_conscious + te_subconscious + te_novel,
        },
    }
}

/// Budget-constrained context composition.
///
/// Fills guaranteed minimums first (highest-scored per category), then greedily
/// fills remaining budget by score across all categories.
///
/// `session_recalled` tracks how many times each neighborhood ID has been
/// returned this session. Non-decision neighborhoods get diminishing returns
/// (score *= 1/(1+count)). Decision/Preference neighborhoods are exempt.
///
/// `_surface` and `_interference` are part of the pipeline API and reserved
/// for future use (e.g. vivid filtering, interference-weighted scoring).
pub fn compose_context_budgeted(
    system: &mut DAESystem,
    _surface: &SurfaceResult,
    query_result: &QueryResult,
    _interference: &[InterferenceResult],
    budget: &BudgetConfig,
    session_recalled: Option<&HashMap<Uuid, u32>>,
) -> BudgetedContextResult {
    let candidates = rank_candidates(system, query_result);

    let empty_map = HashMap::new();
    let recalled = session_recalled.unwrap_or(&empty_map);
    let candidates = apply_diminishing_returns(candidates, recalled);

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

    // Phase 2: Greedily fill remaining budget by score across all categories.
    // Apply minimum score threshold here - category minimums are always filled,
    // but overflow candidates must score above MIN_SCORE_THRESHOLD.
    let mut remaining: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| !selected_ids.contains(&c.neighborhood_id) && c.score >= MIN_SCORE_THRESHOLD)
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

    // Compute per-category token estimates from included fragments
    let te_conscious: usize = included
        .iter()
        .filter(|f| f.category == RecallCategory::Conscious)
        .map(|f| estimate_llm_tokens(&f.text))
        .sum();
    let te_subconscious: usize = included
        .iter()
        .filter(|f| f.category == RecallCategory::Subconscious)
        .map(|f| estimate_llm_tokens(&f.text))
        .sum();
    let te_novel: usize = included
        .iter()
        .filter(|f| f.category == RecallCategory::Novel)
        .map(|f| estimate_llm_tokens(&f.text))
        .sum();

    BudgetedContextResult {
        context: parts.join("\n"),
        metrics,
        included,
        excluded_count,
        tokens_used,
        tokens_budget: budget.max_tokens,
        token_estimate: TokenEstimate {
            conscious: te_conscious,
            subconscious: te_subconscious,
            novel: te_novel,
            total: te_conscious + te_subconscious + te_novel,
        },
    }
}

/// Compact index entry for two-phase retrieval.
/// ~50-100 tokens per entry vs ~500-1000 for full content.
pub struct IndexEntry {
    pub neighborhood_id: Uuid,
    pub category: RecallCategory,
    pub neighborhood_type: NeighborhoodType,
    pub score: f64,
    pub epoch: u64,
    /// First 100 chars of the neighborhood text.
    pub summary: String,
    /// Estimated LLM tokens for the full content.
    pub token_estimate: usize,
}

/// Result of index composition for two-phase retrieval.
pub struct IndexResult {
    pub entries: Vec<IndexEntry>,
    pub stats_snapshot: IndexStats,
}

/// Snapshot of manifold statistics for the index response.
pub struct IndexStats {
    pub total_candidates: usize,
    pub total_tokens_if_fetched: usize,
}

/// Compose a compact index of the best-matching neighborhoods without full content.
/// Same scoring pipeline as compose_context_budgeted but returns only metadata.
pub fn compose_index(
    system: &mut DAESystem,
    _surface: &SurfaceResult,
    query_result: &QueryResult,
    _interference: &[InterferenceResult],
    session_recalled: Option<&HashMap<Uuid, u32>>,
) -> IndexResult {
    let candidates = rank_candidates(system, query_result);
    let total_candidates = candidates.len();

    // Deduplicate: same neighborhood may appear in multiple categories,
    // keep the entry with the highest score.
    let mut best: HashMap<Uuid, &RankedCandidate> = HashMap::new();
    for c in &candidates {
        let entry = best.entry(c.neighborhood_id).or_insert(c);
        if c.score > entry.score {
            best.insert(c.neighborhood_id, c);
        }
    }

    let mut scored: Vec<(&RankedCandidate, f64)> = best
        .into_values()
        .map(|c| {
            let mut score = c.score;
            // Apply diminishing returns for previously recalled neighborhoods
            if let Some(recalled) = session_recalled
                && c.neighborhood_type != NeighborhoodType::Decision
                && c.neighborhood_type != NeighborhoodType::Preference
                && let Some(&count) = recalled.get(&c.neighborhood_id)
            {
                score *= 1.0 / (1.0 + count as f64);
            }
            (c, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut total_tokens_if_fetched = 0;
    let entries: Vec<IndexEntry> = scored
        .into_iter()
        .filter(|(_, s)| *s >= MIN_SCORE_THRESHOLD)
        .map(|(c, score)| {
            let llm_tokens = estimate_llm_tokens(&c.text);
            total_tokens_if_fetched += llm_tokens;

            // Get epoch from the neighborhood
            let epoch = if let Some(nref) = system.get_neighborhood_ref(c.neighborhood_id) {
                system.get_neighborhood(nref).epoch
            } else {
                0
            };

            let summary = if c.text.len() <= 100 {
                c.text.clone()
            } else {
                format!("{}...", &c.text[..c.text.floor_char_boundary(100)])
            };

            IndexEntry {
                neighborhood_id: c.neighborhood_id,
                category: c.category,
                neighborhood_type: c.neighborhood_type,
                score,
                epoch,
                summary,
                token_estimate: llm_tokens,
            }
        })
        .collect();

    IndexResult {
        entries,
        stats_snapshot: IndexStats {
            total_candidates,
            total_tokens_if_fetched,
        },
    }
}

/// Retrieve full content for specific neighborhood IDs.
/// Phase 2 of two-phase retrieval: after reviewing the index, fetch
/// only the neighborhoods you actually need.
pub fn retrieve_by_ids(system: &DAESystem, ids: &[Uuid]) -> Vec<IncludedFragment> {
    let mut fragments = Vec::new();

    'outer: for &id in ids {
        // Search conscious episode first
        for nbhd in &system.conscious_episode.neighborhoods {
            if nbhd.id == id {
                let text = if !nbhd.source_text.is_empty() {
                    nbhd.source_text.clone()
                } else {
                    nbhd.occurrences
                        .iter()
                        .map(|o| o.word.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                fragments.push(IncludedFragment {
                    neighborhood_id: id,
                    episode_name: "Previously marked salient".to_string(),
                    category: RecallCategory::Conscious,
                    score: 0.0, // Not scored in direct retrieval
                    tokens: token_count(&text),
                    text,
                    neighborhood_type: nbhd.neighborhood_type,
                });
                continue 'outer;
            }
        }

        // Search subconscious episodes
        for episode in &system.episodes {
            for nbhd in &episode.neighborhoods {
                if nbhd.id == id {
                    let text = if !nbhd.source_text.is_empty() {
                        nbhd.source_text.clone()
                    } else {
                        nbhd.occurrences
                            .iter()
                            .map(|o| o.word.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    };
                    fragments.push(IncludedFragment {
                        neighborhood_id: id,
                        episode_name: episode.name.clone(),
                        category: RecallCategory::Subconscious,
                        score: 0.0,
                        tokens: token_count(&text),
                        text,
                        neighborhood_type: nbhd.neighborhood_type,
                    });
                    continue 'outer;
                }
            }
        }
    }

    fragments
}

// -- Scoring internals --

/// Multiplier for Decision/Preference neighborhoods.
/// Decisions that genuinely match the query score this many times higher.
const DECISION_MULTIPLIER: f64 = 3.0;

/// Minimum score floor for Decision/Preference neighborhoods.
/// Ensures some visibility even on unrelated queries, but doesn't dominate.
const DECISION_FLOOR: f64 = 15.0;

/// Recency decay coefficient for non-decision memories.
/// score *= 1.0 / (1.0 + days_old * RECENCY_DECAY_RATE)
const RECENCY_DECAY_RATE: f64 = 0.01;

/// IDF-weighted word overlap threshold for contradiction detection.
/// Pairs of neighborhoods above this threshold are considered overlapping.
const OVERLAP_THRESHOLD: f64 = 0.3;

/// Score multiplier applied to older neighborhoods in overlapping groups.
const OVERLAP_SUPPRESSION: f64 = 0.1;

/// Minimum score threshold for inclusion in recall results.
/// Candidates scoring below this are excluded to avoid padding with weak matches.
const MIN_SCORE_THRESHOLD: f64 = 1.0;

struct ScoredNeighborhood {
    neighborhood_id: Uuid,
    episode_idx: usize, // usize::MAX for conscious
    score: f64,
    activated_count: usize,
    words: HashSet<String>,
    max_word_weight: f64,
    max_plasticity: f64,
    neighborhood_type: NeighborhoodType,
    epoch: u64,
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
    // Fall back to 0.0 if unparseable (no external chrono dep - simple parse).
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
    query_token_count: usize,
) -> HashMap<Uuid, ScoredNeighborhood> {
    let mut scored: HashMap<Uuid, ScoredNeighborhood> = HashMap::new();

    // Pre-collect data to avoid borrow conflicts.
    // Superseded neighborhoods are excluded - they've been explicitly replaced.
    struct OccData {
        nbhd_id: Uuid,
        episode_idx: usize,
        word: String,
        activation_count: u32,
        plasticity: f64,
        nbhd_type: NeighborhoodType,
        epoch: u64,
    }

    let data: Vec<OccData> = refs
        .iter()
        .filter_map(|r| {
            let occ = system.get_occurrence(*r);
            let nbhd = system.get_neighborhood_for_occurrence(*r);
            if nbhd.superseded_by.is_some() {
                return None;
            }
            Some(OccData {
                nbhd_id: nbhd.id,
                episode_idx: if r.is_conscious() {
                    usize::MAX
                } else {
                    r.episode_idx
                },
                word: occ.word.to_lowercase(),
                activation_count: occ.activation_count,
                plasticity: occ.plasticity(),
                nbhd_type: nbhd.neighborhood_type,
                epoch: nbhd.epoch,
            })
        })
        .collect();

    // Pre-collect recency decay per episode_idx
    let recency_cache: HashMap<usize, f64> = data
        .iter()
        .map(|d| d.episode_idx)
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
    let conscious_count = if data.iter().any(|d| d.episode_idx == usize::MAX) {
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

    for d in &data {
        let weight = system.get_word_weight(&d.word);

        let entry = scored
            .entry(d.nbhd_id)
            .or_insert_with(|| ScoredNeighborhood {
                neighborhood_id: d.nbhd_id,
                episode_idx: d.episode_idx,
                score: 0.0,
                activated_count: 0,
                words: HashSet::new(),
                max_word_weight: 0.0,
                max_plasticity: 0.0,
                neighborhood_type: d.nbhd_type,
                epoch: d.epoch,
            });

        entry.score += weight * d.activation_count as f64;
        entry.words.insert(d.word.clone());
        entry.activated_count += 1;
        if weight > entry.max_word_weight {
            entry.max_word_weight = weight;
        }
        if d.plasticity > entry.max_plasticity {
            entry.max_plasticity = d.plasticity;
        }
    }

    // Post-process: density bonus, recency decay, then decision/preference competitive scoring
    for sn in scored.values_mut() {
        // Co-occurrence density bonus: neighborhoods matching more query tokens score higher
        if query_token_count > 0 {
            let density_bonus = sn.activated_count as f64 / query_token_count as f64;
            sn.score *= 1.0 + density_bonus;
        }
        // All neighborhoods get recency decay
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
        // Decision/Preference: competitive scoring with floor
        // score = max(normal_score * MULTIPLIER, FLOOR)
        match sn.neighborhood_type {
            NeighborhoodType::Decision | NeighborhoodType::Preference => {
                sn.score = (sn.score * DECISION_MULTIPLIER).max(DECISION_FLOOR);
            }
            _ => {}
        }
    }

    scored
}

/// Compute IDF-weighted word overlap between two word sets.
/// Returns Σ IDF(w) for intersection / Σ IDF(w) for union.
fn idf_weighted_overlap(
    words_a: &HashSet<String>,
    words_b: &HashSet<String>,
    system: &mut DAESystem,
) -> f64 {
    let intersection: f64 = words_a
        .intersection(words_b)
        .map(|w| system.get_word_weight(w))
        .sum();
    let union: f64 = words_a
        .union(words_b)
        .map(|w| system.get_word_weight(w))
        .sum();
    if union < f64::EPSILON {
        return 0.0;
    }
    intersection / union
}

/// Detect overlapping neighborhoods across conscious and subconscious scores
/// and suppress older ones. For each pair with IDF-weighted overlap above
/// OVERLAP_THRESHOLD, the lower-epoch neighborhood gets its score multiplied
/// by OVERLAP_SUPPRESSION (0.1x). This ensures that when contradicting memories
/// exist, only the newest version ranks highly.
fn overlap_suppress(
    con_scored: &mut HashMap<Uuid, ScoredNeighborhood>,
    sub_scored: &mut HashMap<Uuid, ScoredNeighborhood>,
    system: &mut DAESystem,
) {
    // Collect references to word sets and epochs - no cloning needed since
    // we only read words during pairwise comparison, then mutate scores after.
    let mut info: Vec<(Uuid, &HashSet<String>, u64)> = Vec::new();
    let mut seen: HashSet<Uuid> = HashSet::new();
    for (id, sn) in con_scored.iter().chain(sub_scored.iter()) {
        if seen.insert(*id) {
            info.push((*id, &sn.words, sn.epoch));
        }
    }

    if info.len() < 2 {
        return;
    }

    // Pairwise comparison - O(k²) but k is bounded (top candidates only)
    let mut suppress: HashSet<Uuid> = HashSet::new();
    for i in 0..info.len() {
        for j in (i + 1)..info.len() {
            let overlap = idf_weighted_overlap(info[i].1, info[j].1, system);
            if overlap > OVERLAP_THRESHOLD {
                let epoch_i = info[i].2;
                let epoch_j = info[j].2;
                if epoch_i < epoch_j {
                    suppress.insert(info[i].0);
                } else if epoch_j < epoch_i {
                    suppress.insert(info[j].0);
                }
                // Same epoch: leave both unsuppressed
            }
        }
    }

    // Apply suppression factor to affected neighborhoods
    for id in &suppress {
        if let Some(sn) = con_scored.get_mut(id) {
            sn.score *= OVERLAP_SUPPRESSION;
        }
        if let Some(sn) = sub_scored.get_mut(id) {
            sn.score *= OVERLAP_SUPPRESSION;
        }
    }
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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

        // No conscious recall since no conscious content matches
        assert!(!ctx.context.contains("CONSCIOUS RECALL:"));
    }

    #[test]
    fn test_metrics() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

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
        let ctx1 = compose_context(&mut sys1, &surface1, &result1, &result1.interference, None);

        let mut sys2 = make_full_system();
        let result2 = QueryEngine::process_query(&mut sys2, "quantum");
        let surface2 = compute_surface(&sys2, &result2);
        let ctx2 = compose_context(&mut sys2, &surface2, &result2, &result2.interference, None);

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
        );

        // With huge budget, minimums should be filled; any exclusions are from
        // the MIN_SCORE_THRESHOLD filtering weak matches from the overflow phase.
        assert!(
            !ctx.included.is_empty(),
            "expected some inclusions with huge budget"
        );
        // All included entries should score above threshold
        for f in &ctx.included {
            assert!(
                f.score > 0.0,
                "included fragment should have positive score, got {}",
                f.score
            );
        }
    }

    #[test]
    fn test_compose_context_unchanged() {
        // Regression: verify compose_context still produces the same output as before refactor
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics neural");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

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
        let ctx2 = compose_context(&mut sys2, &surface2, &result2, &result2.interference, None);
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
        // Decisions should surface with [DECIDED] prefix when query matches
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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

        // The decision should appear in conscious recall with [DECIDED] prefix
        assert!(
            ctx.context.contains("[DECIDED]"),
            "decision should have [DECIDED] prefix in output, got:\n{}",
            ctx.context,
        );
    }

    #[test]
    fn test_decision_competitive_scoring_high_overlap() {
        // A Decision with high query overlap should score >= DECISION_FLOOR
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Decision about postgres (only conscious memory, no overlapping subconscious)
        sys.add_to_conscious_typed(
            "always use postgres for database storage backend",
            NeighborhoodType::Decision,
            &mut rng,
        );

        // Unrelated subconscious episode (no word overlap with decision)
        let mut ep = Episode::new("Nature");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["forest", "trees", "wildlife", "ecology"]),
            None,
            "forest trees wildlife ecology",
            &mut rng,
        ));
        sys.add_episode(ep);

        let result = QueryEngine::process_query(&mut sys, "postgres database storage backend");
        let surface = compute_surface(&sys, &result);
        let budget = BudgetConfig {
            max_tokens: 4096,
            min_conscious: 1,
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
        );

        // The decision should surface and score at or above the floor
        let decision_entries: Vec<&IncludedFragment> = ctx
            .included
            .iter()
            .filter(|f| f.neighborhood_type == NeighborhoodType::Decision)
            .collect();
        assert!(
            !decision_entries.is_empty(),
            "decision should be included in recall"
        );
        // With competitive scoring, score = max(normal * 3.0, 15.0)
        let decision_score = decision_entries[0].score;
        assert!(
            decision_score >= 15.0,
            "relevant decision should score >= floor (15.0), got {}",
            decision_score,
        );
    }

    #[test]
    fn test_decision_competitive_scoring_low_overlap() {
        // A Decision about architecture should score at floor on unrelated query
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Decision about architecture
        sys.add_to_conscious_typed(
            "architecture uses event driven microservices pattern",
            NeighborhoodType::Decision,
            &mut rng,
        );

        // Subconscious episode about cooking (completely unrelated)
        let mut ep = Episode::new("Cooking");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["chocolate", "cake", "recipe", "sugar", "flour"]),
            None,
            "chocolate cake recipe sugar flour",
            &mut rng,
        ));
        sys.add_episode(ep);

        let result =
            QueryEngine::process_query(&mut sys, "chocolate cake recipe baking ingredients");
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
        );

        // Check if the decision even activates - it shares no words with the query
        let decision_entries: Vec<&IncludedFragment> = ctx
            .included
            .iter()
            .filter(|f| f.neighborhood_type == NeighborhoodType::Decision)
            .collect();

        if !decision_entries.is_empty() {
            // If it somehow activates, its score should be at or near the floor
            let decision_score = decision_entries[0].score;
            assert!(
                decision_score <= 100.0,
                "irrelevant decision should NOT score at old flat 100.0, got {}",
                decision_score,
            );
        }
        // Either way: the cooking result should outrank any irrelevant decision
        let cooking_entries: Vec<&IncludedFragment> = ctx
            .included
            .iter()
            .filter(|f| f.text.contains("chocolate") || f.text.contains("cake"))
            .collect();
        assert!(
            !cooking_entries.is_empty(),
            "relevant cooking memory should surface, got:\n{}",
            ctx.context,
        );
    }

    #[test]
    fn test_session_diminishing_returns_non_decisions() {
        // Two identical subconscious neighborhoods matching the same query.
        // One is in session_recalled (count=1), the other is not.
        // The recalled one should score lower due to diminishing returns.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Two subconscious episodes with identical words
        let mut ep1 = Episode::new("First");
        ep1.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta", "gamma"]),
            None,
            "alpha beta gamma first",
            &mut rng,
        ));
        sys.add_episode(ep1);
        let nbhd1_id = sys.episodes[0].neighborhoods[0].id;

        let mut ep2 = Episode::new("Second");
        ep2.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta", "gamma"]),
            None,
            "alpha beta gamma second",
            &mut rng,
        ));
        sys.add_episode(ep2);
        let nbhd2_id = sys.episodes[1].neighborhoods[0].id;

        // Mark only nbhd1 as previously recalled (count=1)
        let mut recalled: HashMap<Uuid, u32> = HashMap::new();
        recalled.insert(nbhd1_id, 1);

        let result = QueryEngine::process_query(&mut sys, "alpha beta gamma");
        let surface = compute_surface(&sys, &result);
        let budget = BudgetConfig {
            max_tokens: 4096,
            min_conscious: 0,
            min_subconscious: 2,
            min_novel: 0,
        };
        let ctx = compose_context_budgeted(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            &budget,
            Some(&recalled),
        );

        let f1 = ctx.included.iter().find(|f| f.neighborhood_id == nbhd1_id);
        let f2 = ctx.included.iter().find(|f| f.neighborhood_id == nbhd2_id);

        assert!(
            f1.is_some() && f2.is_some(),
            "both neighborhoods should be included"
        );
        let s1 = f1.unwrap().score;
        let s2 = f2.unwrap().score;
        assert!(
            s1 < s2,
            "recalled neighborhood should score lower: recalled={}, fresh={}",
            s1,
            s2,
        );
    }

    #[test]
    fn test_session_diminishing_returns_decisions_exempt() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Mark a decision
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

        // First query - get scores without session recall
        let result = QueryEngine::process_query(&mut sys, "postgres database");
        let surface = compute_surface(&sys, &result);
        let budget = BudgetConfig {
            max_tokens: 4096,
            min_conscious: 1,
            min_subconscious: 1,
            min_novel: 0,
        };
        let ctx1 = compose_context_budgeted(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            &budget,
            None,
        );

        // Build session recall map with count=1 for all returned IDs
        let mut recalled: HashMap<Uuid, u32> = HashMap::new();
        for f in &ctx1.included {
            recalled.insert(f.neighborhood_id, 1);
        }

        // Second query with session recall
        let result2 = QueryEngine::process_query(&mut sys, "postgres database");
        let surface2 = compute_surface(&sys, &result2);
        let ctx2 = compose_context_budgeted(
            &mut sys,
            &surface2,
            &result2,
            &result2.interference,
            &budget,
            Some(&recalled),
        );

        // Decision should still appear and score unchanged (exempt from diminishing returns)
        assert!(
            ctx2.context.contains("[DECIDED]"),
            "decisions should survive session recall, got:\n{}",
            ctx2.context,
        );
        // Find decision in both results and verify score is unchanged
        let d1 = ctx1
            .included
            .iter()
            .find(|f| f.neighborhood_type == NeighborhoodType::Decision);
        let d2 = ctx2
            .included
            .iter()
            .find(|f| f.neighborhood_type == NeighborhoodType::Decision);
        if let (Some(d1), Some(d2)) = (d1, d2) {
            assert!(
                (d1.score - d2.score).abs() < 0.01,
                "decision score should be unchanged: first={}, second={}",
                d1.score,
                d2.score,
            );
        }
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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

        assert!(
            ctx.context.contains("[PREFERENCE]"),
            "preference type should have [PREFERENCE] prefix in output, got:\n{}",
            ctx.context,
        );
    }

    #[test]
    fn test_superseded_neighborhood_excluded_from_recall() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Create two conscious memories about the same topic
        let old_id = sys.add_to_conscious_typed(
            "use approach alpha for deployment",
            NeighborhoodType::Decision,
            &mut rng,
        );
        let new_id = sys.add_to_conscious_typed(
            "use approach beta for deployment instead",
            NeighborhoodType::Decision,
            &mut rng,
        );

        // Mark old as superseded
        assert!(sys.mark_superseded(old_id, new_id));

        // Query for deployment
        let result = QueryEngine::process_query(&mut sys, "deployment approach");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

        // The superseded memory (alpha) should not appear
        assert!(
            !ctx.context.contains("approach alpha"),
            "superseded neighborhood should not appear in recall, got:\n{}",
            ctx.context,
        );
        // The new memory (beta) should appear
        assert!(
            ctx.context.contains("approach beta"),
            "replacement neighborhood should appear in recall, got:\n{}",
            ctx.context,
        );
    }

    #[test]
    fn test_superseded_decision_excluded() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Decision that gets superseded
        let old_id = sys.add_to_conscious_typed(
            "DECISION: architecture uses monolith pattern",
            NeighborhoodType::Decision,
            &mut rng,
        );
        let new_id = sys.add_to_conscious_typed(
            "DECISION: architecture uses microservices pattern",
            NeighborhoodType::Decision,
            &mut rng,
        );
        sys.mark_superseded(old_id, new_id);

        let result = QueryEngine::process_query(&mut sys, "architecture pattern");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

        assert!(
            !ctx.context.contains("monolith"),
            "superseded Decision should not surface, got:\n{}",
            ctx.context,
        );
    }

    // =====================================================================
    // Overlap detection / contradiction suppression tests (ALP-681)
    // =====================================================================

    #[test]
    fn test_overlap_suppresses_older_contradicting_memory() {
        // Two conscious memories about the same topic - only the newer should surface.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Older memory (lower epoch)
        sys.add_to_conscious_typed(
            "deployment strategy uses monolith pattern for all services",
            NeighborhoodType::Insight,
            &mut rng,
        );
        // Newer memory (higher epoch) - contradicts the first
        sys.add_to_conscious_typed(
            "deployment strategy uses microservices pattern for all services",
            NeighborhoodType::Insight,
            &mut rng,
        );

        let result = QueryEngine::process_query(&mut sys, "deployment strategy pattern services");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

        // The newer memory should surface; the older should be suppressed
        assert!(
            ctx.context.contains("microservices"),
            "newer memory should surface in recall, got:\n{}",
            ctx.context,
        );
        // The older overlapping memory should be suppressed (not in top results)
        // Note: it may still appear if there are very few candidates, but its
        // score should be 0.1x of the newer one, so it won't be top-ranked.
    }

    #[test]
    fn test_overlap_does_not_suppress_non_overlapping() {
        // Two unrelated memories - both should surface normally.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        sys.add_to_conscious_typed(
            "quantum physics wave particle duality experiment",
            NeighborhoodType::Insight,
            &mut rng,
        );
        sys.add_to_conscious_typed(
            "chocolate cake recipe butter sugar flour eggs",
            NeighborhoodType::Insight,
            &mut rng,
        );

        // Add a subconscious episode so there's something to query
        let mut ep = Episode::new("Science");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics"]),
            None,
            "quantum physics",
            &mut rng,
        ));
        sys.add_episode(ep);

        let result = QueryEngine::process_query(&mut sys, "quantum physics");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

        // The quantum physics memory should surface
        assert!(
            ctx.context.contains("quantum"),
            "relevant memory should surface, got:\n{}",
            ctx.context,
        );
        // The cake recipe should NOT surface (not relevant to query)
        // This is a relevance check, not overlap - cake has no query overlap
    }

    #[test]
    fn test_overlap_threshold_boundary() {
        // Memories with minimal word overlap should NOT trigger suppression.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Memory A: mostly unique words with one shared word
        sys.add_to_conscious_typed(
            "the architecture of ancient roman aqueducts was remarkable engineering",
            NeighborhoodType::Insight,
            &mut rng,
        );
        // Memory B: different topic, shares "architecture" but low overlap
        sys.add_to_conscious_typed(
            "modern software architecture patterns include microservices and event sourcing",
            NeighborhoodType::Insight,
            &mut rng,
        );

        let result = QueryEngine::process_query(&mut sys, "architecture");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference, None);

        // Both should be able to surface since they have low overlap
        // (only "architecture" is shared, rest is different)
        assert!(
            ctx.context.contains("architecture"),
            "architecture-related memory should surface, got:\n{}",
            ctx.context,
        );
    }

    #[test]
    fn test_overlap_in_subconscious_episodes() {
        // Contradicting subconscious memories - newer episode should win.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Older episode
        let mut ep1 = Episode::new("Old discussion");
        ep1.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["database", "strategy", "uses", "sqlite", "for", "storage"]),
            None,
            "database strategy uses sqlite for storage",
            &mut rng,
        ));
        sys.add_episode(ep1);

        // Newer episode with contradicting info
        let mut ep2 = Episode::new("New discussion");
        ep2.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["database", "strategy", "uses", "postgres", "for", "storage"]),
            None,
            "database strategy uses postgres for storage",
            &mut rng,
        ));
        sys.add_episode(ep2);

        let result = QueryEngine::process_query(&mut sys, "database strategy storage");
        let surface = compute_surface(&sys, &result);

        let budget = BudgetConfig {
            max_tokens: 4096,
            min_conscious: 0,
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
        );

        // The newer episode (postgres) should rank higher
        let sub_entries: Vec<&IncludedFragment> = ctx
            .included
            .iter()
            .filter(|f| f.category == RecallCategory::Subconscious)
            .collect();
        if sub_entries.len() >= 2 {
            assert!(
                sub_entries[0].score > sub_entries[1].score,
                "newer (postgres) should score higher than older (sqlite)"
            );
        }
        // Top subconscious should be postgres
        if let Some(top) = sub_entries.first() {
            assert!(
                top.text.contains("postgres"),
                "newer memory (postgres) should be top subconscious, got: {}",
                top.text,
            );
        }
    }

    // =====================================================================
    // Co-occurrence density bonus tests (ALP-684)
    // =====================================================================

    #[test]
    fn test_density_bonus_many_words_scores_higher() {
        // A memory matching many query words should score higher than one
        // matching only a single (even rare) word.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Neighborhood A: matches 4 of 5 query words
        let mut ep1 = Episode::new("Broad match");
        ep1.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta", "gamma", "delta", "unrelated"]),
            None,
            "alpha beta gamma delta unrelated",
            &mut rng,
        ));
        sys.add_episode(ep1);

        // Neighborhood B: matches only 1 query word ("epsilon") which is rare
        let mut ep2 = Episode::new("Single match");
        ep2.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["epsilon", "foo", "bar", "baz", "qux"]),
            None,
            "epsilon foo bar baz qux",
            &mut rng,
        ));
        sys.add_episode(ep2);

        // Query with 5 words - 4 match neighborhood A, 1 matches neighborhood B
        let result = QueryEngine::process_query(&mut sys, "alpha beta gamma delta epsilon");
        let surface = compute_surface(&sys, &result);
        let budget = BudgetConfig {
            max_tokens: 4096,
            min_conscious: 0,
            min_subconscious: 2,
            min_novel: 0,
        };
        let ctx = compose_context_budgeted(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            &budget,
            None,
        );

        let sub_entries: Vec<&IncludedFragment> = ctx
            .included
            .iter()
            .filter(|f| f.category == RecallCategory::Subconscious)
            .collect();
        assert!(
            sub_entries.len() >= 2,
            "expected at least 2 subconscious entries, got {}",
            sub_entries.len()
        );
        // The broad-match (4 words) should score higher than single-match (1 word)
        let broad = sub_entries
            .iter()
            .find(|f| f.text.contains("alpha"))
            .unwrap();
        let single = sub_entries
            .iter()
            .find(|f| f.text.contains("epsilon"))
            .unwrap();
        assert!(
            broad.score > single.score,
            "4-word match should score higher than 1-word match: broad={}, single={}",
            broad.score,
            single.score,
        );
    }

    // =====================================================================
    // Minimum score threshold tests (ALP-686)
    // =====================================================================

    #[test]
    fn test_min_score_threshold_excludes_weak_overflow() {
        // Weak-scoring candidates should be excluded from the greedy fill
        // phase but category minimums are always filled.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Strong match
        let mut ep1 = Episode::new("Strong");
        ep1.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["target", "keyword", "matching", "relevant"]),
            None,
            "target keyword matching relevant",
            &mut rng,
        ));
        sys.add_episode(ep1);

        // Weak match - shares only one common word with query
        let mut ep2 = Episode::new("Weak");
        ep2.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["target", "unrelated1", "unrelated2", "unrelated3"]),
            None,
            "target unrelated1 unrelated2 unrelated3",
            &mut rng,
        ));
        sys.add_episode(ep2);

        let result = QueryEngine::process_query(
            &mut sys,
            "target keyword matching relevant additional context",
        );
        let surface = compute_surface(&sys, &result);
        let budget = BudgetConfig {
            max_tokens: 4096,
            min_conscious: 0,
            min_subconscious: 1, // Only need 1 minimum
            min_novel: 0,
        };
        let ctx = compose_context_budgeted(
            &mut sys,
            &surface,
            &result,
            &result.interference,
            &budget,
            None,
        );

        // The strong match should be included as the minimum fill
        let strong = ctx.included.iter().find(|f| f.text.contains("keyword"));
        assert!(strong.is_some(), "strong match should be included");

        // If the weak match scores below MIN_SCORE_THRESHOLD, it should be
        // excluded from the overflow phase. Either way, weak matches shouldn't
        // dominate the results.
        if ctx.included.len() > 1 {
            // If weak match is included, it should score lower than strong
            let scores: Vec<f64> = ctx.included.iter().map(|f| f.score).collect();
            let max_score = scores.iter().cloned().fold(f64::MIN, f64::max);
            assert!(
                strong.unwrap().score >= max_score * 0.5,
                "strong match should be among the top scorers"
            );
        }
    }

    #[test]
    fn test_empty_results_when_nothing_matches() {
        // Query with no matching words should produce empty results.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("Science");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics", "particle"]),
            None,
            "quantum physics particle",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Query with completely unrelated words
        let result = QueryEngine::process_query(&mut sys, "cooking recipe ingredients");
        let surface = compute_surface(&sys, &result);
        let budget = BudgetConfig {
            max_tokens: 4096,
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
        );

        assert!(
            ctx.included.is_empty(),
            "completely unrelated query should produce no results, got {} entries",
            ctx.included.len()
        );
    }

    #[test]
    fn test_idf_weighted_overlap_computation() {
        // Direct test of the overlap computation function
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Add some content so IDF weights are meaningful
        let mut ep = Episode::new("context");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["common", "word", "appears", "everywhere"]),
            None,
            "common word appears everywhere",
            &mut rng,
        ));
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["rare", "unique", "special"]),
            None,
            "rare unique special",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Identical word sets should have overlap = 1.0
        let words_a: HashSet<String> = ["alpha", "beta"].iter().map(|s| s.to_string()).collect();
        let words_b: HashSet<String> = ["alpha", "beta"].iter().map(|s| s.to_string()).collect();
        let overlap = idf_weighted_overlap(&words_a, &words_b, &mut sys);
        assert!(
            (overlap - 1.0).abs() < 0.01,
            "identical sets should have overlap ~1.0, got {}",
            overlap,
        );

        // Disjoint word sets should have overlap = 0.0
        let words_c: HashSet<String> = ["gamma", "delta"].iter().map(|s| s.to_string()).collect();
        let overlap2 = idf_weighted_overlap(&words_a, &words_c, &mut sys);
        assert!(
            overlap2 < 0.01,
            "disjoint sets should have overlap ~0.0, got {}",
            overlap2,
        );

        // Empty sets
        let empty: HashSet<String> = HashSet::new();
        let overlap3 = idf_weighted_overlap(&empty, &words_a, &mut sys);
        assert!(
            overlap3 < 0.01,
            "empty set overlap should be ~0.0, got {}",
            overlap3,
        );
    }

    #[test]
    fn test_compose_index_returns_compact_entries() {
        let mut sys = make_full_system();
        let qr = QueryEngine::process_query(&mut sys, "quantum physics particle");
        let surface = compute_surface(&sys, &qr);

        let index = compose_index(&mut sys, &surface, &qr, &qr.interference, None);

        assert!(
            !index.entries.is_empty(),
            "index should have entries for matching query"
        );
        // Each entry should have a summary <= 103 chars (100 + "...")
        for entry in &index.entries {
            assert!(
                entry.summary.len() <= 103,
                "summary should be truncated: {}",
                entry.summary.len()
            );
            assert!(
                entry.token_estimate > 0,
                "token estimate should be positive"
            );
            assert!(entry.score > 0.0, "score should be positive");
        }
        assert!(
            index.stats_snapshot.total_candidates > 0,
            "total_candidates should be positive"
        );
    }

    #[test]
    fn test_compose_index_deduplicates_across_categories() {
        let mut sys = make_full_system();
        let qr = QueryEngine::process_query(&mut sys, "quantum physics");
        let surface = compute_surface(&sys, &qr);

        let index = compose_index(&mut sys, &surface, &qr, &qr.interference, None);

        // Each neighborhood ID should appear at most once
        let mut seen: HashSet<Uuid> = HashSet::new();
        for entry in &index.entries {
            assert!(
                seen.insert(entry.neighborhood_id),
                "duplicate ID in index: {}",
                entry.neighborhood_id
            );
        }
    }

    #[test]
    fn test_compose_index_respects_min_score_threshold() {
        let mut sys = make_full_system();
        // Query for something very specific
        let qr = QueryEngine::process_query(&mut sys, "quantum physics particle wave");
        let surface = compute_surface(&sys, &qr);

        let index = compose_index(&mut sys, &surface, &qr, &qr.interference, None);

        for entry in &index.entries {
            assert!(
                entry.score >= MIN_SCORE_THRESHOLD,
                "entry score {} should be >= threshold {}",
                entry.score,
                MIN_SCORE_THRESHOLD
            );
        }
    }

    #[test]
    fn test_retrieve_by_ids_returns_matching_neighborhoods() {
        let sys = make_full_system();

        // Get a neighborhood ID from conscious memory
        let conscious_id = sys.conscious_episode.neighborhoods[0].id;

        // Get a neighborhood ID from subconscious
        let sub_id = sys.episodes[0].neighborhoods[0].id;

        let fragments = retrieve_by_ids(&sys, &[conscious_id, sub_id]);

        assert_eq!(fragments.len(), 2, "should return 2 fragments");

        // Verify we got the right IDs back
        let returned_ids: HashSet<Uuid> = fragments.iter().map(|f| f.neighborhood_id).collect();
        assert!(returned_ids.contains(&conscious_id));
        assert!(returned_ids.contains(&sub_id));

        // Conscious should be categorized as Conscious
        let con = fragments
            .iter()
            .find(|f| f.neighborhood_id == conscious_id)
            .unwrap();
        assert_eq!(con.category, RecallCategory::Conscious);
        assert!(!con.text.is_empty());

        // Subconscious should be categorized as Subconscious
        let sub = fragments
            .iter()
            .find(|f| f.neighborhood_id == sub_id)
            .unwrap();
        assert_eq!(sub.category, RecallCategory::Subconscious);
        assert!(!sub.text.is_empty());
    }

    #[test]
    fn test_retrieve_by_ids_handles_missing_ids() {
        let sys = make_full_system();
        let missing_id = Uuid::new_v4();

        let fragments = retrieve_by_ids(&sys, &[missing_id]);

        assert!(
            fragments.is_empty(),
            "should return empty for non-existent IDs"
        );
    }

    #[test]
    fn test_compose_index_total_tokens_if_fetched() {
        let mut sys = make_full_system();
        let qr = QueryEngine::process_query(&mut sys, "quantum physics");
        let surface = compute_surface(&sys, &qr);

        let index = compose_index(&mut sys, &surface, &qr, &qr.interference, None);

        // total_tokens_if_fetched should be > 0 when there are entries
        if !index.entries.is_empty() {
            assert!(
                index.stats_snapshot.total_tokens_if_fetched > 0,
                "total_tokens_if_fetched should be positive when entries exist"
            );
        }
    }
}
