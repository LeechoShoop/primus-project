---
# primus-vm - Qualification Record v2

> WARNING: THIS FILE IS AN IMMUTABLE AUDIT ARTIFACT.
> Once qualified.md exists, the implementation files of primus-vm are
> considered FROZEN. No AI agent, automated tool, or human contributor may
> modify any .rs file or Cargo.toml inside primus-vm/ without:
>   1. Deleting this file first (requires a separate, reviewed commit).
>   2. Re-running the full test suite - all tests must pass.
>   3. Re-running fuzz targets for minimum 30 minutes each.
>   4. Generating a new qualified.md that supersedes this one.
> Violation of this protocol invalidates the qualification of the entire module.

## Qualification Date
2026-05-09T15:25:34Z

## Supersedes
qualified.md v1 - invalidated by commit: fix(physics): replace f32 with f64
in consensus-critical calculations

## Change Rationale
f32 arithmetic in calculate_entropy_tax, calculate_gravity_assist_from_iter,
and get_spacetime_curvature caused non-deterministic rounding for complexity
values > 2^24, which would cause changeset hash divergence between nodes
and trigger a consensus fork. All three functions now compute in f64 internally.

## Module Version
0.1.0

## Rust Toolchain
stable-x86_64-pc-windows-msvc (default)

## Test Suite Results
| Test suite | Tests run | Passed | Failed | Ignored |
|------------|-----------|--------|--------|---------|
| unit (src/) | 4 | 4 | 0 | 0 |
| pvm_integration | 3 | 3 | 0 | 0 |
| wasm_integration | 2 | 2 | 0 | 0 |
| property_tests | 24 | 24 | 0 | 0 |
| **TOTAL** | 33 | 33 | 0 | 0 |

## Previously Failing Test - Now Fixed
| Test | Root cause | Fix applied | Status |
|------|-----------|-------------|--------|
| prop_physics_entropy_tax_bound | f32 mantissa overflow for complexity > 2^24 | f64 internal arithmetic | PASS |

## New Tests Added
| Test | Invariant verified | Severity |
|------|--------------------|----------|
| prop_entropy_tax_determinism | Same inputs always produce identical entropy tax | Critical |

## Invariants Verified (complete list)
1. Arbitrary (amount, energy) pairs must never cause UB or panic.
2. Charging more than u64::MAX total must return GasOverflow, never wrap.
3. After OutOfGas the meter must remain in a consistent state (consumed > limit)
4. remaining() == limit.saturating_sub(consumed) always holds.
5. from_energy(NaN, Inf, etc) must not panic.
6. limit in [BASE_CONTRACT_GAS, MAX_GAS_PER_REACTION] always.
7. get_galactic_drift: for any crystal_index, result < 256.
8. calculate_orbital_resonance: result in {0.0, 30.0} always.
9. calculate_gravity_assist_from_iter: result in [0.0, MAX_GRAVITY_PULL]
10. get_spacetime_curvature: result is finite, never NaN.
11. calculate_macro_shift: result >= 0.0 always.
12. calculate_entropy_tax: result >= complexity always
13. Entropy tax must be deterministic for a given (complexity, temp) pair.
14. ptr < 0 -> None, len < 0 -> None, overflow -> None, OOB -> None
15. transfer_mass calls must not overflow pending_out (uses saturating_add).
16. No panic, no UB for arbitrary inputs.
17. ThermalLimitExceeded is returned before changeset grows past the limit.
18. If Err(_) -> changeset is EMPTY (atomic failure invariant).
19. executing the same valid SignedReaction twice in a row (same nonce) must return NonceMismatch on the second call.
20. Transfer with amount = u64::MAX and energy = f32 that casts to u64::MAX must return InsufficientMass or ArithmeticOverflow, never Ok.
21. MAX_WASM_MEMORY_PAGES * 65536 == 16 MiB (16_777_216).
22. GAS_HEAT_DIVISOR > 0.0 (division must never be by zero).
23. MODULE_CACHE_SIZE > 0.
24. storage_cost = code.len() * 100 must not silently overflow (test with code.len() > usize::MAX / 100).

## Safety Guards Added
1. MAX_SAFE_COMPLEXITY = 2^53 in wasm/limits.rs
2. execute_single returns ArithmeticOverflow if complexity_scaled > MAX_SAFE_COMPLEXITY
3. Compile-time assert: size_of::<f64>() == 8 in physics.rs

## Known Limitations
- Wasmer backend (Phase 3) not yet implemented.
- Fuzz corpus runs require cargo fuzz and were not executed at qualification time.
- Physics results remain f32 at API boundary for compatibility; internal
  precision is f64.

## Fuzz Test Results
| Field | Value |
|-------|-------|
| Fuzz target | pvm_harness |
| Engine | BLOCKED - libFuzzer not available on Windows MSVC |
| Duration | 0 seconds - not executed |
| Total executions | 0 |
| Executions/sec | N/A |
| Crashes found | N/A - not executed |
| Sanitizer | N/A |
| Date run | N/A |

### Fuzz Coverage Notes
proptest property tests (24 cases) provide partial coverage but are NOT a substitute for libFuzzer corpus-based fuzzing. Full fuzz qualification is PENDING CI execution.

### Result
BLOCKED - libFuzzer requires Linux/macOS. cargo-fuzz is not supported on Windows (x86_64-pc-windows-msvc toolchain). Fuzz execution must be performed in CI on a Linux runner before this module can be considered fully qualified.
Recommend: GitHub Actions ubuntu-latest + cargo fuzz run pvm_harness -- -max_total_time=1800

## CI Pipeline
| File | Purpose |
|------|---------|
| .github/workflows/fuzz.yml | Weekly 30-min fuzz on ubuntu-latest + integrity check |
| Trigger | Every Monday 02:00 UTC + manual workflow_dispatch |
| Fuzz full qualification | Pending first successful CI run |

## Integrity
This record's authenticity is guaranteed by git history.
The commit that introduced or last modified this file is the
authoritative reference. Do not verify via file hash -
verify via `git log --follow primus-vm/qualified.md`.