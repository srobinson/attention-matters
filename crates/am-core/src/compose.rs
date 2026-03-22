use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::neighborhood::NeighborhoodType;
use crate::query::QueryResult;
use crate::scoring::{MIN_SCORE_THRESHOLD, RankedCandidate, get_episode_name, rank_candidates};
use crate::surface::SurfaceResult;
use crate::system::DAESystem;
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
    /// Neighborhood IDs categorized by recall type (for `am_feedback`).
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
            lines.push(format!("SUBCONSCIOUS RECALL {index}:"));
            lines.push(format!("[Source: {ep_name}]"));
        }
        RecallCategory::Novel => {
            lines.push("NOVEL CONNECTION:".to_string());
            lines.push(format!("[Source: {ep_name}]"));
        }
    }
    // Decisions get [DECIDED] prefix so the AI knows not to re-litigate
    let formatted_text = if nbhd_type == NeighborhoodType::Decision {
        format!("[DECIDED] {text}")
    } else if nbhd_type == NeighborhoodType::Preference {
        format!("[PREFERENCE] {text}")
    } else {
        text.to_string()
    };
    lines.push(format!("\"{formatted_text}\""));
    lines
}

const ENTRY_HEADER_OVERHEAD_TOKENS: usize = 20;

/// Apply diminishing returns to previously-recalled candidates.
/// Decision/Preference types get softer decay (0.5x rate) instead of full exemption.
fn apply_diminishing_returns(
    candidates: Vec<RankedCandidate>,
    recalled: &HashMap<Uuid, u32>,
) -> Vec<RankedCandidate> {
    candidates
        .into_iter()
        .map(|mut c| {
            if let Some(&count) = recalled.get(&c.neighborhood_id) {
                let decay_rate = match c.neighborhood_type {
                    NeighborhoodType::Decision | NeighborhoodType::Preference => 0.5,
                    _ => 1.0,
                };
                c.score *= 1.0 / (1.0 + f64::from(count) * decay_rate);
            }
            c
        })
        .collect()
}

/// Compose human-readable context from surface and activation results.
///
/// `session_recalled` tracks how many times each neighborhood ID has been
/// returned this session. All neighborhoods get diminishing returns -
/// Decision/Preference types use softer decay (0.5x rate).
///
/// Interference gates neighborhood scores; vivid neighborhoods get boosted.
///
/// # Examples
///
/// Full ingest, query, compose pipeline:
///
/// ```
/// use am_core::{system::DAESystem, query::QueryEngine, compose::compose_context, surface::compute_surface, tokenizer::ingest_text};
/// use rand::SeedableRng;
/// use rand::rngs::SmallRng;
///
/// let mut system = DAESystem::new("demo");
/// let mut rng = SmallRng::seed_from_u64(42);
///
/// // Ingest some content
/// let ep = ingest_text("Geometric memory uses quaternions on S3", None, &mut rng);
/// system.add_episode(ep);
///
/// // Query and compose
/// let qr = QueryEngine::process_query(&mut system, "quaternions");
/// let surface = compute_surface(&system, &qr);
/// let ctx = compose_context(&mut system, &surface, &qr, None);
///
/// // included_ids tracks which neighborhoods contributed to the result
/// assert_eq!(ctx.included_ids.len(), ctx.recalled_ids.conscious.len()
///     + ctx.recalled_ids.subconscious.len() + ctx.recalled_ids.novel.len());
/// ```
pub fn compose_context(
    system: &mut DAESystem,
    surface: &SurfaceResult,
    query_result: &QueryResult,
    session_recalled: Option<&HashMap<Uuid, u32>>,
) -> ContextResult {
    let candidates = rank_candidates(system, query_result, &query_result.interference, surface);

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
    con.sort_by(|a, b| b.score.total_cmp(&a.score));

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
    sub.sort_by(|a, b| b.score.total_cmp(&a.score));

    for (i, entry) in sub.iter().take(2).enumerate() {
        selected_ids.insert(entry.neighborhood_id);
        subconscious_ids.push(entry.neighborhood_id);
        te_subconscious += estimate_llm_tokens(&entry.text);
        let ep_name = get_episode_name(system, entry.episode_ref);
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
    novel.sort_by(|a, b| b.score.total_cmp(&a.score));

    if let Some(best) = novel.first() {
        selected_ids.insert(best.neighborhood_id);
        novel_ids.push(best.neighborhood_id);
        te_novel += estimate_llm_tokens(&best.text);
        let ep_name = get_episode_name(system, best.episode_ref);
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
/// returned this session. All neighborhoods get diminishing returns -
/// Decision/Preference types use softer decay (0.5x rate).
///
/// Interference gates neighborhood scores; vivid neighborhoods get boosted.
pub fn compose_context_budgeted(
    system: &mut DAESystem,
    surface: &SurfaceResult,
    query_result: &QueryResult,
    budget: &BudgetConfig,
    session_recalled: Option<&HashMap<Uuid, u32>>,
) -> BudgetedContextResult {
    let candidates = rank_candidates(system, query_result, &query_result.interference, surface);

    let empty_map = HashMap::new();
    let recalled = session_recalled.unwrap_or(&empty_map);
    let candidates = apply_diminishing_returns(candidates, recalled);

    // Split candidates by category, sorted by score desc
    let mut conscious: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Conscious)
        .collect();
    conscious.sort_by(|a, b| b.score.total_cmp(&a.score));

    let mut subconscious: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Subconscious)
        .collect();
    subconscious.sort_by(|a, b| b.score.total_cmp(&a.score));

    let mut novel: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Novel)
        .collect();
    novel.sort_by(|a, b| b.score.total_cmp(&a.score));

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
        let ep_name = get_episode_name(system, candidate.episode_ref);
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
    remaining.sort_by(|a, b| b.score.total_cmp(&a.score));

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
    total_candidates: usize,
    total_tokens_if_fetched: usize,
}

impl IndexResult {
    /// Number of candidate neighborhoods before filtering and deduplication.
    #[must_use]
    pub fn total_candidates(&self) -> usize {
        self.total_candidates
    }

    /// Estimated LLM tokens if all entries were fetched.
    #[must_use]
    pub fn total_tokens_if_fetched(&self) -> usize {
        self.total_tokens_if_fetched
    }
}

/// Compose a compact index of the best-matching neighborhoods without full content.
/// Same scoring pipeline as `compose_context_budgeted` but returns only metadata.
pub fn compose_index(
    system: &mut DAESystem,
    surface: &SurfaceResult,
    query_result: &QueryResult,
    session_recalled: Option<&HashMap<Uuid, u32>>,
) -> IndexResult {
    let candidates = rank_candidates(system, query_result, &query_result.interference, surface);
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
            // Decision/Preference types get softer decay (0.5x rate)
            if let Some(recalled) = session_recalled
                && let Some(&count) = recalled.get(&c.neighborhood_id)
            {
                let decay_rate = match c.neighborhood_type {
                    NeighborhoodType::Decision | NeighborhoodType::Preference => 0.5,
                    _ => 1.0,
                };
                score *= 1.0 / (1.0 + f64::from(count) * decay_rate);
            }
            (c, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.total_cmp(&a.1));

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
        total_candidates,
        total_tokens_if_fetched,
    }
}

/// Retrieve full content for specific neighborhood IDs.
/// Phase 2 of two-phase retrieval: after reviewing the index, fetch
/// only the neighborhoods you actually need.
pub fn retrieve_by_ids(system: &mut DAESystem, ids: &[Uuid]) -> Vec<IncludedFragment> {
    let mut fragments = Vec::new();

    for &id in ids {
        let Some(n_ref) = system.get_neighborhood_ref(id) else {
            continue;
        };

        let episode = system.resolve_episode(n_ref.episode_ref);
        let nbhd = &episode.neighborhoods[n_ref.neighborhood_idx];
        let (episode_name, category) = if n_ref.is_conscious() {
            (
                "Previously marked salient".to_string(),
                RecallCategory::Conscious,
            )
        } else {
            (episode.name.clone(), RecallCategory::Subconscious)
        };

        let text = if nbhd.source_text.is_empty() {
            nbhd.occurrences
                .iter()
                .map(|o| o.word.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            nbhd.source_text.clone()
        };

        fragments.push(IncludedFragment {
            neighborhood_id: id,
            episode_name,
            category,
            score: 0.0, // Not scored in direct retrieval
            tokens: token_count(&text),
            text,
            neighborhood_type: nbhd.neighborhood_type,
        });
    }

    fragments
}

// Scoring internals, salient extraction, and recency computation are in
// their own modules: crate::scoring, crate::salient, crate::recency.

#[cfg(test)]
#[path = "compose_tests.rs"]
mod tests;
