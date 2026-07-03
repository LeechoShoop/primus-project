// =============================================================================
// primus-types/src/lib.rs
//
// ARCHITECTURAL INVARIANTS — read before touching anything in this crate:
//
//   INVARIANT 1 — NO CRYPTOGRAPHIC LOGIC (with one transitional exception).
//     The only hash call in this crate is SignedReaction::signing_digest(),
//     which computes the message to be signed, not a signature.
//     The one exception: PrimusNR::verify() is gated behind the `verify`
//     feature flag and is explicitly marked for migration to primus-core.
//     No further ML-DSA call sites may be added to this crate.
//
//   INVARIANT 2 — BINCODE WIRE FORMAT IS FROZEN AT DEFINITION.
//     bincode v1 serializes structs by field declaration order and enums by
//     variant declaration index (0-based). The comments on each type state
//     the exact wire layout. Any change to field or variant order is a hard
//     protocol break. Use ONLY field additions at the end of structs with
//     #[serde(default)]. NEVER reorder, rename, or remove existing fields.
//
//   INVARIANT 3 — ALL PUBLIC TYPES MUST SATISFY: Send + Sync + Clone.
//     Required for rayon parallel verification (Send), Arc<RwLock<>> state
//     sharing (Sync), and PVM changeset construction (Clone).
//
//   INVARIANT 4 — NO std TYPES IN CORE STRUCTS.
//     Vec<u8>, String, and arrays only. No HashMap, BTreeMap, SystemTime.
//     The exception is peer.rs, whose SocketAddr helpers are cfg(feature="std").
//
//   INVARIANT 5 — SERIALIZATION FORMAT SEPARATION.
//     bincode (via serde) is the P2P wire format and on-disk storage format.
//     rkyv is the in-process zero-copy format for hot paths in primus-core.
//     These formats are NEVER interchangeable. rkyv output must never be
//     written to disk or sent over the network.
// =============================================================================

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use alloc::{string::String, vec::Vec};

// ── Public modules ────────────────────────────────────────────────────────────
pub mod atom;
pub mod crystal;

pub mod constants;
pub mod error;
pub mod galactic_sync;
pub mod ipc;
pub mod payload;
pub mod peer;
pub mod physics;
pub mod reaction;
pub mod proof;


// ── Flat re-exports — the stable public surface ───────────────────────────────
pub use atom::{Atom, Element, QuantumState};
pub use crystal::Crystal;

pub use constants::{
    MINING_REWARD_AMOUNT, MINING_REWARD_TAG, PK_BYTES, PROTOCOL_MIN_FEE, REACTION_HASH_BYTES,
    SEED_DOMAIN_TAG, SIG_BYTES, MPT_PROOF_VERSION,
};
pub use error::PrimusError;
pub use galactic_sync::{GalacticStatus, SyncMessage};
pub use ipc::{IpcRequest, IpcResponse};
pub use payload::Payload;
pub use peer::{NoiseHandshakePayload, PrimusNR};
pub use physics::PhysicsCanon;
pub use reaction::SignedReaction;
pub use proof::{MerkleProof, PathStep};


// ── Compile-time invariant checks ─────────────────────────────────────────────
#[cfg(test)]
mod invariants {
    use super::*;

    fn assert_send_sync_clone<T: Send + Sync + Clone>() {}

    #[test]
    fn public_types_are_send_sync_clone() {
        assert_send_sync_clone::<Atom>();
        assert_send_sync_clone::<SignedReaction>();
        assert_send_sync_clone::<Payload>();
        assert_send_sync_clone::<PrimusNR>();
        assert_send_sync_clone::<NoiseHandshakePayload>();
        assert_send_sync_clone::<PrimusError>();
    }
}
