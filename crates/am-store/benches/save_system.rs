//! Criterion benchmarks for `BrainStore::save_system` at three scale points.
//!
//! Run with: `cargo bench -p am-store`
//!
//! Benchmarks:
//! - `save_system/100_episodes` (~5k occurrences)
//! - `save_system/1000_episodes` (~50k occurrences)
//! - `save_system/10000_episodes` (~500k occurrences)

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use rand::SeedableRng;
use rand::rngs::SmallRng;

use am_core::{DAESystem, Episode, Neighborhood};
use am_store::BrainStore;

/// Build a synthetic DAESystem with `n_episodes` episodes, each containing
/// ~5 neighborhoods of ~10 occurrences (~50 occurrences per episode).
fn build_system(n_episodes: usize) -> DAESystem {
    let mut rng = SmallRng::seed_from_u64(42);
    let mut system = DAESystem::new("bench");

    // Pool of words to draw from (simulating realistic token overlap)
    let words: Vec<String> = (0..200).map(|i| format!("word{i}")).collect();

    for ep_idx in 0..n_episodes {
        let mut ep = Episode::new(&format!("episode-{ep_idx}"));

        // 5 neighborhoods per episode, ~10 tokens each
        for n_idx in 0..5 {
            let base = ((ep_idx * 5 + n_idx) * 3) % words.len();
            let tokens: Vec<String> = (0..10)
                .map(|i| words[(base + i) % words.len()].clone())
                .collect();
            let text = tokens.join(" ");
            let nbhd = Neighborhood::from_tokens(&tokens, None, &text, &mut rng);
            ep.add_neighborhood(nbhd);
        }

        system.add_episode(ep);
    }

    // Add some conscious content too
    system.add_to_conscious("benchmark conscious insight one", &mut rng);
    system.add_to_conscious("benchmark conscious insight two", &mut rng);

    system
}

fn bench_save_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("save_system");

    for &n_episodes in &[100, 1_000, 10_000] {
        let system = build_system(n_episodes);
        let total_occ = system.n();

        group.bench_with_input(
            BenchmarkId::new("episodes", format!("{n_episodes} ({total_occ} occ)")),
            &system,
            |b, sys| {
                b.iter_with_setup(
                    || BrainStore::open_in_memory().expect("in-memory store"),
                    |store| {
                        store.save_system(sys).expect("save_system");
                    },
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_save_system);
criterion_main!(benches);
