// =============================================================================
// primus-net-opt/src/gravity_shield.rs
//
// GravityShield — first line of defence for incoming raw network bytes.
//
// This module lives in primus-net-opt so it can be reused by server.rs and
// network.rs without depending on primus-core internals.
//
// Responsibilities (cheap pre-screening only):
//   1. Bincode deserialization — drop malformed frames immediately.
//   2. Structural validity check via SignedReaction::validate_structure().
//      This uses rkyv field-range checks without running ML-DSA crypto.
//   3. Basic sanity: non-empty sender key, non-negative energy.
//
// What the shield does NOT do (deferred to the PVM inside primus-core):
//   - ML-DSA-87 signature verification
//   - Nonce / replay-protection checks
//   - Mass balance / entropy-tax calculation
//   - Quantum state validation
//
// The shield holds no engine state, so it is zero-cost to clone and
// thread-safe by construction. All context-dependent checks are performed
// by CoreHandle::shield_filter in primus-core after this pass.
// =============================================================================

use anyhow::{Result, anyhow};
use primus_types::reaction::SignedReaction;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight pre-filter applied to raw P2P bytes before they enter the
/// mempool pipeline.
#[derive(Clone, Default)]
pub struct GravityShield {
    /// Cumulative count of frames dropped by this shield instance.
    /// Shared via Arc so multiple tasks can increment it safely.
    pub drops: Arc<AtomicU64>,
}

impl GravityShield {
    pub fn new() -> Self {
        Self {
            drops: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns the total number of frames dropped since node start.
    pub fn drop_count(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }

    /// Validate raw bytes and return an owned `SignedReaction`.
    /// Increments the drop counter on any validation failure.
    pub fn filter_bytes(&self, raw: &[u8]) -> Result<SignedReaction> {
        let result = self.filter_bytes_inner(raw);
        if result.is_err() {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn filter_bytes_inner(&self, raw: &[u8]) -> Result<SignedReaction> {
        // ── Layer 1: rkyv structural check and deserialization ──────────────────────────────────
        // (RDIV-001) Transaction frames are now rkyv-encoded, not bincode.
        let archived = rkyv::check_archived_root::<SignedReaction>(raw)
            .map_err(|e| anyhow!("GravityShield: malformed frame (rkyv): {}", e))?;
        let rx: SignedReaction = rkyv::Deserialize::<SignedReaction, _>::deserialize(archived, &mut rkyv::Infallible)
            .map_err(|e| anyhow!("GravityShield: rkyv deserialization failed: {:?}", e))?;

        // ── Layer 2: Structural field validity ───────────────────────────────
        rx.validate_structure()
            .map_err(|e| anyhow!("GravityShield: structural check failed: {:?}", e))?;

        // ── Layer 3: Basic sanity ─────────────────────────────────────────────
        if rx.sender.public_key.is_empty() {
            return Err(anyhow!("GravityShield: zero-length sender public key"));
        }
        if rx.energy < 0.0 {
            return Err(anyhow!(
                "GravityShield: negative energy ({}) — Conservation of Energy violation",
                rx.energy
            ));
        }

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_shield_increments_drop_counter() {
        let shield = GravityShield::new();
        assert_eq!(shield.drop_count(), 0);

        // Feed garbage bytes — should fail and increment counter
        let _ = shield.filter_bytes(b"not valid bincode");
        assert_eq!(shield.drop_count(), 1);

        // Feed another bad frame
        let _ = shield.filter_bytes(&[0xFF; 10]);
        assert_eq!(shield.drop_count(), 2);
    }
}
