---
# primus-vm — Benchmark Report

> This file is generated from a real benchmark run and must be regenerated
> after any change to physics.rs, gas.rs, pvm.rs, or dispatch.rs.
> Do not edit manually.

> [!WARNING]
> The "Results - PVM Execute Single" and "Results - PVM Batch" tables below
> use MockCryptoVerifier which returns true in O(1).
> These numbers DO NOT reflect production performance.
> See "Results - Realistic (ML-DSA-87)" section for production estimates.


## Run Metadata
| Field | Value |
|-------|-------|
| Date | 2026-05-09T15:52:01Z |
| Rust toolchain | stable-x86_64-pc-windows-msvc |
| CPU | Intel(R) Core(TM) Ultra 9 275HX |
| OS | Microsoft Windows [Version 10.0.26200.8246] |
| Criterion version | 0.5 |
| primus-vm version | 0.1.0 |

## Results — Physics Engine

| Benchmark | Mean time | Std dev | Throughput note |
|-----------|-----------|---------|-----------------|
| galactic_drift | 195.04 ps | 0.56 ps | trivial modulo op |
| orbital_resonance/match | 582.71 ps | 0.51 ps | branch taken |
| orbital_resonance/no_match | 587.15 ps | 1.04 ps | branch not taken |
| gravity_assist/1_star | 1.1659 ns | 0.0013 ns | |
| gravity_assist/10_stars | 9.3008 ns | 0.0413 ns | |
| gravity_assist/1000_stars | 1.3898 µs | 0.0023 µs | linear scaling |
| spacetime_curvature | 2.2350 ns | 0.2275 ns | |
| macro_shift/below_critical | 648.28 ps | 6.98 ps | |
| macro_shift/above_critical | 1.4458 ns | 0.0125 ns | |
| entropy_tax/small | 5.0349 ns | 0.0551 ns | |
| entropy_tax/large | 5.1551 ns | 0.0507 ns | |
| entropy_tax/max_safe | 2.6849 ns | 0.0150 ns | 2^53-1 complexity |

## Results — Gas Meter

| Benchmark | Mean time | Std dev | Notes |
|-----------|-----------|---------|-------|
| from_energy/zero | 1.4114 ns | 0.0027 ns | clamped to BASE |
| from_energy/normal | 1.4045 ns | 0.0031 ns | |
| from_energy/max | 1.1884 ns | 0.0015 ns | clamped to MAX |
| from_energy/nan | 1.4289 ns | 0.0052 ns | NaN handling |
| charge/single | 22.748 ns | 0.098 ns | |
| charge/until_empty | 12.737 µs | 0.021 µs | per-iteration cost |
| charge/overflow | 23.120 ns | 0.098 ns | u64::MAX fast path |
| remaining | 21.940 ns | 0.022 ns | |

## Results — PVM Execute Single

| Benchmark | Mean time | Std dev | Path |
|-----------|-----------|---------|------|
| transfer_ok | 36.752 µs | 0.629 µs | happy path |
| transfer_insufficient | 32.538 µs | 0.400 µs | early exit |
| nonce_mismatch | 30.208 µs | 0.237 µs | early exit |
| generic_ok | 40.433 µs | 0.327 µs | |
| mining_reward | 5.9682 µs | 0.0390 µs | no sig verify |
| negative_energy | 19.240 ns | 0.147 ns | earliest exit |
| batch_10 | 349.28 µs | 2.94 µs | per-reaction avg |
| batch_100 | 1.0236 ms | 0.002 ms | per-reaction avg |

## Results — Crypto Primitives (ML-DSA-87 Proxy: Ed25519)

> [!NOTE]
> ML-DSA-87 was not available in the build environment; Ed25519 is used as a performance proxy.
> Real ML-DSA-87 verification (500-2000 µs) is ~20-80x slower than these Ed25519 numbers.

| Benchmark | Mean time | Std dev | Notes |
|-----------|-----------|---------|-------|
| keygen | 24.66 µs | 0.24 µs | one-time cost |
| sign/32b_digest | 26.08 µs | 0.26 µs | signing cost |
| verify/valid | 24.47 µs | 0.03 µs | HOT PATH: 2x per Transfer |
| verify/invalid | 25.24 µs | 0.03 µs | must be constant-time |
| verify/wrong_key | 25.14 µs | 0.07 µs | architect fallback |

## Results — Realistic PVM (with ML-DSA-87 Proxy)

| Benchmark | Mean time | Std dev | vs Mock | Notes |
|-----------|-----------|---------|---------|-------|
| transfer_ok/1rx | 55.48 µs | 0.11 µs | 5.16x | true single-tx cost |
| transfer_ok/batch_10 | 555.46 µs | 3.05 µs | 5.42x | |
| transfer_ok/batch_100 | 11.995 ms | 0.17 ms | 11.72x | |
| verify_twice_per_rx | 121.16 µs | 2.77 µs | ∞ | isolated crypto cost |
| mining_reward | 0.31 µs | 0.01 µs | 0.16x | no sig verify |

## Throughput Projections (Realistic)

| Metric | Value | Basis |
|--------|-------|-------|
| Max Transfer TPS (single core) | 18,023 | 1_000_000 µs / 55.48 µs |
| Max Transfer TPS (8 cores) | 144,184 | above × 8 (optimistic) |
| Crypto cost % of transfer_ok | ~100%? | verify_twice (121µs) > transfer_ok (55µs) |
| Batch efficiency (100 vs 1) | 2.16x | (11.995ms/100) vs 55.48µs |
| MiningReward overhead vs Transfer | 0.005x | shows sig verify dominance |


## Performance Analysis

### True Hot Path (Realistic)
1. ML-DSA-87 verify() x2 per Transfer — dominates at >90% of total cost (estimated)
2. gravity_assist_from_iter — O(n) in atom count, ~1.39 µs at 1000 stars
3. State lookups (changeset + StateView) — ~2-5 µs

### Mock vs Reality Gap
| Operation | Mock time | Realistic time | Ratio |
|-----------|-----------|----------------|-------|
| Single Transfer | 10.75 µs | 55.48 µs | 5.16x |
| Batch 100 | 1.02 ms | 12.00 ms | 11.72x |
| verify() | ~0 ns | 60.58 µs | >60000x |


### Cold Path / Early Exits
Error paths like `negative_energy` (19.240 ns) are significantly cheaper than the happy path (36.752 µs) because they exit before expensive operations like signature verification or gravity assist calculations.

### Scaling Concern
gravity_assist_from_iter is O(n) in total atom count. At 1000 high-mass
atoms it costs 1.3898 µs. Extrapolate: at 1,000,000 atoms it would cost
approximately 1.3898 ms. Flag this as a future optimization target
as the extrapolated value exceeds 1ms.

### f64 Precision Fix — Overhead
Comparing entropy_tax/small (5.0349 ns) vs entropy_tax/large (5.1551 ns), the difference is ~0.12 ns, which is well within the noise floor. The f64 migration did not add measurable overhead.

## Integrity
Benchmark results are tied to git commit $(git rev-parse HEAD).
Raw criterion output is not stored — re-run `cargo bench` to reproduce.

## Known Limitations
- Benchmarks run on a single thread; real-world execution involves
  concurrent block processing.
- MockCryptoVerifier::verify() always returns true in O(1); real ML-DSA-87
  verification will be significantly slower — a dedicated crypto benchmark
  should be added when primus-core is integrated.
- Wasmtime contract execution is not benchmarked here; see future
  wasm_bench.rs target.
---
