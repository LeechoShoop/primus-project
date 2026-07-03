use criterion::{black_box, criterion_group, criterion_main, Criterion};
use primus_vm::wasm::gas::GasMeter;

fn bench_from_energy(c: &mut Criterion) {
    let mut group = c.benchmark_group("from_energy");
    group.bench_function("zero", |b| {
        b.iter(|| GasMeter::from_energy(black_box(0.0)))
    });
    group.bench_function("normal", |b| {
        b.iter(|| GasMeter::from_energy(black_box(100.0)))
    });
    group.bench_function("max", |b| {
        b.iter(|| GasMeter::from_energy(black_box(f32::MAX)))
    });
    group.bench_function("nan", |b| {
        b.iter(|| GasMeter::from_energy(black_box(f32::NAN)))
    });
    group.finish();
}

fn bench_charge(c: &mut Criterion) {
    let mut group = c.benchmark_group("charge");
    group.bench_function("single", |b| {
        b.iter_with_setup(
            || GasMeter::from_energy(1000.0),
            |mut meter| {
                meter.charge(black_box(100)).unwrap();
            },
        )
    });

    group.bench_function("until_empty", |b| {
        b.iter_with_setup(
            || GasMeter { limit: 10_000, consumed: 0 },
            |mut meter| {
                while meter.charge(black_box(1)).is_ok() {}
            },
        )
    });

    group.bench_function("overflow", |b| {
        b.iter_with_setup(
            || GasMeter::from_energy(100.0),
            |mut meter| {
                let _ = meter.charge(black_box(u64::MAX));
            },
        )
    });
    group.finish();
}

fn bench_remaining(c: &mut Criterion) {
    let mut group = c.benchmark_group("remaining");
    group.bench_function("50_percent", |b| {
        b.iter_with_setup(
            || {
                let mut meter = GasMeter::from_energy(100.0);
                let half = meter.limit / 2;
                meter.charge(half).unwrap();
                meter
            },
            |meter| {
                black_box(meter.remaining());
            },
        )
    });
    group.finish();
}

criterion_group!(benches, bench_from_energy, bench_charge, bench_remaining);
criterion_main!(benches);
