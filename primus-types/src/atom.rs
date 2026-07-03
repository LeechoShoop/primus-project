// =============================================================================
// primus-types/src/atom.rs
//
// WIRE FORMAT CONTRACT (bincode v1, little-endian):
//
//   Atom field order on the wire (FROZEN — do not reorder):
//     0:  public_key         Vec<u8>      length-prefixed bytes
//     1:  element            Element      u32 discriminant (see Element below)
//     2:  neutron_count      u32          LE
//     3:  mass               u64          LE
//     4:  charge             f32          IEEE 754 LE
//     5:  last_reaction_hash [u8; 32]     raw bytes, no length prefix
//     6:  last_active_index  u64          LE
//     7:  nonce              u64          LE
//     8:  quantum_state      QuantumState discriminant + optional payload
//
//   ADDING FIELDS: Append after field 8 with #[serde(default)].
//   NEVER reorder, rename, or remove existing fields — any such change is
//   a hard protocol break requiring a network-wide upgrade.
//   Always bump the protocol version constant when adding fields.
// =============================================================================

use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};


// ── Element ───────────────────────────────────────────────────────────────────

/// The elemental classification of an Atom, governing its base physics
/// parameters: initial charge, base mass, viscosity factor, and binding
/// potential.
///
/// BINCODE WIRE DISCRIMINANTS (frozen):
///   Hydrogen = 0   (default, lightest element, lowest binding potential)
///   Carbon   = 1
///   Oxygen   = 2
///   Gold     = 3   (heaviest, highest binding potential)
///
/// ORDERING RULE: Never insert between existing variants — doing so shifts
/// every subsequent variant's discriminant, silently corrupting all existing
/// serialized atoms. Additions must go at the end.
#[derive(
    Serialize,
    Deserialize,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Zeroize,
)]
#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
#[repr(u32)]
pub enum Element {
    #[default]
    Hydrogen = 0,
    Carbon = 1,
    Oxygen = 2,
    Gold = 3,
}

impl Element {
    /// The canonical initial charge for this element.
    /// Protocol constants — defined once so the SDK and Core initialize atoms
    /// identically. Do not derive these from floating-point arithmetic.
    pub fn initial_charge(self) -> f32 {
        match self {
            Element::Hydrogen => 2.20,
            Element::Carbon => 2.55,
            Element::Oxygen => 3.44,
            Element::Gold => 2.54,
        }
    }

    /// Base mass granted to a newly materialized atom of this element.
    pub fn base_mass(self) -> u64 {
        match self {
            Element::Hydrogen => 1_000,
            Element::Carbon => 6_000,
            Element::Oxygen => 4_000,
            Element::Gold => 50_000,
        }
    }
}

// ── QuantumState ──────────────────────────────────────────────────────────────

/// The quantum logic state of an Atom (Phase 3 / Intellect milestone).
///
/// BINCODE WIRE DISCRIMINANTS (frozen):
///   Stable        = 0   (no payload)
///   Entangled     = 1   + 32 bytes (partner atom ID)
///   Superposition = 2   + 8 bytes (u64 superposition seed)
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
    ZeroizeOnDrop,
)]
#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
#[repr(C)]
pub enum QuantumState {
    #[default]
    Stable,
    Entangled([u8; 32]),
    Superposition(u64),
}

// ── Atom ─────────────────────────────────────────────────────────────────────

/// The canonical on-chain identity and state of a Primus participant.
///
/// This is the Single Source of Truth replacing:
///   - primus-core::atom::PrimusAtom
///   - primus-sdk::transaction::AtomSnapshot
///
/// # Serialization formats
///
/// This struct derives both `serde` and `rkyv`. They serve different roles
/// and must NEVER be used interchangeably:
///
///   - **bincode (via serde)** — the P2P wire format and on-disk storage format.
///     Field order on the wire equals field declaration order (see module header).
///     This is the canonical format for cross-node communication.
///
///   - **rkyv** — zero-copy in-memory access for hot paths inside primus-core
///     (mempool scanning, PVM state tree). rkyv bytes are NEVER written to disk
///     or sent over the network. The `size_32` feature flag compacts rkyv
///     offsets to 32 bits; this affects only in-process memory layout and has
///     no effect on the bincode wire format.
///
/// Using rkyv output as a wire format, or bincode output for zero-copy access,
/// is a bug. If you are unsure which format to use, use bincode.
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
pub struct Atom {

    /// ML-DSA-87 verifying key bytes. Length must equal PK_BYTES (2592).
    pub public_key: Vec<u8>,

    /// Elemental classification. Governs viscosity, binding potential, charge.
    pub element: Element,

    /// Isotope neutron count. Non-zero values modify decay rate and binding.
    pub neutron_count: u32,

    /// Accumulated mass — the atom's economic value in mass units.
    pub mass: u64,

    /// Electronegativity charge. Set from protocol constants at creation;
    /// may evolve through the absorb_decay path.
    ///
    /// HASHING RULE: Never feed `charge.to_bits()` directly into a hash.
    /// Always encode via `PhysicsCanon::encode(charge)` first to guarantee
    /// cross-architecture determinism. See physics.rs for rationale.
    pub charge: f32,

    /// The reaction_hash of the most recently confirmed reaction this atom
    /// participated in. Anti-replay anchor for the next transaction.
    /// Initialized to [0u8; 32] for a newly materialized atom.
    pub last_reaction_hash: [u8; 32],

    /// The crystal (block) index at which this atom last participated in a
    /// reaction. Used for entropy decay calculation.
    pub last_active_index: u64,

    /// Sequential nonce. Incremented by the PVM after each confirmed reaction.
    pub nonce: u64,

    /// Quantum logic state. `#[serde(default)]` ensures atoms serialized
    /// before the quantum milestone deserialize as `QuantumState::Stable`.
    #[serde(default)]
    pub quantum_state: QuantumState,
}

impl Atom {
    /// Construct a new atom at zero mass with the given element.
    ///
    /// Used by the PVM when a Transfer targets an address that has never
    /// appeared on-chain (receiver materialization path).
    pub fn new_materialized(public_key: Vec<u8>, element: Element) -> Self {
        Self {
            charge: element.initial_charge(),
            public_key,
            element,
            neutron_count: 0,
            mass: 0,
            last_reaction_hash: [0u8; 32],
            last_active_index: 0,
            nonce: 0,
            quantum_state: QuantumState::Stable,
        }
    }

    /// Construct a full sender snapshot for embedding in a `SignedReaction`.
    ///
    /// All fields must be fetched from live chain state immediately before
    /// calling this. Stale `mass`, `last_reaction_hash`, `nonce`,
    /// `neutron_count`, or `last_active_index` values will fail the PVM's
    /// sequence and balance checks.
    ///
    /// # Arguments
    ///
    /// * `public_key`         — The sender's ML-DSA-87 verifying key (PK_BYTES).
    /// * `element`            — The sender's element (from on-chain state).
    /// * `mass`               — Current on-chain mass of the sender.
    /// * `last_reaction_hash` — Hash of the sender's most recent confirmed reaction.
    /// * `nonce`              — Current on-chain nonce of the sender.
    /// * `neutron_count`      — Current isotope neutron count (0 for standard atoms).
    /// * `last_active_index`  — Crystal index of the sender's last reaction.
    pub fn sender_snapshot(
        public_key: Vec<u8>,
        element: Element,
        mass: u64,
        last_reaction_hash: [u8; 32],
        nonce: u64,
        neutron_count: u32,
        last_active_index: u64,
    ) -> Self {
        Self {
            charge: element.initial_charge(),
            public_key,
            element,
            neutron_count,
            mass,
            last_reaction_hash,
            last_active_index,
            nonce,
            quantum_state: QuantumState::Stable,
        }
    }

    /// Construct a zero-mass receiver snapshot for a new (never-seen) address.
    ///
    /// The PVM auto-materializes atoms for receiver addresses with no existing
    /// on-chain state. Equivalent to `new_materialized(pk, Element::Hydrogen)`.
    pub fn new_receiver(public_key: Vec<u8>) -> Self {
        Self::new_materialized(public_key, Element::Hydrogen)
    }

    /// The first byte of the public key, used by the sectoral mempool and
    /// the galactic drift resonance check.
    ///
    /// Returns 0 for an empty `public_key` — this should never occur in a
    /// valid atom. Callers that need a hard guarantee should call
    /// `validate_structure()` before using this value.
    pub fn sector(&self) -> u8 {
        self.public_key.first().copied().unwrap_or(0)
    }
}
