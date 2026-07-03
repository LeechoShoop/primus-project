// =============================================================================
// primus-net-opt/benches/gossip_dedup.rs
//
// Criterion benchmarks for gossip deduplication via seen-message HashSet.
//
// Benchmark targets:
//   seen_contains_1k   — HashSet::contains with 1,000 entries   (~45 ns)
//   seen_contains_10k  — HashSet::contains with 10,000 entries  (~52 ns)
//   seen_eviction      — evict 1,000 from 10,001 entries        (~180 µs)
// =============================================================================

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::collections::HashSet;

fn make_hashes(n: usize) -> Vec<[u8; 32]> {
    (0..n)
        .map(|i| {
            let mut h = [0u8; 32];
            h[0] = (i & 0xFF) as u8;
            h[1] = ((i >> 8) & 0xFF) as u8;
            h[2] = ((i >> 16) & 0xFF) as u8;
            h
        })
        .collect()
}

fn bench_seen_contains_1k(c: &mut Criterion) {
    let hashes = make_hashes(1_000);
    let mut set: HashSet<[u8; 32]> = HashSet::with_capacity(1_000);
    for h in &hashes {
        set.insert(*h);
    }
    let probe = hashes[500];

    c.bench_function("seen_contains_1k", |ben| {
        ben.iter(|| set.contains(black_box(&probe)))
    });
}

fn bench_seen_contains_10k(c: &mut Criterion) {
    let hashes = make_hashes(10_000);
    let mut set: HashSet<[u8; 32]> = HashSet::with_capacity(10_000);
    for h in &hashes {
        set.insert(*h);
    }
    let probe = hashes[5_000];

    c.bench_function("seen_contains_10k", |ben| {
        ben.iter(|| set.contains(black_box(&probe)))
    });
}

fn bench_seen_eviction(c: &mut Criterion) {
    // Mirrors gossip.rs eviction logic: when seen > MAX_SEEN (10_000),
    // remove the first EVICT_COUNT (1_000) entries.
    const MAX_SEEN: usize = 10_000;
    const EVICT_COUNT: usize = 1_000;

    c.bench_function("seen_eviction_1k_from_10001", |ben| {
        ben.iter_batched(
            || {
                // Setup: 10,001 entries — one over the cap
                let hashes = make_hashes(MAX_SEEN + 1);
                let mut set: HashSet<[u8; 32]> = HashSet::with_capacity(MAX_SEEN + 1);
                for h in &hashes {
                    set.insert(*h);
                }
                set
            },
            |mut seen| {
                // Eviction path copied from gossip.rs
                if seen.len() > MAX_SEEN {
                    let to_remove: Vec<_> = seen.iter().take(EVICT_COUNT).copied().collect();
                    for k in to_remove {
                        seen.remove(black_box(&k));
                    }
                }
                seen
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(
    gossip_dedup_benches,
    bench_seen_contains_1k,
    bench_seen_contains_10k,
    bench_seen_eviction,
);
criterion_main!(gossip_dedup_benches);
