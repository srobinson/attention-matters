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

/// Result of context composition.
pub struct ContextResult {
    pub context: String,
    pub metrics: ContextMetrics,
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
}

/// Score and categorize all activated neighborhoods into ranked candidates.
/// Conscious neighborhoods scored by IDF-weighted activation.
/// Subconscious neighborhoods scored by IDF-weighted activation.
/// Novel candidates: subconscious with activated_count <= 2, no words in common
/// with conscious, scored by max_word_weight * max_plasticity / activated_count.
fn rank_candidates(
    system: &mut DAESystem,
    query_result: &QueryResult,
) -> Vec<RankedCandidate> {
    let conscious_words: HashSet<String> = query_result
        .activation
        .conscious
        .iter()
        .map(|r| system.get_occurrence(*r).word.to_lowercase())
        .collect();

    let con_scored = score_neighborhoods(system, &query_result.activation.conscious, true);
    let sub_scored = score_neighborhoods(system, &query_result.activation.subconscious, false);

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
        });
    }

    candidates
}

/// Format a single entry for the composed context string.
fn format_entry(category: RecallCategory, index: usize, ep_name: &str, text: &str) -> Vec<String> {
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
    lines.push(format!("\"{}\"", text));
    lines
}

const ENTRY_HEADER_OVERHEAD_TOKENS: usize = 20;

/// Compose human-readable context from surface and activation results.
///
/// `_surface` and `_interference` are part of the pipeline API and reserved
/// for future use (e.g. vivid filtering, interference-weighted scoring).
pub fn compose_context(
    system: &mut DAESystem,
    _surface: &SurfaceResult,
    query_result: &QueryResult,
    _interference: &[InterferenceResult],
) -> ContextResult {
    let candidates = rank_candidates(system, query_result);

    let mut selected_ids: HashSet<Uuid> = HashSet::new();
    let mut parts: Vec<String> = Vec::new();
    let mut metrics = ContextMetrics {
        conscious: 0,
        subconscious: 0,
        novel: 0,
    };

    // Conscious: top 1
    let mut con: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| c.category == RecallCategory::Conscious)
        .collect();
    con.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    if let Some(best) = con.first() {
        selected_ids.insert(best.neighborhood_id);
        let entry = format_entry(RecallCategory::Conscious, 0, "", &best.text);
        parts.extend(entry);
        metrics.conscious = 1;
    }

    // Subconscious: top 2 (excluding already selected)
    let mut sub: Vec<&RankedCandidate> = candidates
        .iter()
        .filter(|c| {
            c.category == RecallCategory::Subconscious
                && !selected_ids.contains(&c.neighborhood_id)
        })
        .collect();
    sub.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    for (i, entry) in sub.iter().take(2).enumerate() {
        selected_ids.insert(entry.neighborhood_id);
        let ep_name = get_episode_name(system, entry.episode_idx);
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let lines = format_entry(RecallCategory::Subconscious, i + 1, &ep_name, &entry.text);
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
        let ep_name = get_episode_name(system, best.episode_idx);
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let lines = format_entry(RecallCategory::Novel, 0, &ep_name, &best.text);
        parts.extend(lines);
        metrics.novel = 1;
    }

    ContextResult {
        context: parts.join("\n"),
        metrics,
    }
}

/// Budget-constrained context composition.
///
/// Fills guaranteed minimums first (highest-scored per category), then greedily
/// fills remaining budget by score across all categories.
///
/// `_surface` and `_interference` are part of the pipeline API and reserved
/// for future use (e.g. vivid filtering, interference-weighted scoring).
pub fn compose_context_budgeted(
    system: &mut DAESystem,
    _surface: &SurfaceResult,
    query_result: &QueryResult,
    _interference: &[InterferenceResult],
    budget: &BudgetConfig,
) -> BudgetedContextResult {
    let candidates = rank_candidates(system, query_result);

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
    let unique_candidate_ids: HashSet<Uuid> = candidates.iter().map(|c| c.neighborhood_id).collect();
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
        });
        true
    };

    // Phase 1: Fill guaranteed minimums
    let mut con_filled = 0usize;
    for c in &conscious {
        if con_filled >= budget.min_conscious {
            break;
        }
        if try_add(c, &mut selected_ids, &mut included, &mut tokens_used, budget.max_tokens, system)
        {
            con_filled += 1;
        }
    }

    let mut sub_filled = 0usize;
    for c in &subconscious {
        if sub_filled >= budget.min_subconscious {
            break;
        }
        if try_add(c, &mut selected_ids, &mut included, &mut tokens_used, budget.max_tokens, system)
        {
            sub_filled += 1;
        }
    }

    let mut novel_filled = 0usize;
    for c in &novel {
        if novel_filled >= budget.min_novel {
            break;
        }
        if try_add(c, &mut selected_ids, &mut included, &mut tokens_used, budget.max_tokens, system)
        {
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
        try_add(c, &mut selected_ids, &mut included, &mut tokens_used, budget.max_tokens, system);
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
        let lines = format_entry(RecallCategory::Conscious, 0, "", &entry.text);
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
        let lines =
            format_entry(RecallCategory::Subconscious, i + 1, &entry.episode_name, &entry.text);
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
        let lines = format_entry(RecallCategory::Novel, 0, &entry.episode_name, &entry.text);
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

struct ScoredNeighborhood {
    neighborhood_id: Uuid,
    episode_idx: usize, // usize::MAX for conscious
    score: f64,
    activated_count: usize,
    words: HashSet<String>,
    max_word_weight: f64,
    max_plasticity: f64,
}

fn score_neighborhoods(
    system: &mut DAESystem,
    refs: &[OccurrenceRef],
    _is_conscious: bool,
) -> HashMap<Uuid, ScoredNeighborhood> {
    let mut scored: HashMap<Uuid, ScoredNeighborhood> = HashMap::new();

    // Pre-collect data to avoid borrow conflicts
    let data: Vec<(Uuid, usize, String, u32, f64)> = refs
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
            )
        })
        .collect();

    for (nbhd_id, ep_idx, word, activation_count, plasticity) in &data {
        let weight = system.get_word_weight(word);

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
            });

        entry.score += weight * *activation_count as f64;
        entry.words.insert(word.clone());
        entry.activated_count += 1;
        if weight > entry.max_word_weight {
            entry.max_word_weight = weight;
        }
        if *plasticity > entry.max_plasticity {
            entry.max_plasticity = *plasticity;
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

/// Extract salient-tagged content and add to conscious episode.
pub fn extract_salient(system: &mut DAESystem, text: &str, rng: &mut impl Rng) -> u32 {
    let mut count = 0u32;
    for cap in SALIENT_RE.captures_iter(text) {
        if let Some(content) = cap.get(1) {
            system.add_to_conscious(content.as_str(), rng);
            count += 1;
        }
    }
    count
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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference);

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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference);

        // No conscious recall since no conscious content matches
        assert!(!ctx.context.contains("CONSCIOUS RECALL:"));
    }

    #[test]
    fn test_metrics() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference);

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
        let ctx1 = compose_context(&mut sys1, &surface1, &result1, &result1.interference);

        let mut sys2 = make_full_system();
        let result2 = QueryEngine::process_query(&mut sys2, "quantum");
        let surface2 = compute_surface(&sys2, &result2);
        let ctx2 = compose_context(&mut sys2, &surface2, &result2, &result2.interference);

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
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference);

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
        let ctx2 = compose_context(&mut sys2, &surface2, &result2, &result2.interference);
        assert_eq!(ctx.context, ctx2.context);
    }
}
