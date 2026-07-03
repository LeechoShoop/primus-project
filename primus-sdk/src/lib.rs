// =============================================================================
// primus-sdk/src/lib.rs — Primus-Project SDK
//
// PUBLIC API DESIGN (Audit Finding 10):
//
//   Primary surface (stable, semver-protected, documented):
//     Wallet, TransactionBuilder, Transaction — the three types every consumer
//     needs. Import them from the crate root.
//
//   Wire-format types (stable, needed by node integrations):
//     AtomSnapshot, AtomElement, Payload, QuantumLogicSnapshot — exported
//     at the crate root because external code that constructs or inspects
//     on-chain state needs them.
//
//   Constants (stable):
//     PK_BYTES, SIG_BYTES, PROTOCOL_MIN_FEE — size guards and protocol values
//     that callers legitimately need without touching the internal modules.
//
//     SK_BYTES is intentionally NOT exported. External callers must never hold
//     raw signing-key bytes. All signing goes through Wallet::sign(), which
//     keeps the signing key as an ephemeral local variable inside crypto::sign_with_seed.
//
//   Internal / advanced (hidden from generated docs):
//     primus_sdk::internal — low-level crypto primitives (derive_seed, sha3_256,
//     keypair_from_seed, sign_with_seed, KeyPairBytes). Accessible to node
//     operators, test harnesses, and protocol implementors, but NOT part of
//     the stable public API. Callers that use `internal` accept that these
//     functions may change between minor versions.
//
// WHAT IS DELIBERATELY ABSENT:
//   - SK_BYTES: signing keys must never leave the Wallet.
//   - keypair_from_seed at the crate root: forces all keygen through Wallet.
//   - sign_with_seed at the crate root: same reason.
//   - derive_seed at the crate root: prevents ad-hoc derivation without the
//     PRIMUS_SDK domain separator.
// =============================================================================

pub mod crypto;
pub mod error;
pub mod transaction;
pub mod wallet;
pub mod proof;
pub mod proof_util;

// ── Primary API ───────────────────────────────────────────────────────────────

/// BIP-39 wallet with ML-DSA-87 post-quantum signing.
///
/// Create one with [`Wallet::generate`] (new key) or [`Wallet::from_mnemonic`]
/// (restore from backup), then persist it with [`Wallet::save`] /
/// [`Wallet::load`].
pub use wallet::Wallet;

/// Verify a balance proof against a trusted state root (WASM-safe).
pub use proof::verify_balance_proof;

/// Fluent builder for signed Transfer transactions.
///
/// Fetch on-chain state (mass, last_reaction_hash, nonce) from the node, then:
///
/// ```rust
/// use primus_sdk::{Wallet, TransactionBuilder, PROTOCOL_MIN_FEE};
///
/// # fn example(wallet: &Wallet, recipient_pk: Vec<u8>, mass: u64, last_hash: [u8; 32], nonce: u64) {
/// let tx = TransactionBuilder::new(wallet)
///     .recipient(recipient_pk)
///     .amount(1_000)
///     .sender_mass(mass)
///     .sender_last_hash(last_hash)
///     .sender_nonce(nonce)
///     .build()
///     .unwrap();
///
/// let bytes = tx.to_bytes().unwrap();
/// // → PrimusMessage::NewReaction(bytes, 10)
/// # }
/// ```
pub use transaction::TransactionBuilder;

/// A fully signed, bincode-serializable Primus reaction.
pub use transaction::Transaction;

// ── Wire-format types ─────────────────────────────────────────────────────────

/// Mirrors `primus-core/atom.rs Element`. Used in `AtomSnapshot` and
/// `TransactionBuilder::sender_element`.
pub use transaction::AtomElement;

/// Mirrors `primus-core/atom.rs QuantumLogic`. Embedded in `AtomSnapshot`.
pub use transaction::QuantumLogicSnapshot;

/// Minimal on-chain atom representation embedded in a `Transaction`.
pub use transaction::AtomSnapshot;

/// On-chain payload type. Used to inspect what a `Transaction` will do.
pub use transaction::Payload;

/// Full atom state (balance, nonce, element) from the blockchain.
pub use primus_types::Atom;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Byte length of an ML-DSA-87 verifying (public) key: 2592.
///
/// Use this to validate addresses and public-key slices from the network.
pub use crypto::PK_BYTES;

/// Byte length of an ML-DSA-87 signature: 4627.
///
/// Use this to validate signature slices from the network.
pub use crypto::SIG_BYTES;

/// Minimum network fee (in mass units) accepted by the PVM: 10.
///
/// Fees below this value are rejected by the node. Use as the default `fee`
/// argument to `TransactionBuilder::fee()`.
pub use transaction::PROTOCOL_MIN_FEE;

// ── Internal / advanced ───────────────────────────────────────────────────────

/// Low-level cryptographic primitives.
///
/// # ⚠️ Stability warning
///
/// Items in this module are NOT part of the stable public API of `primus-sdk`.
/// They may change or be removed in any minor version. Use only when you need
/// direct access to the ML-DSA-87 key derivation pipeline for node integration,
/// test harnesses, or protocol tooling.
///
/// Most consumers should use [`Wallet`] and [`TransactionBuilder`] instead.
#[doc(hidden)]
pub mod internal {
    /// Domain-separated 32-byte child seed derivation.
    pub use crate::crypto::derive_seed;
    /// Derive an ML-DSA-87 keypair from a 32-byte seed.
    pub use crate::crypto::keypair_from_seed;
    /// SHA3-256 hash of `data`.
    pub use crate::crypto::sha3_256;
    /// Sign `payload` by re-deriving the ML-DSA-87 signing key from `seed`.
    pub use crate::crypto::sign_with_seed;
    /// Verify an ML-DSA-87 signature. Returns `false` (not `Err`) on failure.
    pub use crate::crypto::verify;
    /// Return type of `keypair_from_seed`.
    pub use crate::crypto::KeyPairBytes;
    /// Raw ML-DSA-87 signing-key byte length (4896).
    ///
    /// Exposed here only for size assertions in node integration code.
    /// Never use this to hold or transmit raw signing-key bytes.
    pub use crate::crypto::SK_BYTES;
}
