// =============================================================================
// primus-types/src/payload.rs
//
// WIRE FORMAT CONTRACT — VARIANT DISCRIMINANTS ARE FROZEN:
//
//   bincode v1 encodes enum variants as their 0-based declaration index,
//   serialized as u32 in little-endian. These discriminants ARE the protocol:
//
//     Generic       wire discriminant: 0x00000001 (LE: 00 00 00 00)
//     Transfer      wire discriminant: 0x00000001 (LE: 01 00 00 00)
//     MiningReward  wire discriminant: 0x00000002 (LE: 02 00 00 00)
//     Unknown       wire discriminant: catch-all (serde text formats only)
//
//   ADDITION RULE: New variants must be inserted BEFORE Unknown and AFTER
//   MiningReward. Unknown must remain the final serde-visible variant.
//   Document the new variant's wire discriminant in this comment block.
// =============================================================================

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;


/// The intent of a `SignedReaction` — what economic action the sender is
/// requesting the PVM to execute.
///
/// Each variant maps to a distinct execution path in `PVM::execute_payload()`.
/// The variant's wire discriminant (its 0-based declaration index) is part of
/// the protocol specification and must never change.
#[derive(
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    Default,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Zeroize,
)]

#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
#[repr(u32)]
pub enum Payload {
    /// No-op reaction. Used for genesis injection and debug tooling.
    /// The PVM charges the entropy tax but performs no mass transfer.
    /// Wire discriminant: 0x00000000
    #[default]
    Generic,

    /// Transfer `amount` mass from sender to receiver.
    /// Total cost to sender = `amount` + `energy` (fee).
    /// Wire discriminant: 0x00000001
    Transfer {
        /// Mass to transfer from sender.atom to receiver.atom.
        amount: u64,
    },

    /// Block reward. Constructed exclusively by `engine::build_mining_reward_rx()`.
    ///
    /// PVM verification steps:
    ///   1. Assert `receiver.public_key == architect_pk`.
    ///   2. Assert `reaction_hash == SHA3-256(MINING_REWARD_TAG || index || pk)`.
    ///   3. Credit `amount` mass to the Architect atom.
    ///   4. Skip entropy tax; skip signature verification.
    ///
    /// SECURITY: The self-authenticating reaction_hash makes this variant safe
    /// without a signature. Any forgery produces a different hash, yielding a
    /// different state root, which every peer rejects.
    /// Wire discriminant: 0x00000002
    MiningReward {
        /// Mass to credit to the Architect. Fixed at MINING_REWARD_AMOUNT per
        /// block. Variable in the struct for future epoch-based reward schedule
        /// changes without a wire format break.
        amount: u64,
    },

    /// Deploy a WASM contract.
    /// Wire discriminant: 0x00000003
    Contract {
        /// The WASM binary code. Max size enforced by PVM (1 MiB).
        code: Vec<u8>,
    },

    /// Call an existing WASM contract.
    /// Wire discriminant: 0x00000004
    ContractCall {
        /// The address (public key) of the atom hosting the contract.
        address: Vec<u8>,
        /// Input data for the contract execution.
        data: Vec<u8>,
    },

    /// Catch-all for unrecognized variants from newer network nodes.
    ///
    /// The PVM hard-rejects this variant. It exists so that a node running
    /// older software can deserialize (then reject) a reaction with a future
    /// payload type, rather than crashing on deserialization failure.
    ///
    /// BINCODE NOTE: `#[serde(other)]` applies only to text-format deserializers
    /// (JSON, CBOR, RON). bincode reads the discriminant directly and will never
    /// produce `Unknown` on deserialization; it would only appear if code
    /// explicitly constructs `Payload::Unknown` and serializes it.
    #[serde(other)]
    Unknown,
}

impl Payload {
    /// Returns the transfer amount for `Transfer` and `MiningReward` payloads.
    /// Returns `None` for `Generic`, `Contract`, `ContractCall`, and `Unknown`.
    ///
    /// Used by the PVM for mass accounting without pattern matching at call sites.
    pub fn transfer_amount(&self) -> Option<u64> {
        match self {
            Payload::Transfer { amount } | Payload::MiningReward { amount } => Some(*amount),
            _ => None,
        }
    }

    /// Returns `true` if this payload requires ML-DSA-87 signature verification.
    ///
    /// `MiningReward` is exempt (self-authenticating via reaction_hash).
    /// `Unknown` is also `false`, but only because the PVM rejects it before
    /// reaching the signature check — not because it is trusted.
    pub fn requires_signature_verification(&self) -> bool {
        !matches!(self, Payload::MiningReward { .. } | Payload::Unknown)
    }
}

impl ArchivedPayload {
    /// Zero-copy equivalent of `Payload::transfer_amount()`.
    pub fn transfer_amount(&self) -> Option<u64> {
        match self {
            ArchivedPayload::Transfer { amount } => Some(*amount),
            ArchivedPayload::MiningReward { amount } => Some(*amount),
            _ => None,
        }
    }

    /// Zero-copy equivalent of `Payload::requires_signature_verification()`.
    pub fn requires_signature_verification(&self) -> bool {
        !matches!(
            self,
            ArchivedPayload::MiningReward { .. } | ArchivedPayload::Unknown
        )
    }
}
