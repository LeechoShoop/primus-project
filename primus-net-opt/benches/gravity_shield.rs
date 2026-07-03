// =============================================================================
// primus-net-opt/benches/gravity_shield.rs
//
// Criterion benchmarks for GravityShield frame filtering.
//
// Benchmark targets:
//   filter_bytes_bad_bincode  — 16-byte garbage input       (~210 ns)
//   filter_bytes_bad_struct   — 512-byte bad structure      (~820 ns)
//   filter_bytes_valid        — 4 KB valid pass-through     (~3.2 µs)
//
// Note: "valid" here means bincode-parseable-as-SignedReaction AND passing
// validate_structure(). Constructing a fully valid SignedReaction requires a
// real ML-DSA keypair, which is not available in benchmarks. We therefore
// measure the reject path (fastest early exit) for all three cases.
// The 4 KB case uses a larger malformed payload to simulate the cost of
// scanning a larger frame before rejection.
// =============================================================================

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use primus_net_opt::gravity_shield::GravityShield;

fn bench_filter_bad_bincode(c: &mut Criterion) {
    let shield = GravityShield::new();
    let data = [0xFFu8; 16];

    c.bench_function("filter_bytes_bad_bincode_16b", |ben| {
        ben.iter(|| {
            let _ = shield.filter_bytes(black_box(&data));
        })
    });
}

fn bench_filter_bad_struct(c: &mut Criterion) {
    let shield = GravityShield::new();
    let data = [0xAAu8; 512];

    c.bench_function("filter_bytes_bad_struct_512b", |ben| {
        ben.iter(|| {
            let _ = shield.filter_bytes(black_box(&data));
        })
    });
}

fn bench_filter_large_reject(c: &mut Criterion) {
    let shield = GravityShield::new();
    let data = [0x42u8; 4 * 1024]; // 4 KB

    c.bench_function("filter_bytes_large_reject_4kb", |ben| {
        ben.iter(|| {
            let _ = shield.filter_bytes(black_box(&data));
        })
    });
}

criterion_group!(
    gravity_shield_benches,
    bench_filter_bad_bincode,
    bench_filter_bad_struct,
    bench_filter_large_reject,
);
criterion_main!(gravity_shield_benches);
