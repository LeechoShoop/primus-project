// =============================================================================
// primus-types/src/reaction.rs
//
// SignedReaction — the unified transaction type for Obsidian Nexus.
//
// WIRE FORMAT CONTRACT (bincode v1, field declaration order):
//   0:  sender          Atom       (see atom.rs for Atom wire layout)
//   1:  receiver        Atom
//   2:  reaction_hash   [u8; 32]
//   3:  energy          f32        (network fee; always an integer in practice)
//   4:  timestamp       u64        (Unix seconds, for mempool age-out only)
//   5:  signature       Vec<u8>    (ML-DSA-87, SIG_BYTES = 4627)
//   6:  payload         Payload    (discriminant + optional data)
//
// SIGNING vs REACTION HASH:
//   These two SHA3-256 values have different semantic roles and MUST NOT be
//   conflated, even though they are computed from the same inputs today:
//
//   reaction_hash — the on-chain transaction identifier. Stored in the
//     Crystal's reactions list, becomes the sender's new last_reaction_hash
//     after confirmation, and is indexed by the mempool. Computed by
//     compute_reaction_hash().
//
//   signing_digest — the exact byte sequence the ML-DSA-87 key signs.
//     The PVM verifies the signature against signing_digest(), NOT against
//     reaction_hash. Computed by signing_digest().
//
//   Today signing_digest() == compute_reaction_hash() by construction.
//   They are kept as separate methods with separate names because they may
//   diverge in a future protocol version (e.g., signing_digest gains a
//   version prefix or domain separation tag). Callers MUST use the
//   semantically correct method — do not substitute one for the other.
//
// FEE ENCODING:
//   `energy` is stored as f32 for wire-format compatibility with existing
//   primus-core and primus-sdk. For all protocol comparisons (fee floor
//   check, signing digest) the fee is encoded via PhysicsCanon::encode()
//   to guarantee deterministic results across architectures. See physics.rs.
// =============================================================================

use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use zeroize::{Zeroize, ZeroizeOnDrop};


use crate::atom::Atom;
use crate::constants::{MINING_REWARD_TAG, REACTION_HASH_BYTES};
use crate::error::PrimusError;
use crate::payload::Payload;
use crate::physics::PhysicsCanon;

/// A fully constructed, signed Primus transaction.
///
/// Produced by: `primus-sdk::TransactionBuilder::build()`
/// Consumed by: `primus-core::pvm::PVM::execute_payload()`
/// Transmitted as: `PrimusMessage::NewReaction(bincode::serialize(reaction), ttl)`
#[derive(
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Zeroize,
    ZeroizeOnDrop,
)]

#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
#[repr(C)]
pub struct SignedReaction {
    /// The sender's on-chain state snapshot at signing time.
    /// The PVM validates that `sender.last_reaction_hash` matches the
    /// on-chain value — this is the sequence / anti-replay check.
    pub sender: Atom,

    /// The receiver's current on-chain state, or a zero-mass snapshot
    /// for a new address (auto-materialized by the PVM on confirmation).
    pub receiver: Atom,

    /// The on-chain identifier for this transaction.
    /// SHA3-256 of the canonical transfer parameters — see compute_reaction_hash().
    /// This is NOT the signing message; see signing_digest().
    pub reaction_hash: [u8; 32],

    /// Network fee in mass units. Burned by the protocol (not sent to receiver).
    /// Must satisfy `PhysicsCanon::encode(energy) >= PROTOCOL_MIN_FEE`.
    ///
    /// Stored as f32 for wire-format compatibility. Always set to an integer
    /// value (minimum = PROTOCOL_MIN_FEE = 10) in practice. All comparisons
    /// and hash inputs use PhysicsCanon::encode(energy) — never raw f32 bits.
    pub energy: f32,

    /// Unix timestamp (seconds) at transaction construction time.
    /// Not used by the PVM for ordering (last_reaction_hash provides that).
    /// Used only for mempool age-out and diagnostic logging.
    pub timestamp: u64,

    /// ML-DSA-87 signature over `signing_digest()`.
    /// Length must equal SIG_BYTES (4627) for non-MiningReward transactions.
    /// Empty (`vec![]`) for MiningReward (self-authenticating via reaction_hash).
    pub signature: Vec<u8>,

    /// The economic action this reaction requests.
    #[serde(default)]
    pub payload: Payload,
}

impl SignedReaction {
    // =========================================================================
    // SIGNING DIGEST
    // =========================================================================

    /// Compute the exact byte sequence that an ML-DSA-87 key must sign.
    ///
    /// # Protocol specification
    ///
    /// ```text
    /// signing_input =
    ///     sender.public_key                         (PK_BYTES bytes, no length prefix)
    ///  ++ receiver.public_key                       (PK_BYTES bytes, no length prefix)
    ///  ++ transfer_amount_or_zero.to_le_bytes()     (8 bytes, little-endian u64)
    ///  ++ PhysicsCanon::encode(fee).to_le_bytes()   (8 bytes, little-endian u64)
    ///  ++ sender.last_reaction_hash                 (32 bytes, raw)
    ///
    /// signing_digest = SHA3-256(signing_input)
    /// ```
    ///
    /// The fee is encoded via `PhysicsCanon::encode()` rather than
    /// `energy as u64` to guarantee identical output across x86 and ARM
    /// regardless of FPU extended-precision behaviour.
    ///
    /// # Usage
    ///
    /// SDK:  `let sig = wallet.sign(&reaction.signing_digest());`
    /// PVM:  `Crypto::verify(&sender.public_key, &rx.signing_digest(), &rx.signature)`
    ///
    /// This is the ONLY path to the signing message. It must not be
    /// reimplemented in primus-sdk or primus-core.
    pub fn signing_digest(&self) -> [u8; REACTION_HASH_BYTES] {
        let transfer_amount: u64 = self.payload.transfer_amount().unwrap_or(0);
        let fee_encoded: u64 = PhysicsCanon::encode(self.energy);

        let mut input = Vec::with_capacity(
            self.sender.public_key.len()
                + self.receiver.public_key.len()
                + 8   // amount
                + 8   // fee
                + 32, // last_reaction_hash
        );
        input.extend_from_slice(&self.sender.public_key);
        input.extend_from_slice(&self.receiver.public_key);
        input.extend_from_slice(&transfer_amount.to_le_bytes());
        input.extend_from_slice(&fee_encoded.to_le_bytes());
        input.extend_from_slice(&self.sender.last_reaction_hash);

        Sha3_256::digest(&input).into()
    }

    // =========================================================================
    // REACTION HASH DERIVATION
    // =========================================================================

    /// Compute the canonical on-chain identifier for this transaction.
    ///
    /// The reaction_hash is stored in the Crystal's `reactions` list,
    /// becomes the sender's new `last_reaction_hash` after confirmation,
    /// and is indexed by the mempool.
    ///
    /// # Current implementation
    ///
    /// In the current protocol version, reaction_hash and signing_digest
    /// are computed from the same inputs and therefore produce the same
    /// output. They are kept as separate methods because their semantic
    /// roles differ and they may diverge in a future protocol version.
    ///
    /// Callers MUST use `compute_reaction_hash()` when computing the
    /// on-chain identifier, and `signing_digest()` when computing the
    /// message to sign or verify. Do not substitute one for the other.
    pub fn compute_reaction_hash(&self) -> [u8; REACTION_HASH_BYTES] {
        self.signing_digest()
    }

    // =========================================================================
    // MINING REWARD HASH
    // =========================================================================

    /// Compute the canonical reaction_hash for a MiningReward transaction.
    ///
    /// MiningReward transactions are self-authenticating: validity is
    /// established by verifying that `reaction_hash` matches this formula.
    /// No ML-DSA signature is required or checked.
    ///
    /// Formula: `SHA3-256(MINING_REWARD_TAG || crystal_index_le8 || architect_pk)`
    ///
    /// Both `engine::build_mining_reward_rx()` and `PVM::execute_payload()`
    /// must use this function. Independent re-implementations are bugs.
    pub fn mining_reward_hash(
        crystal_index: u64,
        architect_pk: &[u8],
    ) -> [u8; REACTION_HASH_BYTES] {
        let mut hasher = Sha3_256::new();
        hasher.update(MINING_REWARD_TAG);
        hasher.update(crystal_index.to_le_bytes());
        hasher.update(architect_pk);
        hasher.finalize().into()
    }

    // =========================================================================
    // ZERO-COPY DESERIALIZATION
    // =========================================================================

    /// Deserialize and structurally validate a `SignedReaction` from rkyv bytes
    /// without any allocation or copying.
    ///
    /// Returns a reference into `bytes` valid for the lifetime of `bytes`.
    /// The returned `ArchivedSignedReaction` supports zero-copy field access
    /// and can be validated further via `ArchivedSignedReaction::validate_structure()`.
    ///
    /// # When to use
    ///
    /// Use this on the hot path in primus-core (mempool ingress, PVM pre-filter)
    /// where avoiding allocation on every inbound transaction matters.
    /// Use `bincode::deserialize::<SignedReaction>()` for storage I/O and IPC.
    ///
    /// # Errors
    ///
    /// Returns `PrimusError::DeserializationFailed` if the bytes fail rkyv's
    /// structural check (check_bytes). This does NOT validate the transaction
    /// semantics — call `validate_structure()` on the returned reference.
    pub fn from_bytes_zero_copy(bytes: &[u8]) -> Result<&ArchivedSignedReaction, PrimusError> {
        rkyv::check_archived_root::<SignedReaction>(bytes).map_err(|_| {
            PrimusError::DeserializationFailed {
                reason: "rkyv structural validation (check_bytes) failed",
            }
        })
    }

    // =========================================================================
    // STRUCTURAL VALIDATION
    // =========================================================================

    /// Validate the structural invariants of this reaction without accessing
    /// on-chain state and without performing ML-DSA signature verification.
    ///
    /// Checks performed:
    ///   1. Sender public key length == PK_BYTES
    ///   2. Receiver public key length == PK_BYTES
    ///   3. Signature length == SIG_BYTES (exempt for MiningReward)
    ///   4. Fee >= PROTOCOL_MIN_FEE (via PhysicsCanon::encode for determinism)
    ///   5. reaction_hash matches compute_reaction_hash()
    ///
    /// Does NOT check:
    ///   - Sender mass sufficiency (requires StateTree)
    ///   - Sequence number correctness (requires StateTree)
    ///   - Cryptographic signature validity (requires ml-dsa)
    ///
    /// This method is callable in no_std without chain state or cryptography.
    pub fn validate_structure(&self) -> Result<(), PrimusError> {
        use crate::constants::{PK_BYTES, PROTOCOL_MIN_FEE, SIG_BYTES};

        if self.sender.public_key.len() != PK_BYTES {
            return Err(PrimusError::InvalidPublicKeyLength {
                expected: PK_BYTES,
                actual: self.sender.public_key.len(),
            });
        }
        if self.receiver.public_key.len() != PK_BYTES {
            return Err(PrimusError::InvalidPublicKeyLength {
                expected: PK_BYTES,
                actual: self.receiver.public_key.len(),
            });
        }

        if self.payload.requires_signature_verification() && self.signature.len() != SIG_BYTES {
            return Err(PrimusError::InvalidSignatureLength {
                expected: SIG_BYTES,
                actual: self.signature.len(),
            });
        }

        // Use PhysicsCanon::encode for the fee comparison to guarantee
        // identical results on x86 (80-bit FPU) and ARM (strict 32-bit).
        // Raw `energy as u64` truncation is NOT deterministic across
        // architectures for values near integer boundaries.
        if PhysicsCanon::encode(self.energy) < PROTOCOL_MIN_FEE {
            return Err(PrimusError::FeeBelowMinimum {
                minimum: PROTOCOL_MIN_FEE,
                actual: PhysicsCanon::encode(self.energy),
            });
        }

        let expected_hash = self.compute_reaction_hash();
        if self.reaction_hash != expected_hash {
            return Err(PrimusError::ReactionHashMismatch {
                expected: expected_hash,
                actual: self.reaction_hash,
            });
        }

        Ok(())
    }
}

// ── ArchivedSignedReaction — zero-copy hot-path methods ───────────────────────

impl ArchivedSignedReaction {
    /// Zero-copy equivalent of `SignedReaction::signing_digest()`.
    ///
    /// See `SignedReaction::signing_digest()` for the full protocol specification.
    /// This implementation accesses all fields through rkyv's zero-copy references
    /// to avoid any heap allocation on the verification hot path.
    pub fn signing_digest(&self) -> [u8; REACTION_HASH_BYTES] {
        let transfer_amount: u64 = self.payload.transfer_amount().unwrap_or(0);
        // ArchivedF32 derefs to f32 via core::ops::Deref in rkyv 0.7.
        // In rkyv 0.7, Archived<f32> extraction: f32::from() resolves via
        // the Into<f32> impl provided by rend (or is a plain copy if the
        // archive_le/be features are not enabled and f32 archives as itself).
        let fee_encoded: u64 = PhysicsCanon::encode(self.energy);

        let mut input = Vec::with_capacity(
            self.sender.public_key.len() + self.receiver.public_key.len() + 8 + 8 + 32,
        );
        input.extend_from_slice(self.sender.public_key.as_slice());
        input.extend_from_slice(self.receiver.public_key.as_slice());
        input.extend_from_slice(&transfer_amount.to_le_bytes());
        input.extend_from_slice(&fee_encoded.to_le_bytes());
        input.extend_from_slice(self.sender.last_reaction_hash.as_slice());

        Sha3_256::digest(&input).into()
    }

    /// Zero-copy structural validation.
    ///
    /// Mirrors `SignedReaction::validate_structure()` exactly. The reaction_hash
    /// check delegates to `compute_reaction_hash()` (which delegates to
    /// `signing_digest()`) to keep both code paths consistent.
    pub fn validate_structure(&self) -> Result<(), PrimusError> {
        use crate::constants::{PK_BYTES, PROTOCOL_MIN_FEE, SIG_BYTES};

        if self.sender.public_key.len() != PK_BYTES {
            return Err(PrimusError::InvalidPublicKeyLength {
                expected: PK_BYTES,
                actual: self.sender.public_key.len(),
            });
        }
        if self.receiver.public_key.len() != PK_BYTES {
            return Err(PrimusError::InvalidPublicKeyLength {
                expected: PK_BYTES,
                actual: self.receiver.public_key.len(),
            });
        }

        if self.payload.requires_signature_verification() && self.signature.len() != SIG_BYTES {
            return Err(PrimusError::InvalidSignatureLength {
                expected: SIG_BYTES,
                actual: self.signature.len(),
            });
        }

        let fee_encoded = PhysicsCanon::encode(self.energy);
        if fee_encoded < PROTOCOL_MIN_FEE {
            return Err(PrimusError::FeeBelowMinimum {
                minimum: PROTOCOL_MIN_FEE,
                actual: fee_encoded,
            });
        }

        // Use signing_digest() here — in the current protocol version it is
        // identical to compute_reaction_hash(). The name matches what the
        // non-archived counterpart calls so diffs remain readable.
        let expected_hash = self.signing_digest();
        if self.reaction_hash != expected_hash {
            return Err(PrimusError::ReactionHashMismatch {
                expected: expected_hash,
                actual: self.reaction_hash,
            });
        }

        Ok(())
    }
}
