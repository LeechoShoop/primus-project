use criterion::{black_box, criterion_group, criterion_main, Criterion};
use primus_vm::physics::*;
use primus_vm::wasm::limits::MAX_SAFE_COMPLEXITY;
use primus_types::atom::{Atom, Element, QuantumState};

fn bench_galactic_drift(c: &mut Criterion) {
    let mut group = c.benchmark_group("galactic_drift");
    for index in [0, 1, 255, 256, u64::MAX].iter() {
        group.bench_with_input(format!("index_{}", index), index, |b, &i| {
            b.iter(|| get_galactic_drift(black_box(i)))
        });
    }
    group.finish();
}

fn bench_orbital_resonance(c: &mut Criterion) {
    let mut group = c.benchmark_group("orbital_resonance");
    let drift = 0xAA;
    let matching_pk = vec![0xAA, 0xBB, 0xCC];
    let non_matching_pk = vec![0x11, 0x22, 0x33];

    group.bench_function("match", |b| {
        b.iter(|| calculate_orbital_resonance(black_box(&matching_pk), black_box(drift)))
    });
    group.bench_function("no_match", |b| {
        b.iter(|| calculate_orbital_resonance(black_box(&non_matching_pk), black_box(drift)))
    });
    group.finish();
}

fn bench_gravity_assist(c: &mut Criterion) {
    let mut group = c.benchmark_group("gravity_assist");
    let atom_id = vec![0xAA, 0x01];
    
    // Setup stars: pk[0] matches atom_id[0], mass > 45,000
    let mut stars = Vec::new();
    for i in 0..1000 {
        let pk = vec![0xAA, (i % 256) as u8];
        let atom = Atom {
            public_key: pk.clone(),
            element: Element::Hydrogen,
            neutron_count: 0,
            mass: 50_000,
            charge: 2.2,
            last_reaction_hash: [0; 32],
            last_active_index: 0,
            nonce: 0,
            quantum_state: QuantumState::Stable,
        };
        stars.push((pk, atom));
    }

    let stars_refs: Vec<(&Vec<u8>, &Atom)> = stars.iter().map(|(pk, a)| (pk, a)).collect();

    group.bench_function("1_star", |b| {
        b.iter(|| {
            calculate_gravity_assist_from_iter(
                black_box(stars_refs[0..1].iter().map(|(pk, a)| (*pk, *a))),
                black_box(&atom_id),
            )
        })
    });

    group.bench_function("10_stars", |b| {
        b.iter(|| {
            calculate_gravity_assist_from_iter(
                black_box(stars_refs[0..10].iter().map(|(pk, a)| (*pk, *a))),
                black_box(&atom_id),
            )
        })
    });

    group.bench_function("1000_stars", |b| {
        b.iter(|| {
            calculate_gravity_assist_from_iter(
                black_box(stars_refs.iter().map(|(pk, a)| (*pk, *a))),
                black_box(&atom_id),
            )
        })
    });
    group.finish();
}

fn bench_spacetime_curvature(c: &mut Criterion) {
    let mut group = c.benchmark_group("spacetime_curvature");
    let hash = [0xAA; 32];
    let temp = 100.0;
    group.bench_function("varied", |b| {
        b.iter(|| get_spacetime_curvature(black_box(&hash), black_box(temp)))
    });
    group.finish();
}

fn bench_macro_shift(c: &mut Criterion) {
    let mut group = c.benchmark_group("macro_shift");
    group.bench_function("below_critical", |b| {
        b.iter(|| calculate_macro_shift(black_box(249.9)))
    });
    group.bench_function("above_critical", |b| {
        b.iter(|| calculate_macro_shift(black_box(500.0)))
    });
    group.finish();
}

fn bench_entropy_tax(c: &mut Criterion) {
    let mut group = c.benchmark_group("entropy_tax");
    group.bench_function("small", |b| {
        b.iter(|| calculate_entropy_tax(black_box(100), black_box(50.0)))
    });
    group.bench_function("large", |b| {
        b.iter(|| calculate_entropy_tax(black_box(2u64.pow(53) - 1), black_box(9999.0)))
    });
    group.bench_function("max_safe", |b| {
        b.iter(|| calculate_entropy_tax(black_box(MAX_SAFE_COMPLEXITY), black_box(1.0)))
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_galactic_drift,
    bench_orbital_resonance,
    bench_gravity_assist,
    bench_spacetime_curvature,
    bench_macro_shift,
    bench_entropy_tax
);
criterion_main!(benches);
