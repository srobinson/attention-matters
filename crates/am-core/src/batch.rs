//! Batch query: amortize IDF computation across multiple parallel queries.
//!
//! When Nancy dispatches work to multiple workers, each worker needs context
//! from the same manifold. Activating N queries independently means N full
//! index rebuilds and N separate IDF computations. Batch query activates the
//! union of all query tokens once, drifts once, computes interference once,
//! then partitions results per query.
//!
//! The IDF weights are a global property of the manifold — they don't change
//! between queries in the same batch. This is where the amortization comes from.

use std::collections::{HashMap, HashSet};

use crate::compose::{BudgetConfig, BudgetedContextResult, compose_context_budgeted};
use crate::query::{QueryEngine, QueryResult};
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
    /// Optional project ID for affinity scoring.
    pub project_id: Option<String>,
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

/// Batch query engine that amortizes activation and IDF across multiple queries.
pub struct BatchQueryEngine;

impl BatchQueryEngine {
    /// Process multiple queries in a single batch.
    ///
    /// Strategy:
    /// 1. Collect union of all query tokens (deduplicated).
    /// 2. Activate the union once — single index rebuild, single activation pass.
    /// 3. Drift the union once — single O(n²) or O(n) pass.
    /// 4. Compute interference once for the full activated set.
    /// 5. For each individual query, build a per-query activation subset,
    ///    compute surface, and compose context with its own budget.
    ///
    /// The IDF weights don't change between step 2 and step 5 because
    /// activation doesn't modify the neighborhood index — it only bumps
    /// occurrence counters. So `get_word_weight()` returns the same value
    /// for all queries in the batch.
    pub fn batch_query(
        system: &mut DAESystem,
        requests: &[BatchQueryRequest],
    ) -> Vec<BatchQueryResult> {
        if requests.is_empty() {
            return Vec::new();
        }

        // Step 1: Union of all query tokens
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

        // Step 2: Activate the union once
        let mut all_subconscious: Vec<OccurrenceRef> = Vec::new();
        let mut all_conscious: Vec<OccurrenceRef> = Vec::new();

        // Build word→refs map for per-query partitioning
        let mut word_to_sub_refs: HashMap<String, Vec<OccurrenceRef>> = HashMap::new();
        let mut word_to_con_refs: HashMap<String, Vec<OccurrenceRef>> = HashMap::new();

        for token in &all_tokens {
            let activation = system.activate_word(token);
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
        QueryEngine::drift_and_consolidate(system, &all_refs);

        // Step 4: Compute interference once for the full set
        let (_, _word_groups) =
            QueryEngine::compute_interference(system, &all_subconscious, &all_conscious);
        QueryEngine::apply_kuramoto_coupling(system, &_word_groups);

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
                req.project_id.as_deref(),
                None,
            );

            results.push(BatchQueryResult {
                query: req.query.clone(),
                context,
                activated_count,
            });
        }

        results
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
        words.iter().map(|s| s.to_string()).collect()
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
                project_id: None,
            },
            BatchQueryRequest {
                query: "rust compiler".to_string(),
                max_tokens: Some(4096),
                project_id: None,
            },
        ];

        let results = BatchQueryEngine::batch_query(&mut sys, &requests);

        assert_eq!(results.len(), 2, "should return one result per query");
        assert_eq!(results[0].query, "quantum physics");
        assert_eq!(results[1].query, "rust compiler");
    }

    #[test]
    fn test_batch_activates_correct_subsets() {
        let mut sys = make_batch_system();

        let requests = vec![
            BatchQueryRequest {
                query: "quantum physics".to_string(),
                max_tokens: Some(4096),
                project_id: None,
            },
            BatchQueryRequest {
                query: "neural network".to_string(),
                max_tokens: Some(4096),
                project_id: None,
            },
        ];

        let results = BatchQueryEngine::batch_query(&mut sys, &requests);

        // Both queries should have activated occurrences
        assert!(
            results[0].activated_count > 0,
            "quantum physics should activate occurrences"
        );
        assert!(
            results[1].activated_count > 0,
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
                project_id: None,
            },
            BatchQueryRequest {
                query: "quantum physics neural".to_string(),
                max_tokens: Some(100000), // Huge budget
                project_id: None,
            },
        ];

        let results = BatchQueryEngine::batch_query(&mut sys, &requests);

        assert!(
            results[0].context.tokens_used <= 50,
            "tight budget should be respected: {}",
            results[0].context.tokens_used
        );
        // Huge budget should include more or equal content
        assert!(
            results[1].context.included.len() >= results[0].context.included.len(),
            "larger budget should include >= fragments"
        );
    }

    #[test]
    fn test_batch_empty_requests() {
        let mut sys = make_batch_system();
        let results = BatchQueryEngine::batch_query(&mut sys, &[]);
        assert!(results.is_empty());
    }

    #[test]
    fn test_batch_single_query_equivalent() {
        // A batch of 1 should produce similar results to a direct query
        let mut sys1 = make_batch_system();
        let mut sys2 = make_batch_system();

        // Batch query
        let batch_results = BatchQueryEngine::batch_query(
            &mut sys1,
            &[BatchQueryRequest {
                query: "quantum physics".to_string(),
                max_tokens: Some(4096),
                project_id: None,
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
            None,
        );

        // Same number of included fragments (the drift/interference may differ
        // slightly because batch activates union, but structure should match)
        assert_eq!(
            batch_results[0].context.included.len(),
            direct.included.len(),
            "batch of 1 should match direct query fragment count"
        );
    }

    #[test]
    fn test_batch_overlapping_queries_share_activation() {
        let mut sys = make_batch_system();

        // Two queries that share "quantum" — this word gets activated once
        let requests = vec![
            BatchQueryRequest {
                query: "quantum physics".to_string(),
                max_tokens: Some(4096),
                project_id: None,
            },
            BatchQueryRequest {
                query: "quantum computing".to_string(),
                max_tokens: Some(4096),
                project_id: None,
            },
        ];

        let results = BatchQueryEngine::batch_query(&mut sys, &requests);

        // Both should have results
        assert!(results[0].activated_count > 0);
        assert!(results[1].activated_count > 0);

        // "quantum" occurrences should appear in both result sets
        // (they share the word from the union activation)
        assert!(
            !results[0].context.context.is_empty(),
            "first query should have context"
        );
        assert!(
            !results[1].context.context.is_empty(),
            "second query should have context"
        );
    }

    #[test]
    fn test_batch_with_project_affinity() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Episode with project_id
        let mut ep = Episode::new("project-specific");
        ep.project_id = "my-project".to_string();
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta", "gamma"]),
            None,
            "alpha beta gamma",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Episode without project_id
        let mut ep2 = Episode::new("generic");
        ep2.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "delta", "epsilon"]),
            None,
            "alpha delta epsilon",
            &mut rng,
        ));
        sys.add_episode(ep2);

        sys.add_to_conscious("alpha research", &mut rng);

        let requests = vec![
            BatchQueryRequest {
                query: "alpha".to_string(),
                max_tokens: Some(4096),
                project_id: Some("my-project".to_string()),
            },
            BatchQueryRequest {
                query: "alpha".to_string(),
                max_tokens: Some(4096),
                project_id: None,
            },
        ];

        let results = BatchQueryEngine::batch_query(&mut sys, &requests);
        assert_eq!(results.len(), 2);
        // Both should return context — project affinity affects scoring, not filtering
    }
}
