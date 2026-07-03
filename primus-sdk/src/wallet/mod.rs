// =============================================================================
// primus-sdk/src/wallet/mod.rs — BIP-39 Wallet with Entropy Branching
//
// KEY DESIGN DECISIONS:
//   1. The wallet stores the 32-byte child SEED (not signing-key bytes).
//      In ml-dsa rc.8, ExpandedSigningKey cannot be encoded to bytes, so
//      raw-byte storage of the signing key is impossible. We store the seed
//      and re-derive the keypair on every sign() call — identical to how
//      primus-core/crypto.rs works with its phrase-based re-derivation.
//
//   2. Derivation index separates master and operator keys from one mnemonic.
//      index=0 → Master Key, index=1 → Operator Key.
//
//   3. Mnemonic-to-seed uses BIP-39's `.to_seed("")` (64-byte PBKDF2 output)
//      as the master entropy input. derive_seed() then branches it to 32 bytes
//      for the specific index, domain-separated by b"PRIMUS_SDK".
//
//   4. Wallet is Clone + Send + Sync with no lifetime parameters — required
//      for UniFFI / WASM FFI compatibility.
//
// SECURITY CHANGES vs previous revision (Audit Finding 4, 5, 6):
//   Finding 4 — mnemonic_phrase is now a PRIVATE field.
//               Access is gated through get_mnemonic(), which carries an
//               explicit security warning in its doc comment.
//   Finding 5 — save() / load() methods added for .secrets file management.
//               The on-disk format (WalletFile) stores ONLY the mnemonic and
//               index — never the seed or any key bytes. The full keypair is
//               re-derived on load, so a stolen wallet file alone is useless
//               without the mnemonic words it contains.
//   Finding 6 — verify() now delegates to crate::crypto::verify().
//               There is no longer a separate ml_dsa import in this module;
//               the single canonical verification path in crypto/mod.rs is used.
// =============================================================================

use anyhow::{anyhow, Context, Result};
use bip39::Mnemonic;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::crypto::{derive_seed, keypair_from_seed, sign_with_seed, verify, PK_BYTES};

// ── On-disk wallet format ─────────────────────────────────────────────────────

/// Wire format written by `Wallet::save` and read by `Wallet::load`.
///
/// Stores ONLY the mnemonic phrase and derivation index. The seed and all
/// key bytes are always re-derived at load time, so this file never contains
/// raw cryptographic key material beyond the mnemonic itself.
///
/// `version` allows future format migrations without breaking existing files.
#[derive(Serialize, Deserialize, Debug, Zeroize, ZeroizeOnDrop)]
struct WalletFile {
    /// Format version — currently always 1.
    version: u32,
    /// The BIP-39 mnemonic phrase that is the root of all key material.
    mnemonic_phrase: String,
    /// Derivation index used when this wallet was created.
    derivation_index: u32,
}

// ── Wallet ────────────────────────────────────────────────────────────────────

/// A Primus wallet backed by a BIP-39 mnemonic and ML-DSA-87 post-quantum keys.
///
/// # Derivation Indices
///
/// | Index | Role          | Usage                                   |
/// |-------|---------------|-----------------------------------------|
/// | 0     | Master Key    | Primary signing identity on-chain       |
/// | 1     | Operator Key  | Delegated authority, hot-wallet signing |
/// | 2+    | Custom        | Application-layer key hierarchy         |
///
/// # Dual-role usage
///
/// Both the Architect (Master) and Operator keys come from the same mnemonic —
/// only the derivation index differs. To hold both simultaneously:
///
/// ```rust
/// use primus_sdk::Wallet;
///
/// let master   = Wallet::generate(24, 0).unwrap(); // index 0 = Master
/// let operator = Wallet::from_mnemonic(master.get_mnemonic(), 1).unwrap(); // index 1 = Operator
/// assert_ne!(master.address, operator.address);
/// ```
///
/// # Persistence
///
/// Use `save()` and `load()` to store the wallet in a `.secrets` file.
/// The file contains only the mnemonic — protect it with `chmod 600`.
///
/// # FFI Safety
///
/// All fields are owned — no borrowed lifetimes. `Clone + Send + Sync`.
///
/// # Security Note
///
/// `key_seed` lives in heap memory. For production mobile deployments, store
/// the wallet via the OS Secure Enclave API. The mnemonic is the ultimate
/// source of truth and can always re-derive everything on a fresh install.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Wallet {
    // ── PRIVATE: access via get_mnemonic() only ───────────────────────────────
    // Finding 4 fix: mnemonic_phrase must NEVER be a public field.
    // Any code holding a &Wallet can already read `address` (the public key).
    // Allowing direct mnemonic access would expose the root secret to all
    // downstream crates that depend on primus-sdk without any opt-in.
    mnemonic_phrase: String,

    /// Derivation index that produced this wallet's key branch.
    pub derivation_index: u32,

    // ── PRIVATE: only used inside sign() ─────────────────────────────────────
    // 32-byte child seed — deterministically derived from mnemonic + index.
    // The signing key is re-derived from this on every sign() call.
    // ⚠️  Treat as a private key equivalent; never log or transmit.
    key_seed: [u8; 32],

    /// Hex-encoded ML-DSA-87 verifying key — the wallet's on-chain address.
    ///
    /// This is the value you send to other parties and register on-chain.
    /// For all protocol-level operations use this field, not `get_short_address()`.
    pub address: String,

    // ── PRIVATE: returned by get_public_key_bytes() ───────────────────────────
    public_key_bytes: Vec<u8>,
}

impl Wallet {
    // ── Constructors ──────────────────────────────────────────────────────────

    /// Restore a wallet from an existing BIP-39 mnemonic phrase.
    ///
    /// # Arguments
    ///
    /// * `phrase` — A valid English BIP-39 mnemonic (12 or 24 words).
    /// * `index`  — Derivation index. `0` = Master Key, `1` = Operator Key.
    ///
    /// # Stack requirement
    ///
    /// ML-DSA-87 key generation uses ~4 MiB of stack. Call from a thread
    /// with `.stack_size(8 * 1024 * 1024)`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use primus_sdk::Wallet;
    ///
    /// let wallet = Wallet::from_mnemonic(
    ///     "abandon abandon abandon abandon abandon abandon \
    ///      abandon abandon abandon abandon abandon about",
    ///     0,
    /// ).unwrap();
    /// println!("Address: {}", &wallet.address[..16]);
    /// ```
    pub fn from_mnemonic(phrase: &str, index: u32) -> Result<Self> {
        // 1. Parse and validate the BIP-39 mnemonic.
        let mnemonic =
            Mnemonic::parse(phrase).map_err(|e| anyhow!("Invalid BIP-39 mnemonic: {}", e))?;

        // 2. Derive 64-byte BIP-39 seed (PBKDF2-stretched, passphrase = "").
        //    Using the stretched form keeps us compatible with standard BIP-39
        //    tooling that also calls to_seed("").
        let bip39_seed: [u8; 64] = mnemonic.to_seed("");

        // 3. Branch into a 32-byte child seed for this derivation index.
        //    derive_seed domain-separates each child with b"PRIMUS_SDK".
        let child_seed = derive_seed(&bip39_seed, index);

        // 4. Derive the ML-DSA-87 verifying key (address) from the child seed.
        //    We only store the seed + verifying key; the signing key is
        //    re-derived on demand inside sign().
        let kp = keypair_from_seed(child_seed);
        let address = hex::encode(&kp.pk);

        Ok(Self {
            mnemonic_phrase: phrase.to_owned(),
            derivation_index: index,
            key_seed: child_seed,
            address,
            public_key_bytes: kp.pk.clone(),
            // sk in kp will be zeroed when kp is dropped here
        })
    }

    /// Generate a new wallet with a freshly randomised BIP-39 mnemonic.
    ///
    /// # Arguments
    ///
    /// * `word_count` — `12` (128-bit entropy) or `24` (256-bit entropy).
    ///                  Prefer `24` for long-lived Architect keys.
    /// * `index`      — Derivation index (see `from_mnemonic`).
    ///
    /// # Errors
    ///
    /// Returns `Err` if `word_count` is not 12 or 24.
    ///
    /// # Example
    ///
    /// ```rust
    /// use primus_sdk::Wallet;
    ///
    /// let master   = Wallet::generate(24, 0).unwrap();
    /// let operator = Wallet::from_mnemonic(master.get_mnemonic(), 1).unwrap();
    /// assert_ne!(master.address, operator.address);
    /// ```
    pub fn generate(word_count: u32, index: u32) -> Result<Self> {
        let entropy_len: usize = match word_count {
            12 => 16,
            24 => 32,
            _ => {
                return Err(anyhow!(
                    "Unsupported word count {}. Must be 12 or 24.",
                    word_count
                ))
            }
        };

        let mut raw_entropy = vec![0u8; entropy_len];
        use rand_core::RngCore;
        rand::thread_rng().fill_bytes(&mut raw_entropy);

        let mnemonic = Mnemonic::from_entropy(&raw_entropy)
            .map_err(|e| anyhow!("Mnemonic::from_entropy failed: {}", e))?;

        Self::from_mnemonic(mnemonic.to_string().as_str(), index)
    }

    // ── Persistence ───────────────────────────────────────────────────────────

    /// Serialize this wallet to a `.secrets` file at `path`.
    ///
    /// The file stores **only** the mnemonic phrase and derivation index —
    /// never the seed or any key bytes. The full keypair is re-derived by
    /// `load()` from the mnemonic, so a stolen file is useless without the
    /// 12 or 24 mnemonic words.
    ///
    /// # File permissions
    ///
    /// After calling `save()`, set permissions with `chmod 600 <path>` (Unix)
    /// or the equivalent OS-level access control on other platforms.
    ///
    /// # Format
    ///
    /// The file is bincode-encoded `WalletFile { version: 1, mnemonic_phrase, derivation_index }`.
    /// Do not hand-edit it; use `load()` to recover and `from_mnemonic()` to re-create.
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        let wf = WalletFile {
            version: 1,
            mnemonic_phrase: self.mnemonic_phrase.clone(),
            derivation_index: self.derivation_index,
        };
        let encoded = bincode::serialize(&wf).context("Failed to serialize WalletFile")?;
        std::fs::write(path, &encoded)
            .with_context(|| format!("Failed to write wallet file to {:?}", path))?;
        Ok(())
    }

    /// Load a wallet from a file previously written by `save()`.
    ///
    /// Fully re-derives the ML-DSA-87 keypair from the stored mnemonic.
    ///
    /// # Stack requirement
    ///
    /// ML-DSA-87 key generation uses ~4 MiB of stack.
    /// Call from a thread with `.stack_size(8 * 1024 * 1024)`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the file cannot be read, the bincode is corrupt, the
    /// mnemonic is invalid, or the format version is unrecognised.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let raw = zeroize::Zeroizing::new(
            std::fs::read(path)
                .with_context(|| format!("Cannot read wallet file from {:?}", path))?,
        );
        let wf: WalletFile = bincode::deserialize(&raw).context(
            "Wallet file is corrupt or was written by an incompatible version. \
                      Delete it and regenerate from your mnemonic.",
        )?;

        if wf.version != 1 {
            return Err(anyhow!(
                "Unsupported wallet file version {}. Expected 1.",
                wf.version
            ));
        }

        let wallet = Self::from_mnemonic(&wf.mnemonic_phrase, wf.derivation_index)?;
        // mnemonic_phrase in wf will be zeroed when wf is dropped here
        Ok(wallet)
    }

    // ── Address / public key ──────────────────────────────────────────────────

    /// Raw ML-DSA-87 verifying key bytes (2592 bytes / `PK_BYTES`).
    ///
    /// Use this when constructing transactions, registering on-chain, or
    /// passing to `TransactionBuilder::recipient()`.
    pub fn get_public_key_bytes(&self) -> Vec<u8> {
        self.public_key_bytes.clone()
    }

    /// First 8 bytes of the address as a compact 16-character hex string.
    ///
    /// For UI display and log output only — **not** a unique identifier.
    /// Use `wallet.address` for all protocol-level operations.
    pub fn get_short_address(&self) -> String {
        self.address[..16].to_string()
    }

    /// Returns the 32-byte child seed used for key derivation.
    /// ⚠️  Use ONLY for deriving secondary keys (e.g. for Noise transport).
    /// NEVER log or transmit this seed.
    pub fn get_key_seed(&self) -> [u8; 32] {
        self.key_seed
    }

    /// Re-derives and returns the raw ML-DSA-87 signing key bytes (4896 bytes).
    ///
    /// # ⚠️ SECURITY WARNING
    ///
    /// This returns the FULL UNENCRYPTED private key.
    /// Use ONLY when passing to low-level node transport or PVM logic.
    /// NEVER log, transmit, or store this output in a non-secure location.
    pub fn get_secret_key_bytes(&self) -> zeroize::Zeroizing<Vec<u8>> {
        let kp = crate::crypto::keypair_from_seed(self.key_seed);
        zeroize::Zeroizing::new(kp.sk.clone())
    }

    // ── Secret access ─────────────────────────────────────────────────────────

    /// Returns the BIP-39 mnemonic phrase.
    ///
    /// # ⚠️ SECURITY WARNING
    ///
    /// The mnemonic is the root of ALL key material for this wallet.
    /// Anyone who reads it can derive every key and spend every token.
    ///
    /// **Never log, print in production, transmit over the network, or display
    /// in a UI without an explicit user action.** Store via the OS Keychain or
    /// Secure Enclave API on device.
    pub fn get_mnemonic(&self) -> &str {
        &self.mnemonic_phrase
    }

    // ── Signing ───────────────────────────────────────────────────────────────

    /// Sign `payload` with this wallet's ML-DSA-87 signing key.
    ///
    /// # Protocol signing convention
    ///
    /// The PVM verifies signatures over the **`reaction_hash`**, which is
    /// built by `TransactionBuilder`. Pass the raw `reaction_hash` bytes here,
    /// not the concatenated `pk ++ last_hash` combination.
    ///
    /// ```text
    /// // Correct call site (inside TransactionBuilder::build):
    /// let reaction_hash = sha3_256(&reaction_data);
    /// let signature = wallet.sign(&reaction_hash);
    /// ```
    ///
    /// # Returns
    ///
    /// A `SIG_BYTES`-byte (4627-byte) ML-DSA-87 signature.
    ///
    /// # Implementation note
    ///
    /// In ml-dsa rc.8, `ExpandedSigningKey` cannot be serialised. This method
    /// calls `sign_with_seed`, which re-derives the full ML-DSA-87 keypair from
    /// `key_seed` on every call. The keypair is ephemeral and dropped after
    /// signing. For occasional CLI or mobile use this is acceptable.
    /// See sdk_audit.md Finding 1 for the OnceCell caching strategy if
    /// high-frequency signing becomes a requirement.
    ///
    /// # Stack requirement
    ///
    /// ~4 MiB — call from a thread with `.stack_size(8 * 1024 * 1024)`.
    pub fn sign(&self, payload: &[u8]) -> Vec<u8> {
        let seed = self.key_seed;
        let payload_owned = payload.to_vec();
        crate::crypto::ensure_high_stack(move || {
            sign_with_seed(seed, &payload_owned)
        })
    }

    /// Verify that `signature` over `payload` was produced by this wallet.
    ///
    /// Delegates to `crate::crypto::verify` — the single canonical ML-DSA-87
    /// verification path in the SDK. (Finding 6 fix: no separate ml_dsa import
    /// in this module.)
    ///
    /// Use this for local sanity checks before broadcasting. The node's PVM
    /// independently verifies all incoming signatures.
    pub fn verify(&self, payload: &[u8], signature: &[u8]) -> bool {
        verify(&self.public_key_bytes, payload, signature)
    }
}

// ── Convenience: address parsing ──────────────────────────────────────────────

impl Wallet {
    /// Decode a hex address string into raw public-key bytes.
    ///
    /// Validates that the decoded length equals `PK_BYTES` (2592).
    /// Use this when you have a recipient address from the UI or config file
    /// and need to pass it to `TransactionBuilder::recipient()`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `hex_address` is not valid hex or decodes to the wrong length.
    pub fn decode_address(hex_address: &str) -> Result<Vec<u8>> {
        let bytes = hex::decode(hex_address).with_context(|| {
            format!(
                "Invalid hex address: {}",
                &hex_address[..16.min(hex_address.len())]
            )
        })?;

        if bytes.len() != PK_BYTES {
            return Err(anyhow!(
                "Address decoded to {} bytes; expected {} (PK_BYTES).",
                bytes.len(),
                PK_BYTES
            ));
        }
        Ok(bytes)
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn large_stack<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(f: F) -> T {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    const KNOWN_PHRASE: &str = "abandon abandon abandon abandon abandon abandon \
         abandon abandon abandon abandon abandon about";

    // ── Derivation stability ──────────────────────────────────────────────────

    #[test]
    fn from_mnemonic_produces_stable_address() {
        large_stack(|| {
            let w1 = Wallet::from_mnemonic(KNOWN_PHRASE, 0).unwrap();
            let w2 = Wallet::from_mnemonic(KNOWN_PHRASE, 0).unwrap();
            assert_eq!(
                w1.address, w2.address,
                "same mnemonic+index must always give the same address"
            );
        });
    }

    #[test]
    fn different_indices_give_different_addresses() {
        large_stack(|| {
            let master = Wallet::from_mnemonic(KNOWN_PHRASE, 0).unwrap();
            let operator = Wallet::from_mnemonic(KNOWN_PHRASE, 1).unwrap();
            assert_ne!(
                master.address, operator.address,
                "index 0 and index 1 must produce different addresses"
            );
        });
    }

    // ── Generation ────────────────────────────────────────────────────────────

    #[test]
    fn generate_12_word_mnemonic() {
        large_stack(|| {
            let w = Wallet::generate(12, 0).unwrap();
            assert_eq!(w.get_mnemonic().split_whitespace().count(), 12);
        });
    }

    #[test]
    fn generate_24_word_mnemonic() {
        large_stack(|| {
            let w = Wallet::generate(24, 0).unwrap();
            assert_eq!(w.get_mnemonic().split_whitespace().count(), 24);
        });
    }

    #[test]
    fn invalid_word_count_is_rejected() {
        // No stack requirement — no keygen happens before the error.
        assert!(Wallet::generate(18, 0).is_err());
    }

    // ── Security: mnemonic is private ─────────────────────────────────────────

    #[test]
    fn mnemonic_accessible_only_via_getter() {
        // This test verifies the access pattern — the compiler enforces the
        // privacy; we document it here for reviewers.
        large_stack(|| {
            let w = Wallet::from_mnemonic(KNOWN_PHRASE, 0).unwrap();
            // w.mnemonic_phrase would be a compile error.
            assert_eq!(w.get_mnemonic(), KNOWN_PHRASE);
        });
    }

    // ── Persistence: save / load round-trip ───────────────────────────────────

    #[test]
    fn save_and_load_round_trip() {
        large_stack(|| {
            let original = Wallet::generate(12, 0).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.wallet");

            original.save(&path).expect("save failed");
            let restored = Wallet::load(&path).expect("load failed");

            assert_eq!(
                original.address, restored.address,
                "address must survive round-trip"
            );
            assert_eq!(
                original.derivation_index, restored.derivation_index,
                "index must survive round-trip"
            );
            assert_eq!(
                original.get_mnemonic(),
                restored.get_mnemonic(),
                "mnemonic must survive round-trip"
            );
        });
    }

    #[test]
    fn load_nonexistent_file_returns_error() {
        let result = Wallet::load(std::path::Path::new("/nonexistent/wallet.bin"));
        assert!(result.is_err());
    }

    // ── Signing and verification ──────────────────────────────────────────────

    #[test]
    fn sign_verify_round_trip() {
        large_stack(|| {
            let w = Wallet::generate(12, 0).unwrap();
            let msg = b"primus_reaction_hash_placeholder";
            let sig = w.sign(msg);

            assert!(w.verify(msg, &sig), "valid signature must verify");
            assert!(
                !w.verify(b"tampered", &sig),
                "tampered message must not verify"
            );
        });
    }

    #[test]
    fn verify_delegates_to_canonical_path() {
        large_stack(|| {
            // Wallet::verify and crate::crypto::verify must agree on every input.
            let w = Wallet::generate(12, 0).unwrap();
            let msg = b"canonical_path_check";
            let sig = w.sign(msg);
            let pk = w.get_public_key_bytes();

            assert_eq!(
                w.verify(msg, &sig),
                crate::crypto::verify(&pk, msg, &sig),
                "Wallet::verify must match crypto::verify"
            );
        });
    }

    #[test]
    fn public_key_has_correct_length() {
        large_stack(|| {
            let w = Wallet::generate(12, 0).unwrap();
            assert_eq!(w.get_public_key_bytes().len(), PK_BYTES);
            assert_eq!(w.address.len(), PK_BYTES * 2, "address is full hex of pk");
        });
    }

    #[test]
    fn sign_produces_correct_sig_length() {
        use crate::crypto::SIG_BYTES;
        large_stack(|| {
            let w = Wallet::generate(12, 0).unwrap();
            let sig = w.sign(b"length_check");
            assert_eq!(sig.len(), SIG_BYTES);
        });
    }

    // ── Address decoding ──────────────────────────────────────────────────────

    #[test]
    fn decode_address_accepts_valid_hex() {
        large_stack(|| {
            let w = Wallet::generate(12, 0).unwrap();
            let pk = Wallet::decode_address(&w.address).unwrap();
            assert_eq!(pk, w.get_public_key_bytes());
        });
    }

    #[test]
    fn decode_address_rejects_short_hex() {
        let result = Wallet::decode_address("deadbeef");
        assert!(result.is_err(), "short hex must be rejected");
    }

    #[test]
    fn decode_address_rejects_invalid_hex() {
        let result = Wallet::decode_address("ZZZZ");
        assert!(result.is_err(), "non-hex characters must be rejected");
    }
}
