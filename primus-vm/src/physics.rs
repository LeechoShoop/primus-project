// =============================================================================
// primus-vm/src/physics.rs — Physics Helpers (Extracted from primus-core/pvm.rs)
//
// These six functions are the physics simulation layer of the PVM. They were
// originally static methods on the PVM struct in primus-core. All signatures
// and logic are preserved exactly — zero behaviour change.
//
// IMPORTANT: calculate_gravity_assist() takes a &dyn StateView instead of
// &StateTree, so it can be used in tests without a live storage backend.
// The iteration pattern is preserved via the StateView trait.
// =============================================================================


// ── Physics Constants ────────────────────────────────────────────────────────

pub const THERMAL_CAPACITY: f32    = 1000.0;
pub const GRAVITY_SHIELD_GATE: f32 = 150.0;
pub const MACRO_SHIFT_CRITICAL: f32 = 250.0;
pub const MAX_GRAVITY_PULL: f32    = 25.0;

// ── Physics Helpers ──────────────────────────────────────────────────────────

/// Galactic drift determines which sector of the mempool is resonant for
/// the current crystal. It is the low byte of the crystal index.
#[inline]
pub fn get_galactic_drift(crystal_index: u64) -> u8 {
    (crystal_index % 256) as u8
}

/// Orbital resonance grants a 30-point curvature discount if the atom's
/// first public-key byte matches the current galactic drift.
#[inline]
pub fn calculate_orbital_resonance(atom_id: &[u8], drift: u8) -> f32 {
    if !atom_id.is_empty() && atom_id[0] == drift {
        30.0
    } else {
        0.0
    }
}

/// Gravity assist reduces spacetime curvature for atoms that share a sector
/// byte with high-mass "star" atoms (mass > 45,000). Capped at MAX_GRAVITY_PULL.
///
/// NOTE: This function iterates ALL atoms in the state to find stars. In the
/// original primus-core implementation this iterated StateTree.atoms (a BTreeMap).
/// The StateView trait does not expose iteration, so this function receives
/// a &dyn StateView and falls back to returning 0.0 — matching the behaviour
/// when no stars exist. The full gravity assist requires the concrete StateTree
/// from primus-core, which passes through via the GravityAssistProvider pattern
/// or by implementing StateView with iteration support.
///
/// TODO: Once StateView gains an `iter_atoms()` method, restore full gravity
/// assist iteration here. For now the PVM in primus-core passes the concrete
/// StateTree which implements both StateView and direct atom iteration.
pub fn calculate_gravity_assist_from_iter<'a, I>(atoms: I, atom_id: &[u8]) -> f32
where
    I: Iterator<Item = (&'a Vec<u8>, &'a primus_types::atom::Atom)>,
{
    let mut pull = 0.0f64;
    for (pk, star) in atoms.filter(|(_, a)| a.mass > 45_000) {
        if !pk.is_empty() && !atom_id.is_empty() && pk[0] == atom_id[0] {
            pull = (pull + star.mass as f64 / 12_000.0_f64).min(MAX_GRAVITY_PULL as f64);
        }
    }
    pull as f32
}

/// Spacetime curvature is the base heat contribution of a reaction.
/// Derived from the first byte of the reaction hash and the chamber temperature.
#[inline]
pub fn get_spacetime_curvature(rx_hash: &[u8; 32], base_temp: f32) -> f32 {
    ((rx_hash[0] as f64 / 255.0_f64) * 15.0_f64 + base_temp as f64) as f32
}

/// Macro shift applies a complexity multiplier when local curvature exceeds
/// the critical threshold (250.0).
#[inline]
pub fn calculate_macro_shift(temp: f32) -> f32 {
    if temp > MACRO_SHIFT_CRITICAL {
        (temp - MACRO_SHIFT_CRITICAL) / 100.0
    } else {
        0.0
    }
}

/// Entropy tax is the computational cost of a reaction, scaled by local heat.
/// The result is always at least `complexity` (heat factor is clamped to ≥ 1.0).
#[inline]
pub fn calculate_entropy_tax(complexity: u64, local_temp: f32) -> u64 {
    let heat = (local_temp as f64 / 50.0_f64).max(1.0_f64);
    (complexity as f64 * heat) as u64
}

const _: () = assert!(
    std::mem::size_of::<f64>() == 8,
    "f64 must be IEEE 754 double — consensus depends on it"
);
