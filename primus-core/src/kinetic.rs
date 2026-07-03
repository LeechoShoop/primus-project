// =============================================================================
// kinetic.rs — Reaction Primitives (Refactored to use primus-types)
// =============================================================================

use crate::atom::Atom;
use anyhow::Result;
pub use primus_types::payload::Payload;
pub use primus_types::reaction::SignedReaction;
use std::time::{SystemTime, UNIX_EPOCH};

// ── KineticEngine ─────────────────────────────────────────────────────────────

pub struct KineticEngine;

impl KineticEngine {
    /// Constructs a fully signed Transfer reaction using nonce-based anti-replay.
    /// Uses primus_types::SignedReaction and its canonical signing_digest().
    pub fn build_transfer(
        sender_atom: Atom,
        receiver_pk: Vec<u8>,
        amount: u64,
        fee: u64,
        signing_key_bytes: &[u8],
    ) -> Result<SignedReaction> {
        use crate::atom::Element;
        use crate::crypto::Crypto;

        let receiver = Atom::new_materialized(receiver_pk.clone(), Element::Hydrogen);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow::anyhow!("Time drift: {}", e))?
            .as_secs();

        let mut rx = SignedReaction {
            sender: sender_atom,
            receiver,
            reaction_hash: [0u8; 32],
            energy: fee as f32,
            timestamp: now,
            signature: vec![],
            payload: Payload::Transfer { amount },
        };

        // Compute reaction_hash and signature using canonical methods
        rx.reaction_hash = rx.compute_reaction_hash();

        // PVM expects signature over signing_digest()
        let digest = rx.signing_digest();
        rx.signature = Crypto::sign(signing_key_bytes, &digest)?;

        Ok(rx)
    }
}
