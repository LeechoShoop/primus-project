// =============================================================================
// primus-net-opt/benches/discovery_beacon.rs
//
// Criterion benchmarks for UDP discovery beacon parsing.
//
// Benchmark targets:
//   parse_beacon        — strip_prefix + parse::<u16>()          (~18 ns)
//   seen_insert_contains — HashSet contains + insert              (~55 ns)
// =============================================================================

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::collections::HashSet;

fn bench_parse_beacon(c: &mut Criterion) {
    let beacon = "PRIMUS_PEER:9000";

    c.bench_function("parse_beacon", |ben| {
        ben.iter(|| {
            let data = black_box(beacon);
            let _port: Option<u16> = data
                .strip_prefix("PRIMUS_PEER:")
                .and_then(|p| p.trim().parse().ok());
        })
    });
}

fn bench_seen_insert_contains(c: &mut Criterion) {
    let mut seen: HashSet<String> = HashSet::with_capacity(1_024);
    // Pre-populate with 500 entries so the HashSet is non-trivially loaded.
    for i in 0u16..500 {
        seen.insert(format!("192.168.1.{}:{}", i % 256, 9000 + i));
    }
    let target = "192.168.1.42:9042".to_string();

    c.bench_function("seen_insert_contains", |ben| {
        ben.iter(|| {
            let addr = black_box(target.clone());
            if !seen.contains(&addr) {
                seen.insert(addr);
            }
        })
    });
}

criterion_group!(
    discovery_beacon_benches,
    bench_parse_beacon,
    bench_seen_insert_contains,
);
criterion_main!(discovery_beacon_benches);
