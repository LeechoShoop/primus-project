// =============================================================================
// primus-net-opt/src/gravity_net.rs
//
// GravityNet — galactic-filter helpers for the network layer.
//
// MIGRATION NOTE: The original version of this file imported crate::pvm,
// crate::state, and crate::kinetic — all primus-core internals. Those imports
// are invalid in primus-net-opt (wrong crate). The semantic validation logic
// has been moved into CoreHandle::shield_filter in primus-core where it has
// access to the StateTree and PVM.
//
// This module is now a thin helper that classifies messages by their galactic
// sector drift, used by the gossip layer for priority routing.
// =============================================================================

/// Compute the galactic drift for a given crystal index.
/// Mirrors the constant in primus-core's PVM without depending on it.
///
/// Drift = crystal_index mod 256.
#[inline]
pub fn get_galactic_drift(crystal_index: u64) -> u8 {
    (crystal_index % 256) as u8
}

/// Returns true if the sender's first public-key byte matches the current
/// galactic drift sector. Resonant reactions get priority in the mempool.
#[inline]
pub fn is_resonant(sender_pk_first_byte: u8, crystal_index: u64) -> bool {
    sender_pk_first_byte == get_galactic_drift(crystal_index)
}
