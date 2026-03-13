//! Batch query: amortize IDF computation across multiple parallel queries.
//!
//! When Nancy dispatches work to multiple workers, each worker needs context
//! from the same manifold. Activating N queries independently means N full
//! index rebuilds and N separate IDF computations. Batch query activates the
//! union of all query tokens once, drifts once, computes interference once,
//! then partitions results per query.
//!
//! The IDF weights are a global property of the manifold - they don't change
//! between queries in the same batch. This is where the amortization comes from.

use std::collections::{HashMap, HashSet};

use crate::compose::{BudgetConfig, BudgetedContextResult, compose_context_budgeted};
use crate::query::{QueryEngine, QueryManifest, QueryResult};
use crate::surface::compute_surface;
use crate::system::{DAESystem, OccurrenceRef};
use crate::tokenizer::tokenize;

/// A single query in a batch.
#[derive(Debug, Clone)]
pub struct BatchQueryRequest {
    /// The query text.
    pub query: String,
    /// Optional token budget for this query's context. If None, uses default.
    pub max_tokens: Option<usize>,
}

/// Result for a single query within a batch.
pub struct BatchQueryResult {
    /// The original query text.
    pub query: String,
    /// The composed context for this query.
    pub context: BudgetedContextResult,
    /// Number of activated occurrences for this query.
    pub activated_count: usize,
}

/// Combined result of a batch query: per-query results plus a manifest of
/// all mutations applied to the system during the batch operation.
pub struct BatchQueryOutput {
    /// Per-query results.
    pub results: Vec<BatchQueryResult>,
    /// Aggregate manifest of all occurrence mutations across the batch.
    pub manifest: QueryManifest,
}

/// Batch query engine that amortizes activation and IDF across multiple queries.
///
/// # Examples
///
/// ```
/// use am_core::{DAESystem, BatchQueryEngine, BatchQueryRequest, ingest_text};
/// use rand::SeedableRng;
/// use rand::rngs::SmallRng;
///
/// let mut system = DAESystem::new("test");
/// let mut rng = SmallRng::seed_from_u64(42);
/// let ep = ingest_text("Geometric algebra for computer graphics", None, &mut rng);
/// system.add_episode(ep);
///
/// let requests = vec![
///     BatchQueryRequest { query: "algebra".into(), max_tokens: None },
///     BatchQueryRequest { query: "graphics".into(), max_tokens: Some(500) },
/// ];
///
/// let output = BatchQueryEngine::batch_query(&mut system, &requests);
/// assert_eq!(output.results.len(), 2);
/// ```
pub struct BatchQueryEngine;

impl BatchQueryEngine {
    /// Process multiple queries in a single batch.
    ///
    /// Strategy:
    /// 1. Collect union of all query tokens (deduplicated).
    /// 2. Activate each token once per query that contains it. A token
    ///    appearing in N queries gets `activation_count += N`, matching
    ///    the behavior of N independent `process_query` calls. The index
    ///    rebuild happens once (amortized), but the activation counts
    ///    reflect true per-query demand.
    /// 3. Drift the union once - single O(n^2) or O(n) pass.
    /// 4. Compute interference once for the full activated set.
    /// 5. For each individual query, build a per-query activation subset,
    ///    compute surface, and compose context with its own budget.
    ///
    /// The IDF weights don't change between step 2 and step 5 because
    /// activation doesn't modify the neighborhood index - it only bumps
    /// occurrence counters. So `get_word_weight()` returns the same value
    /// for all queries in the batch.
    pub fn batch_query(system: &mut DAESystem, requests: &[BatchQueryRequest]) -> BatchQueryOutput {
        if requests.is_empty() {
            return BatchQueryOutput {
                results: Vec::new(),
                manifest: QueryManifest::default(),
            };
        }

        // Step 1: Union of all query tokens and per-query token sets
        let mut all_tokens: HashSet<String> = HashSet::new();
        let per_query_tokens: Vec<HashSet<String>> = requests
            .iter()
            .map(|req| {
                let tokens = tokenize(&req.query);
                let unique: HashSet<String> =
                    tokens.into_iter().map(|t| t.to_lowercase()).collect();
                all_tokens.extend(unique.iter().cloned());
                unique
            })
            .collect();

        // Count how many queries contain each token. A token shared by N
        // queries must be activated N times to match sequential semantics.
        let mut token_query_count: HashMap<String, usize> = HashMap::new();
        for query_set in &per_query_tokens {
            for token in query_set {
                *token_query_count.entry(token.clone()).or_insert(0) += 1;
            }
        }

        // Step 2: Activate each token, calling activate_word once for the
        // index lookup but bumping activation_count by (N-1) extra times
        // for tokens shared across N queries.
        let mut all_subconscious: Vec<OccurrenceRef> = Vec::new();
        let mut all_conscious: Vec<OccurrenceRef> = Vec::new();
        let mut activated_ids: Vec<uuid::Uuid> = Vec::new();

        // Build word->refs map for per-query partitioning
        let mut word_to_sub_refs: HashMap<String, Vec<OccurrenceRef>> = HashMap::new();
        let mut word_to_con_refs: HashMap<String, Vec<OccurrenceRef>> = HashMap::new();

        for token in &all_tokens {
            // First call: activates once (activation_count += 1) and
            // returns the occurrence refs we need for partitioning.
            let activation = system.activate_word(token);

            // Collect activated UUIDs for the manifest
            for r in activation.subconscious.iter().chain(&activation.conscious) {
                activated_ids.push(system.get_occurrence(*r).id);
            }

            // Additional activations for queries beyond the first.
            // Each extra call must also be tracked in activated_ids so that
            // persist_manifest issues the matching number of SQL increments.
            let extra = token_query_count.get(token).copied().unwrap_or(1) - 1;
            if extra > 0 {
                for r in activation.subconscious.iter().chain(&activation.conscious) {
                    let occ = system.get_occurrence_mut(*r);
                    let id = occ.id;
                    for _ in 0..extra {
                        occ.activate();
                        activated_ids.push(id);
                    }
                }
            }

            for r in &activation.subconscious {
                word_to_sub_refs.entry(token.clone()).or_default().push(*r);
            }
            for r in &activation.conscious {
                word_to_con_refs.entry(token.clone()).or_default().push(*r);
            }
            all_subconscious.extend(activation.subconscious);
            all_conscious.extend(activation.conscious);
        }

        // Step 3: Drift the union once
        let all_refs: Vec<OccurrenceRef> = all_subconscious
            .iter()
            .chain(all_conscious.iter())
            .copied()
            .collect();
        let mut drifted = QueryEngine::drift_and_consolidate(system, &all_refs);

        // Step 4: Compute interference once for the full set
        let (_, word_groups) =
            QueryEngine::compute_interference(system, &all_subconscious, &all_conscious);
        let kuramoto_drifted = QueryEngine::apply_kuramoto_coupling(system, &word_groups);
        drifted.extend(kuramoto_drifted);

        // Build aggregate manifest
        let manifest = QueryManifest {
            drifted,
            activated: activated_ids,
            demoted_activations: Vec::new(),
        };

        // Step 5: Per-query partitioning and context composition
        let mut results = Vec::with_capacity(requests.len());

        for (i, req) in requests.iter().enumerate() {
            let query_tokens = &per_query_tokens[i];

            // Build per-query activation by filtering the union results
            let sub_refs: Vec<OccurrenceRef> = query_tokens
                .iter()
                .flat_map(|t| word_to_sub_refs.get(t).cloned().unwrap_or_default())
                .collect();
            let con_refs: Vec<OccurrenceRef> = query_tokens
                .iter()
                .flat_map(|t| word_to_con_refs.get(t).cloned().unwrap_or_default())
                .collect();

            let activated_count = sub_refs.len() + con_refs.len();

            // Build a per-query QueryResult for compose_context
            let (interference, word_groups) =
                QueryEngine::compute_interference(system, &sub_refs, &con_refs);

            let query_result = QueryResult {
                activation: crate::system::ActivationResult {
                    subconscious: sub_refs,
                    conscious: con_refs,
                },
                interference,
                word_groups,
                query_token_count: query_tokens.len(),
                manifest: QueryManifest::default(),
            };

            let surface = compute_surface(system, &query_result);

            let budget = BudgetConfig {
                max_tokens: req.max_tokens.unwrap_or(4096),
                min_conscious: 1,
                min_subconscious: 1,
                min_novel: 0,
            };

            let context = compose_context_budgeted(
                system,
                &surface,
                &query_result,
                &query_result.interference,
                &budget,
                None,
            );

            results.push(BatchQueryResult {
                query: req.query.clone(),
                context,
                activated_count,
            });
        }

        BatchQueryOutput { results, manifest }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::Episode;
    use crate::neighborhood::Neighborhood;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(std::string::ToString::to_string).collect()
    }

    fn make_batch_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Episode 1: science topics
        let mut ep1 = Episode::new("Science");
        ep1.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics", "particle", "wave"]),
            None,
            "quantum physics particle wave",
            &mut rng,
        ));
        ep1.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["neural", "network", "deep", "learning"]),
            None,
            "neural network deep learning",
            &mut rng,
        ));
        ep1.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["biology", "cell", "membrane", "protein"]),
            None,
            "biology cell membrane protein",
            &mut rng,
        ));
        sys.add_episode(ep1);

        // Episode 2: engineering topics
        let mut ep2 = Episode::new("Engineering");
        ep2.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["rust", "compiler", "borrow", "lifetime"]),
            None,
            "rust compiler borrow lifetime",
            &mut rng,
        ));
        ep2.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["kubernetes", "container", "pod", "deployment"]),
            None,
            "kubernetes container pod deployment",
            &mut rng,
        ));
        sys.add_episode(ep2);

        // Conscious
        sys.add_to_conscious("quantum computing neural architecture", &mut rng);

        sys
    }

    #[test]
    fn test_batch_returns_per_query_results() {
        let mut sys = make_batch_system();

        let requests = vec![
            BatchQueryRequest {
                query: "quantum physics".to_string(),
                max_tokens: Some(4096),
            },
            BatchQueryRequest {
                query: "rust compiler".to_string(),
                max_tokens: Some(4096),
            },
        ];

        let output = BatchQueryEngine::batch_query(&mut sys, &requests);

        assert_eq!(
            output.results.len(),
            2,
            "should return one result per query"
        );
        assert_eq!(output.results[0].query, "quantum physics");
        assert_eq!(output.results[1].query, "rust compiler");
    }

    #[test]
    fn test_batch_activates_correct_subsets() {
        let mut sys = make_batch_system();

        let requests = vec![
            BatchQueryRequest {
                query: "quantum physics".to_string(),
                max_tokens: Some(4096),
            },
            BatchQueryRequest {
                query: "neural network".to_string(),
                max_tokens: Some(4096),
            },
        ];

        let output = BatchQueryEngine::batch_query(&mut sys, &requests);

        // Both queries should have activated occurrences
        assert!(
            output.results[0].activated_count > 0,
            "quantum physics should activate occurrences"
        );
        assert!(
            output.results[1].activated_count > 0,
            "neural network should activate occurrences"
        );
    }

    #[test]
    fn test_batch_respects_per_query_budget() {
        let mut sys = make_batch_system();

        let requests = vec![
            BatchQueryRequest {
                query: "quantum physics neural".to_string(),
                max_tokens: Some(50), // Tight budget
            },
            BatchQueryRequest {
                query: "quantum physics neural".to_string(),
                max_tokens: Some(100_000), // Huge budget
            },
        ];

        let output = BatchQueryEngine::batch_query(&mut sys, &requests);

        assert!(
            output.results[0].context.tokens_used <= 50,
            "tight budget should be respected: {}",
            output.results[0].context.tokens_used
        );
        // Huge budget should include more or equal content
        assert!(
            output.results[1].context.included.len() >= output.results[0].context.included.len(),
            "larger budget should include >= fragments"
        );
    }

    #[test]
    fn test_batch_empty_requests() {
        let mut sys = make_batch_system();
        let output = BatchQueryEngine::batch_query(&mut sys, &[]);
        assert!(output.results.is_empty());
    }

    #[test]
    fn test_batch_single_query_equivalent() {
        // A batch of 1 should produce similar results to a direct query
        let mut sys1 = make_batch_system();
        let mut sys2 = make_batch_system();

        // Batch query
        let batch_output = BatchQueryEngine::batch_query(
            &mut sys1,
            &[BatchQueryRequest {
                query: "quantum physics".to_string(),
                max_tokens: Some(4096),
            }],
        );

        // Direct query
        let query_result = QueryEngine::process_query(&mut sys2, "quantum physics");
        let surface = compute_surface(&sys2, &query_result);
        let budget = BudgetConfig {
            max_tokens: 4096,
            min_conscious: 1,
            min_subconscious: 1,
            min_novel: 0,
        };
        let direct = compose_context_budgeted(
            &mut sys2,
            &surface,
            &query_result,
            &query_result.interference,
            &budget,
            None,
        );

        // Same number of included fragments (the drift/interference may differ
        // slightly because batch activates union, but structure should match)
        assert_eq!(
            batch_output.results[0].context.included.len(),
            direct.included.len(),
            "batch of 1 should match direct query fragment count"
        );
    }

    #[test]
    fn test_batch_overlapping_queries_share_activation() {
        let mut sys = make_batch_system();

        // Two queries that share "quantum" - activated twice (once per query)
        let requests = vec![
            BatchQueryRequest {
                query: "quantum physics".to_string(),
                max_tokens: Some(4096),
            },
            BatchQueryRequest {
                query: "quantum computing".to_string(),
                max_tokens: Some(4096),
            },
        ];

        let output = BatchQueryEngine::batch_query(&mut sys, &requests);

        // Both should have results
        assert!(output.results[0].activated_count > 0);
        assert!(output.results[1].activated_count > 0);

        // "quantum" occurrences should appear in both result sets
        // (they share the word from the union activation)
        assert!(
            !output.results[0].context.context.is_empty(),
            "first query should have context"
        );
        assert!(
            !output.results[1].context.context.is_empty(),
            "second query should have context"
        );
    }

    /// Batch activation counts must match sequential activation counts.
    ///
    /// If word W appears in N queries, running those N queries sequentially
    /// calls `activate_word(W)` N times, producing `activation_count` = N.
    /// Batch must produce the same count so that `drift_rate`, plasticity,
    /// and anchoring thresholds behave identically.
    #[test]
    fn test_batch_activation_matches_sequential_for_shared_tokens() {
        // Sequential: 3 separate queries each containing "quantum"
        let mut sys_seq = make_batch_system();
        let _ = QueryEngine::activate(&mut sys_seq, "quantum alpha");
        let _ = QueryEngine::activate(&mut sys_seq, "quantum beta");
        let _ = QueryEngine::activate(&mut sys_seq, "quantum gamma");

        let seq_counts: Vec<u32> = sys_seq
            .get_word_occurrences("quantum")
            .iter()
            .map(|r| sys_seq.get_occurrence(*r).activation_count)
            .collect();

        // Batch: 3 queries in one batch, all containing "quantum"
        let mut sys_batch = make_batch_system();
        let requests = vec![
            BatchQueryRequest {
                query: "quantum alpha".to_string(),
                max_tokens: Some(4096),
            },
            BatchQueryRequest {
                query: "quantum beta".to_string(),
                max_tokens: Some(4096),
            },
            BatchQueryRequest {
                query: "quantum gamma".to_string(),
                max_tokens: Some(4096),
            },
        ];
        let _results = BatchQueryEngine::batch_query(&mut sys_batch, &requests);

        let batch_counts: Vec<u32> = sys_batch
            .get_word_occurrences("quantum")
            .iter()
            .map(|r| sys_batch.get_occurrence(*r).activation_count)
            .collect();

        // Both paths should produce the same activation count for "quantum".
        // Conscious occurrences start at activation_count=1 (pre-activated
        // by add_to_conscious), so their final count is 1+3=4 rather than 3.
        assert_eq!(
            seq_counts.len(),
            batch_counts.len(),
            "same number of quantum occurrences"
        );
        for (i, (s, b)) in seq_counts.iter().zip(&batch_counts).enumerate() {
            assert_eq!(
                s, b,
                "occurrence {i}: sequential activation_count ({s}) must match batch ({b})"
            );
        }

        // Every occurrence should have been activated at least 3 times
        // (the number of queries containing "quantum").
        for (i, count) in batch_counts.iter().enumerate() {
            assert!(
                *count >= 3,
                "occurrence {i}: expected activation_count >= 3 for token in 3 queries, got {count}"
            );
        }
    }

    /// Tokens unique to a single query get `activation_count` = 1 in batch,
    /// matching sequential behavior.
    #[test]
    fn test_batch_activation_unique_tokens_count_once() {
        let mut sys = make_batch_system();
        let requests = vec![
            BatchQueryRequest {
                query: "quantum physics".to_string(),
                max_tokens: Some(4096),
            },
            BatchQueryRequest {
                query: "rust compiler".to_string(),
                max_tokens: Some(4096),
            },
        ];
        let _results = BatchQueryEngine::batch_query(&mut sys, &requests);

        // "physics" only appears in query 1, "compiler" only in query 2.
        // Each should have activation_count == 1.
        let physics_counts: Vec<u32> = sys
            .get_word_occurrences("physics")
            .iter()
            .map(|r| sys.get_occurrence(*r).activation_count)
            .collect();
        let compiler_counts: Vec<u32> = sys
            .get_word_occurrences("compiler")
            .iter()
            .map(|r| sys.get_occurrence(*r).activation_count)
            .collect();

        for (i, count) in physics_counts.iter().enumerate() {
            assert_eq!(*count, 1, "physics occurrence {i}: expected 1, got {count}");
        }
        for (i, count) in compiler_counts.iter().enumerate() {
            assert_eq!(
                *count, 1,
                "compiler occurrence {i}: expected 1, got {count}"
            );
        }
    }
}
