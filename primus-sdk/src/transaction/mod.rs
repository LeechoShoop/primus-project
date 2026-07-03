/// =============================================================================
// primus-sdk/src/transaction/mod.rs — Transaction Construction & Signing
//
// RDIV-001 FIX (bincode → rkyv wire format):
//   primus-core/src/main.rs::CoreHandleImpl::push_bytes() validates inbound
//   NewReaction payloads with rkyv::check_archived_root::<SignedReaction>.
//   This rejects any non-rkyv payload with MalformedFrame, making every
//   bincode-encoded SDK transaction silently dropped at the ingress boundary.
//
//   Fix: Transaction::to_bytes() now converts the local Transaction snapshot
//   into a primus_types::SignedReaction and serializes it with rkyv, exactly
//   matching the format that primus-core expects.
//
//   Alignment: rkyv produces an AlignedVec whose payload portion is correctly
//   aligned for check_archived_root even when the enclosing length-prefixed
//   TCP frame is read into an unaligned buffer. This is safe because
//   check_archived_root performs its own alignment check internally.
//
// PROTOCOL COMPATIBILITY:
//   Field layout and serde tags MUST match primus-core/kinetic.rs exactly so
//   that bincode-serialised transactions deserialise correctly on the node.
//   Any change to field order, type, or serde attributes here must be mirrored
//   in the node's `ReactionResult` and `Payload` types.
//
// CRITICAL SIGNING FIX (Audit Finding 7):
//   Previous revision signed `sender_pk ++ sender_last_reaction_hash`.
//   The PVM (pvm.rs) verifies signatures over `rx.reaction_hash` — the
//   SHA3-256 hash of the full transfer data. Signing the wrong message made
//   every SDK-built transaction permanently rejected with "Signature REJECTED".
//
//   Correct signing convention (now implemented):
//     reaction_hash = SHA3-256(sender_pk ++ recipient_pk ++ amount_le ++ fee_le ++ last_hash)
//     signature     = ML-DSA-87.sign(signing_key, reaction_hash)
//
//   This matches kinetic.rs::build_transfer exactly and satisfies the PVM check:
//     Crypto::verify(&rx.sender.public_key, &rx.reaction_hash, &rx.signature)
//
// STRICT MODE (Audit Finding 8):
//   All mirrored Core structs carry `#[serde(deny_unknown_fields)]`.
//   If primus-core adds a field to PrimusAtom, Element, or Payload without a
//   matching update here, deserialization will fail loudly rather than silently
//   dropping the new field and producing a different bincode layout.
//
// OTHER CHANGES vs previous revision:
//   Finding 9  — PROTOCOL_MIN_FEE constant replaces the magic number 10.
//   General    — `send` field renamed to `amount` in TransferRequest for clarity.
// =============================================================================
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use sha3::{Digest, Sha3_256};
use crate::wallet::Wallet;

// ── Protocol constant ─────────────────────────────────────────────────────────

/// Minimum network fee (in mass units) accepted by the PVM.
///
/// The fee is burned by the protocol — it is NOT credited to any receiver.
/// All CLI code and SDK callers should use this constant rather than hard-coding
/// `10`, so a protocol upgrade requires only one change here.
pub const PROTOCOL_MIN_FEE: u64 = 10;

// ── Mirrored Core types ───────────────────────────────────────────────────────
//
// These types mirror primus-core structs for bincode wire compatibility.
// The `deny_unknown_fields` attribute (Finding 8) ensures that a Core struct
// gaining a new field is caught immediately at deserialization time rather
// than silently producing a different bincode layout.

/// Mirrors `primus-core/atom.rs Element`.
///
/// ⚠️ Field order and variant names MUST stay in sync with the Core.
///    `deny_unknown_fields` catches additions but not removals — always review
///    both files when changing either.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub enum AtomElement {
    #[default]
    Hydrogen,
    Carbon,
    Oxygen,
    Gold,
}

/// Mirrors `primus-core/atom.rs QuantumLogic`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub enum QuantumLogicSnapshot {
    #[default]
    Stable,
    Entangled([u8; 32]),
    Superposition(u64),
}

/// A minimal on-chain atom snapshot for embedding in transactions.
///
/// Mirrors `primus-core/atom.rs PrimusAtom`. The node uses its own `StateTree`
/// for authoritative balance checks; this snapshot exists only for the PVM's
/// sequence-number and signature verification.
///
/// ⚠️ `deny_unknown_fields` is intentional — see module-level comment.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct AtomSnapshot {
    pub public_key: Vec<u8>,
    pub element: AtomElement,
    pub neutron_count: u32,
    pub mass: u64,
    pub charge: f32,
    pub last_reaction_hash: [u8; 32],
    pub last_active_index: u64,
    pub nonce: u64,
    #[serde(default)]
    pub quantum_state: QuantumLogicSnapshot,
}

impl AtomSnapshot {
    /// Build a sender snapshot from the live on-chain fields.
    ///
    /// `charge` is derived from `element` using the same constants as the Core
    /// so the bincode layout is identical.
    pub fn sender(
        public_key: Vec<u8>,
        mass: u64,
        last_reaction_hash: [u8; 32],
        nonce: u64,
        element: AtomElement,
    ) -> Self {
        let charge = match element {
            AtomElement::Hydrogen => 2.20,
            AtomElement::Carbon => 2.55,
            AtomElement::Oxygen => 3.44,
            AtomElement::Gold => 2.54,
        };
        Self {
            public_key,
            element,
            neutron_count: 0,
            mass,
            charge,
            last_reaction_hash,
            last_active_index: 0,
            nonce,
            quantum_state: QuantumLogicSnapshot::Stable,
        }
    }

    /// Build a zero-mass receiver snapshot.
    ///
    /// Atoms that do not yet exist on-chain are auto-materialized by the PVM
    /// at `mass = 0`. Passing a freshly constructed snapshot with `mass = 0`
    /// is the correct way to represent a new receiver.
    pub fn new_receiver(public_key: Vec<u8>) -> Self {
        Self {
            public_key,
            element: AtomElement::Hydrogen,
            neutron_count: 0,
            mass: 0,
            charge: 2.20,
            last_reaction_hash: [0u8; 32],
            last_active_index: 0,
            nonce: 0,
            quantum_state: QuantumLogicSnapshot::Stable,
        }
    }
}

// ── Payload ───────────────────────────────────────────────────────────────────

/// Mirrors `primus-core/kinetic.rs Payload`.
///
/// ORDERING RULE: `Unknown` MUST be last so `#[serde(other)]` catches any
/// future variants received from the network without breaking deserialization.
///
/// NOTE: `MiningReward` is intentionally absent from the SDK. It is constructed
/// exclusively by the Core's `engine::build_mining_reward_rx()` and is exempt
/// from ML-DSA verification. SDK clients must never construct it; doing so
/// would produce a different state root and be rejected by every peer.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub enum Payload {
    /// No-op / genesis injection.
    #[default]
    Generic,

    /// Transfer `amount` mass from sender to receiver.
    Transfer { amount: u64 },

    /// Catch-all for unknown future variants — always keep last.
    #[serde(other)]
    Unknown,
}

// ── Transaction ───────────────────────────────────────────────────────────────

/// A fully signed Primus reaction, ready for network broadcast.
///
/// # Broadcasting
///
/// Serialize with `bincode::serialize` and wrap in a
/// `PrimusMessage::NewReaction(bytes, ttl)` before sending to a node.
///
/// ```rust
/// use primus_sdk::{Wallet, TransactionBuilder, PROTOCOL_MIN_FEE};
///
/// # fn run() -> Result<(), anyhow::Error> {
/// let wallet    = Wallet::generate(12, 0).unwrap();
/// let recipient = vec![0u8; 2592]; // real ML-DSA-87 pubkey from address book
///
/// let tx    = TransactionBuilder::new(&wallet)
///     .recipient(recipient)
///     .amount(1_000)
///     .sender_mass(500_000)
///     .sender_last_hash([0u8; 32])
///     .build()
///     .unwrap();
///
/// let bytes = tx.to_bytes().unwrap();
/// // → PrimusMessage::NewReaction(bytes, 10)
/// # Ok(()) }
/// ```
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Transaction {
    pub sender: AtomSnapshot,
    pub receiver: AtomSnapshot,
    pub reaction_hash: [u8; 32],
    /// Network fee in mass units (burned by the network, not sent to receiver).
    pub energy: f32,
    pub timestamp: u64,
    pub signature: Vec<u8>,
    #[serde(default)]
    pub payload: Payload,
}

impl Transaction {
    /// Serialize to rkyv bytes for network transmission via `PrimusMessage::NewReaction`.
    ///
    /// # Wire format (RDIV-001)
    ///
    /// Returns rkyv-encoded `primus_types::SignedReaction` bytes. This is the
    /// format expected by `primus-core::CoreHandleImpl::push_bytes()`, which
    /// gates every inbound reaction with `rkyv::check_archived_root::<SignedReaction>`.
    ///
    /// # Conversion
    ///
    /// The SDK's local `Transaction` snapshot is first converted into a
    /// `primus_types::SignedReaction` (the canonical on-wire type), then
    /// rkyv-encoded. This guarantees that the produced bytes pass the core's
    /// structural validation gate.
    ///
    /// # Errors
    ///
    /// Returns `Err` if rkyv serialization fails (allocator failure only;
    /// structurally valid `SignedReaction` values always serialize successfully).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        use primus_types::{
            atom::{Atom, Element, QuantumState},
            payload::Payload as TypesPayload,
            reaction::SignedReaction,
        };

        // ── Map SDK AtomElement → primus_types::Element ───────────────────────────────────
        let map_element = |e: &AtomElement| -> Element {
            match e {
                AtomElement::Hydrogen => Element::Hydrogen,
                AtomElement::Carbon   => Element::Carbon,
                AtomElement::Oxygen   => Element::Oxygen,
                AtomElement::Gold     => Element::Gold,
            }
        };

        // ── Map SDK QuantumLogicSnapshot → primus_types::QuantumState ───────────────
        let map_quantum = |q: &QuantumLogicSnapshot| -> QuantumState {
            match q {
                QuantumLogicSnapshot::Stable            => QuantumState::Stable,
                QuantumLogicSnapshot::Entangled(arr)    => QuantumState::Entangled(*arr),
                QuantumLogicSnapshot::Superposition(n)  => QuantumState::Superposition(*n),
            }
        };

        // ── Build primus_types::Atom from AtomSnapshot ───────────────────────────
        let make_atom = |snap: &AtomSnapshot| -> Atom {
            Atom {
                public_key:         snap.public_key.clone(),
                element:            map_element(&snap.element),
                neutron_count:      snap.neutron_count,
                mass:               snap.mass,
                charge:             snap.charge,
                last_reaction_hash: snap.last_reaction_hash,
                last_active_index:  snap.last_active_index,
                nonce:              snap.nonce,
                quantum_state:      map_quantum(&snap.quantum_state),
            }
        };

        // ── Map SDK Payload → primus_types::Payload ──────────────────────────────
        let types_payload: TypesPayload = match &self.payload {
            Payload::Generic              => TypesPayload::Generic,
            Payload::Transfer { amount }  => TypesPayload::Transfer { amount: *amount },
            Payload::Unknown              => TypesPayload::Generic, // degrade gracefully
        };

        // ── Assemble primus_types::SignedReaction ────────────────────────────────
        let signed_reaction = SignedReaction {
            sender:         make_atom(&self.sender),
            receiver:       make_atom(&self.receiver),
            reaction_hash:  self.reaction_hash,
            energy:         self.energy,
            timestamp:      self.timestamp,
            signature:      self.signature.clone(),
            payload:        types_payload,
        };

        // ── rkyv serialization (RDIV-001) ──────────────────────────────────────
        // primus-core::push_bytes() calls rkyv::check_archived_root::<SignedReaction>
        // on these bytes. rkyv 0.7 with size_32 features must match primus-types exactly.
        //
        // AlignedVec is converted to Vec<u8> so callers receive a plain byte buffer;
        // alignment is no longer a concern once the bytes enter the TCP frame because
        // check_archived_root handles the alignment internally via the bytecheck pass.
        let aligned = rkyv::to_bytes::<_, 256>(&signed_reaction)
            .map_err(|e| anyhow!("Transaction rkyv serialization failed: {}", e))?;
        Ok(aligned.into_vec())
    }

    /// Hex-encode the serialized transaction for debugging and logging.
    pub fn to_hex(&self) -> Result<String> {
        Ok(hex::encode(self.to_bytes()?))
    }
}

// ── TransactionBuilder ────────────────────────────────────────────────────────

/// Fluent builder for constructing signed Primus Transfer transactions.
///
/// # Required fields
///
/// | Method                | Source                              |
/// |-----------------------|-------------------------------------|
/// | `recipient(pk)`       | Address book or node atom endpoint  |
/// | `amount(n)`           | User input                          |
/// | `sender_mass(n)`      | Node atom endpoint (live state)     |
/// | `sender_last_hash(h)` | Node atom endpoint (live state)     |
///
/// Defaults: `fee = PROTOCOL_MIN_FEE`, `nonce = 0`, `element = Hydrogen`.
///
/// # Thin-client usage
///
/// Fetch on-chain state first, then build:
///
/// ```rust
/// use primus_sdk::{Wallet, TransactionBuilder};
///
/// # fn example(recipient_pk_bytes: Vec<u8>, on_chain_mass: u64, on_chain_last_hash: [u8; 32], on_chain_nonce: u64) {
/// # let wallet = Wallet::generate(12, 0).unwrap();
/// // (Fetch mass, last_hash, nonce from the node RPC)
/// let tx = TransactionBuilder::new(&wallet)
///     .recipient(recipient_pk_bytes)
///     .amount(1_000)
///     .sender_mass(on_chain_mass)
///     .sender_last_hash(on_chain_last_hash)
///     .sender_nonce(on_chain_nonce)
///     .build()
///     .unwrap();
/// # }
/// ```
#[derive(Clone)]
pub struct TransactionBuilder<'w> {
    wallet: &'w Wallet,
    recipient_pk: Option<Vec<u8>>,
    amount: u64,
    fee: u64,
    sender_mass: u64,
    sender_last_hash: [u8; 32],
    sender_nonce: u64,
    sender_element: AtomElement,
}

impl<'w> TransactionBuilder<'w> {
    /// Create a builder for `wallet`.
    pub fn new(wallet: &'w Wallet) -> Self {
        Self {
            wallet,
            recipient_pk: None,
            amount: 0,
            fee: PROTOCOL_MIN_FEE,
            sender_mass: 0,
            sender_last_hash: [0u8; 32],
            sender_nonce: 0,
            sender_element: AtomElement::Hydrogen,
        }
    }

    /// Set the recipient's ML-DSA-87 public key bytes (2592 bytes / `PK_BYTES`).
    ///
    /// Use `Wallet::decode_address(hex_str)` to convert a hex address string
    /// into the raw bytes expected here.
    pub fn recipient(mut self, pk: Vec<u8>) -> Self {
        self.recipient_pk = Some(pk);
        self
    }

    /// Set the amount of mass to transfer.
    pub fn amount(mut self, amount: u64) -> Self {
        self.amount = amount;
        self
    }

    /// Override the network fee (default: `PROTOCOL_MIN_FEE`).
    ///
    /// Fees below `PROTOCOL_MIN_FEE` are rejected by the PVM.
    /// Setting a higher fee does not increase transaction priority in the
    /// current mempool implementation but may in future versions.
    pub fn fee(mut self, fee: u64) -> Self {
        self.fee = fee;
        self
    }

    /// Set the sender's current on-chain mass.
    ///
    /// Fetch the live value from the node's atom endpoint before building.
    /// Using a stale value will cause the PVM's `InsufficientMass` check to
    /// fail or succeed incorrectly.
    pub fn sender_mass(mut self, mass: u64) -> Self {
        self.sender_mass = mass;
        self
    }

    /// Set the sender's `last_reaction_hash` from on-chain state.
    ///
    /// This field binds the signature to the sender's current position in the
    /// reaction chain (anti-replay). It MUST match the node's on-chain value
    /// exactly — a mismatch produces a `Sequence Mismatch` error at the PVM.
    pub fn sender_last_hash(mut self, hash: [u8; 32]) -> Self {
        self.sender_last_hash = hash;
        self
    }

    /// Set the sender's current nonce (increments with each confirmed reaction).
    pub fn sender_nonce(mut self, nonce: u64) -> Self {
        self.sender_nonce = nonce;
        self
    }

    /// Override the sender's element type (default: Hydrogen).
    ///
    /// The element affects the charge field embedded in the `AtomSnapshot`.
    /// Fetch the real element from the node if the atom has evolved beyond
    /// its initial Hydrogen state.
    pub fn sender_element(mut self, element: AtomElement) -> Self {
        self.sender_element = element;
        self
    }

    /// Build and sign the transaction.
    ///
    /// # Signing convention (matches pvm.rs)
    ///
    /// ```text
    /// reaction_hash = SHA3-256(sender_pk ++ recipient_pk ++ amount_le8 ++ fee_le8 ++ last_hash)
    /// signature     = ML-DSA-87.sign(signing_key, reaction_hash)
    /// ```
    ///
    /// The PVM verifies:
    /// ```text
    /// ML-DSA-87.verify(sender_pk, reaction_hash, signature) == true
    /// ```
    ///
    /// # Errors
    ///
    /// - `recipient` was not set.
    /// - `amount == 0`.
    /// - `fee < PROTOCOL_MIN_FEE`.
    /// - `amount + fee` overflows `u64`.
    /// - `sender_mass < amount + fee` (insufficient balance — pre-flight check).
    pub fn build(self) -> Result<Transaction> {
        // ── Guard: recipient required ─────────────────────────────────────────
        let recipient_pk = self
            .recipient_pk
            .ok_or_else(|| anyhow!("TransactionBuilder: recipient public key is required"))?;

        // ── Guard: amount > 0 ─────────────────────────────────────────────────
        if self.amount == 0 {
            return Err(anyhow!("TransactionBuilder: amount must be greater than 0"));
        }

        // ── Guard: fee >= protocol minimum ────────────────────────────────────
        if self.fee < PROTOCOL_MIN_FEE {
            return Err(anyhow!(
                "TransactionBuilder: fee {} is below PROTOCOL_MIN_FEE {}.",
                self.fee,
                PROTOCOL_MIN_FEE
            ));
        }

        // ── Guard: mass sufficiency (mirrors pvm.rs InsufficientMass check) ───
        let total_cost = self
            .amount
            .checked_add(self.fee)
            .ok_or_else(|| anyhow!("TransactionBuilder: amount + fee overflows u64"))?;
        if self.sender_mass < total_cost {
            return Err(anyhow!(
                "TransactionBuilder: insufficient sender mass — has {}, needs {} \
                 ({} amount + {} fee).",
                self.sender_mass,
                total_cost,
                self.amount,
                self.fee
            ));
        }

        let sender_pk = self.wallet.get_public_key_bytes();

        let map_element = |e: &AtomElement| -> primus_types::atom::Element {
            match e {
                AtomElement::Hydrogen => primus_types::atom::Element::Hydrogen,
                AtomElement::Carbon   => primus_types::atom::Element::Carbon,
                AtomElement::Oxygen   => primus_types::atom::Element::Oxygen,
                AtomElement::Gold     => primus_types::atom::Element::Gold,
            }
        };

        // ── Canonical SignedReaction for hashing (RDIV-002 fix) ───────────────
        // We construct a temporary primus_types::SignedReaction to use its
        // canonical compute_reaction_hash() and signing_digest() methods.
        // This ensures PhysicsCanon::encode() is used for the fee, matching the node.
        let types_rx = primus_types::reaction::SignedReaction {
            sender: primus_types::atom::Atom {
                public_key:         sender_pk.clone(),
                element:            map_element(&self.sender_element),
                neutron_count:      0, // not used in hash
                mass:               self.sender_mass,
                charge:             0.0, // not used in hash
                last_reaction_hash: self.sender_last_hash,
                last_active_index:  0,
                nonce:              self.sender_nonce,
                quantum_state:      primus_types::atom::QuantumState::Stable,
            },
            receiver: primus_types::atom::Atom {
                public_key:         recipient_pk.clone(),
                element:            primus_types::atom::Element::Hydrogen,
                neutron_count:      0,
                mass:               0,
                charge:             0.0,
                last_reaction_hash: [0u8; 32],
                last_active_index:  0,
                nonce:              0,
                quantum_state:      primus_types::atom::QuantumState::Stable,
            },
            reaction_hash:  [0u8; 32],
            energy:         self.fee as f32,
            timestamp:      0, // not used in hash
            signature:      vec![],
            payload:        primus_types::payload::Payload::Transfer { amount: self.amount },
        };

        let reaction_hash = types_rx.compute_reaction_hash();
        let signing_digest = types_rx.signing_digest();

        // ── Signature (CRITICAL FIX — Finding 7) ──────────────────────────────
        //
        // Sign the signing_digest directly.
        //
        // Previous (broken) code signed `sender_pk ++ sender_last_hash` or
        // the unscaled reaction_hash, causing "ReactionHashMismatch" or
        // "Signature REJECTED" at the PVM.
        let signature = self.wallet.sign(&signing_digest);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow!("System clock error: {}", e))?
            .as_secs();

        let sender = AtomSnapshot::sender(
            sender_pk,
            self.sender_mass,
            self.sender_last_hash,
            self.sender_nonce,
            self.sender_element,
        );
        let receiver = AtomSnapshot::new_receiver(recipient_pk);

        Ok(Transaction {
            sender,
            receiver,
            reaction_hash,
            energy: self.fee as f32,
            timestamp,
            signature,
            payload: Payload::Transfer {
                amount: self.amount,
            },
        })
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::Wallet;

    fn large_stack<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> T {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    fn make_wallets() -> (Wallet, Wallet) {
        let sender = Wallet::generate(12, 0).unwrap();
        let recipient = Wallet::generate(12, 1).unwrap();
        (sender, recipient)
    }

    // ── Happy path ────────────────────────────────────────────────────────────

    #[test]
    fn build_transfer_succeeds() {
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let tx = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(500)
                .fee(PROTOCOL_MIN_FEE)
                .sender_mass(100_000)
                .sender_last_hash([0u8; 32])
                .sender_nonce(0)
                .build()
                .expect("build() failed");

            assert_eq!(tx.energy, PROTOCOL_MIN_FEE as f32);
            matches!(&tx.payload, Payload::Transfer { amount } if *amount == 500);
        });
    }

    // ── Signing convention (Critical Fix — Finding 7) ─────────────────────────

    #[test]
    fn signature_is_over_reaction_hash_not_pk_concat() {
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let last_hash = [0xABu8; 32];

            let tx = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(100)
                .sender_mass(50_000)
                .sender_last_hash(last_hash)
                .build()
                .unwrap();

            // The PVM verifies: verify(sender_pk, reaction_hash, signature)
            let verified_over_hash = sender.verify(&tx.reaction_hash, &tx.signature);
            assert!(
                verified_over_hash,
                "signature must verify over reaction_hash — this is what pvm.rs checks"
            );

            // The old broken path (pk ++ last_hash) must NOT verify the signature.
            let mut old_msg = sender.get_public_key_bytes();
            old_msg.extend_from_slice(&last_hash);
            let verified_over_old_msg = sender.verify(&old_msg, &tx.signature);
            assert!(
                !verified_over_old_msg,
                "signature must NOT verify over old pk++last_hash message (that was the bug)"
            );
        });
    }

    #[test]
    fn signature_verifies_locally_as_pvm_would() {
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let last_hash = [0u8; 32];

            let tx = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(200)
                .fee(PROTOCOL_MIN_FEE)
                .sender_mass(50_000)
                .sender_last_hash(last_hash)
                .build()
                .unwrap();

            // Reproduce the PVM's exact verification call.
            let pk_bytes = sender.get_public_key_bytes();
            assert!(
                crate::crypto::verify(&pk_bytes, &tx.reaction_hash, &tx.signature),
                "crypto::verify(sender_pk, reaction_hash, sig) must pass — \
                 this is what pvm.rs executes"
            );
        });
    }

    // ── Reaction hash construction ────────────────────────────────────────────

    #[test]
    fn reaction_hash_matches_kinetic_rs_formula() {
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let amount = 777u64;
            let fee = PROTOCOL_MIN_FEE;
            let last_hash = [0x11u8; 32];

            let tx = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(amount)
                .fee(fee)
                .sender_mass(100_000)
                .sender_last_hash(last_hash)
                .build()
                .unwrap();

            // Reproduce the hash manually, matching kinetic.rs::build_transfer.
            let mut expected_data = sender.get_public_key_bytes();
            expected_data.extend_from_slice(&recipient.get_public_key_bytes());
            expected_data.extend_from_slice(&amount.to_le_bytes());
            expected_data.extend_from_slice(&primus_types::physics::PhysicsCanon::encode(fee as f32).to_le_bytes());
            expected_data.extend_from_slice(&last_hash);
            let expected_hash: [u8; 32] = Sha3_256::digest(&expected_data).into();

            assert_eq!(
                tx.reaction_hash,
                expected_hash,
                "reaction_hash must match the kinetic.rs formula exactly"
            );
        });
    }

    // ── Guard validations ─────────────────────────────────────────────────────

    #[test]
    fn missing_recipient_is_rejected() {
        large_stack(|| {
            let (sender, _) = make_wallets();
            let result = TransactionBuilder::new(&sender)
                .amount(100)
                .sender_mass(50_000)
                .build();
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("recipient"));
        });
    }

    #[test]
    fn zero_amount_is_rejected() {
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let result = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(0)
                .sender_mass(50_000)
                .build();
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("amount must be greater than 0"));
        });
    }

    #[test]
    fn fee_below_minimum_is_rejected() {
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let result = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(100)
                .fee(PROTOCOL_MIN_FEE - 1)
                .sender_mass(50_000)
                .build();
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("PROTOCOL_MIN_FEE"));
        });
    }

    #[test]
    fn insufficient_mass_is_rejected() {
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let result = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(99_999)
                .sender_mass(50) // way too low
                .build();
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("insufficient sender mass"));
        });
    }

    // ── Serialisation ─────────────────────────────────────────────────────────

    #[test]
    fn transaction_round_trips_rkyv() {
        use primus_types::reaction::SignedReaction;
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let tx = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(100)
                .sender_mass(50_000)
                .sender_last_hash([0u8; 32])
                .build()
                .unwrap();

            let bytes: Vec<u8> = tx.to_bytes().unwrap();

            // Verify the bytes pass primus-core's ingress gate exactly.
            let archived = rkyv::check_archived_root::<SignedReaction>(&bytes)
                .expect("rkyv structural check must pass (RDIV-001)");
            assert_eq!(
                archived.reaction_hash.as_slice(),
                &tx.reaction_hash,
                "round-trip must preserve reaction_hash"
            );
            assert_eq!(
                archived.signature.len(),
                tx.signature.len(),
                "round-trip must preserve signature length"
            );
        });
    }

    #[test]
    fn default_fee_is_protocol_minimum() {
        large_stack(|| {
            let (sender, recipient) = make_wallets();
            let tx = TransactionBuilder::new(&sender)
                .recipient(recipient.get_public_key_bytes())
                .amount(100)
                .sender_mass(50_000)
                .build()
                .unwrap();
            assert_eq!(tx.energy, PROTOCOL_MIN_FEE as f32);
        });
    }
}
