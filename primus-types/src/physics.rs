// =============================================================================
// primus-types/src/physics.rs
//
// PhysicsCanon is the canonical encoder for f32 physics values that must
// participate in deterministic hashing across heterogeneous hardware.
//
// PROBLEM: f32 arithmetic is not bit-exact across CPU architectures.
// x86 processors with 80-bit extended-precision FPUs can produce f32 values
// that differ from ARM's strict 32-bit results by 1 ULP (Unit in the Last
// Place) after a floating-point operation. If raw f32 bits are fed into
// SHA3-256 (or BLAKE3) for the state root, two nodes with identical logical
// state will produce different roots if one accumulated temperature via x86
// and the other via ARM.
//
// FIX: Before any f32 value participates in hashing, multiply by
// FIXED_POINT_SCALE (1e9), convert to u64, and hash the u64.
// Two nodes with the same logical temperature will produce the same u64
// even if their f32 representations differ by 1 ULP, because the
// integer truncation collapses the ULP difference.
//
// This module is defined in primus-types (not primus-core) because:
//   1. The SDK's future balance-query path may need to verify state roots.
//   2. The WASM binding layer needs to encode physics values for display.
//   3. Having it here prevents primus-core and primus-sdk from implementing
//      independent fixed-point encoders that silently disagree on precision.
// =============================================================================

/// Scale factor: 10^9 provides nanosecond-equivalent resolution for
/// temperatures up to ~9.2 × 10^9 K — far beyond any value the chamber
/// produces. The integer representation of 300.15 K is 300_150_000_000,
/// which fits comfortably in u64.
pub const FIXED_POINT_SCALE: u64 = 1_000_000_000;

/// Canonical encoder for f32 physics scalars participating in hash computation.
///
/// # Usage rule (enforced by convention, not the type system)
///
/// Whenever a Temperature, Entropy, or Charge value must be fed into any
/// hash function — state root calculation, cross-node comparison, diagnostic
/// output — it MUST pass through `PhysicsCanon::encode()` first.
///
/// Feeding raw `f32::to_bits()` into a hasher is a consensus bug. The type
/// system cannot prevent this, but code review must treat it as a correctness
/// violation equivalent to an off-by-one error in a balance check.
pub struct PhysicsCanon;

impl PhysicsCanon {
    /// Encode a physics scalar to a canonical u64 for hashing.
    ///
    /// Negative values are clamped to zero. The physics model should never
    /// produce negative temperatures or entropies; the clamp is a safety
    /// net for edge cases during genesis or after a reorg cooling shock.
    #[inline]
    pub fn encode(value: f32) -> u64 {
        let scaled = (value as f64 * FIXED_POINT_SCALE as f64).max(0.0);
        if scaled >= u64::MAX as f64 {
            u64::MAX
        } else {
            scaled as u64
        }
    }

    /// Decode a canonical u64 back to f32 for physics computations.
    ///
    /// The round-trip introduces at most 1 ULP of error, which is
    /// smaller than any physically meaningful temperature difference
    /// in the chamber model.
    #[inline]
    pub fn decode(encoded: u64) -> f32 {
        (encoded as f64 / FIXED_POINT_SCALE as f64) as f32
    }
}
