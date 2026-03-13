//! Criterion benchmarks for the drift paths in `query.rs`.
//!
//! Run with: `cargo bench -p am-core`
//!
//! Benchmarks:
//! - `pairwise_drift` at mobile sizes: 10, 50, 100, 199
//! - `centroid_drift` at mobile sizes: 200, 500, 1000
//! - `process_query` end-to-end pipeline

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;

use am_core::{DAESystem, Episode, Neighborhood, QueryEngine};

/// Build a system with `n` activated occurrences spread across episodes.
/// Each episode has one neighborhood with ~10 occurrences.
fn build_system(n: usize, rng: &mut SmallRng) -> (DAESystem, Vec<am_core::OccurrenceRef>) {
    let mut system = DAESystem::new("bench");

    let words: Vec<String> = (0..20).map(|i| format!("word{i}")).collect();
    let episodes_needed = (n / 10).max(1);

    for ep_idx in 0..episodes_needed {
        let occs_in_this = if ep_idx == episodes_needed - 1 {
            n - ep_idx * 10
        } else {
            10
        };
        let tokens: Vec<String> = (0..occs_in_this)
            .map(|i| words[i % words.len()].clone())
            .collect();
        let nbhd = Neighborhood::from_tokens(&tokens, None, "bench text", rng);
        let mut episode = Episode::new("bench");
        episode.add_neighborhood(nbhd);
        system.add_episode(episode);
    }

    system.rebuild_indexes();

    // Activate a shared word to get OccurrenceRefs
    let activation = system.activate_word("word0");
    let mut all_refs = activation.subconscious;
    all_refs.extend(activation.conscious);

    // If we need more refs, activate additional words
    let mut word_idx = 1;
    while all_refs.len() < n && word_idx < words.len() {
        let act = system.activate_word(&words[word_idx]);
        all_refs.extend(act.subconscious);
        all_refs.extend(act.conscious);
        word_idx += 1;
    }

    all_refs.truncate(n);
    (system, all_refs)
}

fn bench_drift_and_consolidate(c: &mut Criterion) {
    let mut group = c.benchmark_group("drift_and_consolidate");

    // Pairwise path: < 200 mobile occurrences
    for size in [10, 50, 100, 199] {
        group.bench_with_input(BenchmarkId::new("pairwise", size), &size, |b, &size| {
            let mut rng = SmallRng::seed_from_u64(42);
            let (mut system, refs) = build_system(size, &mut rng);
            b.iter(|| {
                QueryEngine::drift_and_consolidate(&mut system, &refs);
            });
        });
    }

    // Centroid path: >= 200 mobile occurrences
    for size in [200, 500, 1000] {
        group.bench_with_input(BenchmarkId::new("centroid", size), &size, |b, &size| {
            let mut rng = SmallRng::seed_from_u64(42);
            let (mut system, refs) = build_system(size, &mut rng);
            b.iter(|| {
                QueryEngine::drift_and_consolidate(&mut system, &refs);
            });
        });
    }

    group.finish();
}

fn bench_process_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("process_query");

    for episode_count in [10, 100, 500] {
        group.bench_with_input(
            BenchmarkId::new("episodes", episode_count),
            &episode_count,
            |b, &count| {
                let mut rng = SmallRng::seed_from_u64(42);
                let mut system = DAESystem::new("bench");

                // Build system with specified episode count
                let words: Vec<String> = (0..30).map(|i| format!("word{i}")).collect();
                for _ in 0..count {
                    let n_tokens = rng.random_range(5..15);
                    let tokens: Vec<String> = (0..n_tokens)
                        .map(|_| words[rng.random_range(0..words.len())].clone())
                        .collect();
                    let text = tokens.join(" ");
                    let nbhd = Neighborhood::from_tokens(&tokens, None, &text, &mut rng);
                    let mut episode = Episode::new("bench");
                    episode.add_neighborhood(nbhd);
                    system.add_episode(episode);
                }

                // Add some conscious memories
                system.add_to_conscious("word0 word1 word2 important concept", &mut rng);

                b.iter(|| {
                    QueryEngine::process_query(&mut system, "word0 word1 word5");
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_drift_and_consolidate, bench_process_query);
criterion_main!(benches);
