// =============================================================================
// crypto.rs — Post-Quantum Cryptographic Core (Mainnet-Ready)
//
// SECURITY INVARIANTS:
//   1. GravityEngine is NEVER involved in key derivation (Entropy Separation).
//   2. DeterministicQuantumRng is ONLY used for seed-phrase key generation.
//   3. load_master_key() is the SOLE runtime entry-point for the signing key.
//      It never panics — all errors propagate via anyhow::Result.
//   4. ML-DSA-87 is MANDATORY. No fallback, no bypass paths exist.
// =============================================================================

use anyhow::{Context, Result, anyhow};
use ml_dsa::signature::{SignatureEncoding, Signer, Verifier};
use ml_dsa::{KeyGen, MlDsa87, SigningKey, VerifyingKey};
use rand_core::{CryptoRng, RngCore};
use sha3::{Digest, Sha3_256};
use std::path::Path;

// ── Key-size constants — single source of truth for ALL size checks ──────────
pub const PK_BYTES: usize = 2592;
pub const SK_BYTES: usize = 4896;
pub const SIG_BYTES: usize = 4627;

// ── DeterministicQuantumRng ───────────────────────────────────────────────────
#[allow(dead_code)]
pub struct DeterministicQuantumRng {
    state: Vec<u8>,
    counter: u64,
}
#[allow(dead_code)]
impl DeterministicQuantumRng {
    pub fn new(seed: [u8; 32]) -> Self {
        Self {
            state: seed.to_vec(),
            counter: 0,
        }
    }
}
impl RngCore for DeterministicQuantumRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(32) {
            let mut h = Sha3_256::new();
            h.update(&self.state);
            h.update(self.counter.to_be_bytes());
            let result = h.finalize();
            let len = chunk.len().min(32);
            chunk[..len].copy_from_slice(&result[..len]);
            self.counter += 1;
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
impl CryptoRng for DeterministicQuantumRng {}

// ── ArchitectKeys ─────────────────────────────────────────────────────────────
/// Typed wrapper — pk/sk can never be accidentally swapped at a call-site.
pub struct ArchitectKeys {
    pub pk: Vec<u8>,
    pub sk: Vec<u8>,
}

// ── crypto ────────────────────────────────────────────────────────────────────
pub struct Crypto;

impl Crypto {
    // ── Hash primitives ───────────────────────────────────────────────────────
    #[inline]
    pub fn sha3_256(data: &[u8]) -> [u8; 32] {
        let mut h = Sha3_256::new();
        h.update(data);
        h.finalize().into()
    }

    // ── Argon2 proof-of-viscosity ─────────────────────────────────────────────

    /// Compute an Argon2 hash of `data` with `salt`. Kept for the
    /// `initiate_protected_reaction` path in `kinetic.rs` and future
    /// proof-of-viscosity work; not called on the current hot mining path.
    #[allow(dead_code)]
    pub fn compute_argon2(data: &[u8], salt: &[u8]) -> Result<Vec<u8>> {
        use argon2::{
            Argon2,
            password_hash::{PasswordHasher, SaltString},
        };
        let salt_obj = SaltString::encode_b64(salt)
            .map_err(|e| anyhow!("Argon2: salt encoding failed: {}", e))?;
        let hash = Argon2::default()
            .hash_password(data, &salt_obj)
            .map_err(|e| anyhow!("Argon2: hashing failed: {}", e))?
            .to_string();
        Ok(hash.into_bytes())
    }

    #[allow(dead_code)]
    pub fn verify_argon2(data: &[u8], hash_bytes: &[u8]) -> bool {
        use argon2::{
            Argon2,
            password_hash::{PasswordHash, PasswordVerifier},
        };
        let Ok(s) = std::str::from_utf8(hash_bytes) else {
            return false;
        };
        let Ok(h) = PasswordHash::new(s) else {
            return false;
        };
        Argon2::default().verify_password(data, &h).is_ok()
    }

    // ── Master Key I/O ────────────────────────────────────────────────────────

    /// Generate a deterministic ML-DSA-87 keypair from `secret_phrase`,
    /// write it to `path` as bincode((pk, sk)), and return `ArchitectKeys`.
    ///
    /// MUST be called from a thread with ≥ 8 MiB stack (MlDsa87::key_gen is
    /// stack-heavy). Never panics.
    #[allow(dead_code)]
    pub fn generate_and_save_key(secret_phrase: &str, path: &Path) -> Result<ArchitectKeys> {
        let seed = Self::sha3_256(secret_phrase.as_bytes());
        let mut rng = DeterministicQuantumRng::new(seed);
        let kp = MlDsa87::key_gen(&mut rng);
        let pk = kp.verifying_key().encode().to_vec();
        let sk = kp.signing_key().encode().to_vec();

        debug_assert_eq!(pk.len(), PK_BYTES);
        debug_assert_eq!(sk.len(), SK_BYTES);

        let encoded = bincode::serialize(&(pk.clone(), sk.clone()))
            .context("Failed to serialize architect keypair")?;
        std::fs::write(path, &encoded)
            .with_context(|| format!("Failed to write master key to {:?}", path))?;

        println!("🔐 New Architect keypair generated and saved to {:?}", path);
        Ok(ArchitectKeys { pk, sk })
    }

    /// PRIMARY RUNTIME ENTRY-POINT for loading the Architect's keypair.
    ///
    /// Reads `path`, deserializes via bincode, validates byte lengths strictly,
    /// and returns `ArchitectKeys`. Never panics.
    pub fn load_master_key(path: &Path) -> Result<ArchitectKeys> {
        let raw = std::fs::read(path)
            .with_context(|| format!("Cannot read master key from {:?}", path))?;
        let (pk, sk): (Vec<u8>, Vec<u8>) = bincode::deserialize(&raw).context(
            "master.key is corrupt or from an incompatible version. \
                      Delete it to regenerate.",
        )?;

        if pk.len() != PK_BYTES {
            return Err(anyhow!(
                "master.key verifying key: {} bytes (expected {})",
                pk.len(),
                PK_BYTES
            ));
        }
        if sk.len() != SK_BYTES {
            return Err(anyhow!(
                "master.key signing key: {} bytes (expected {})",
                sk.len(),
                SK_BYTES
            ));
        }
        Ok(ArchitectKeys { pk, sk })
    }

    // ── ML-DSA-87 Sign / Verify ───────────────────────────────────────────────

    /// Signs `message` with the provided ML-DSA-87 signing key bytes.
    /// Returns `Err` if `sk_bytes` is not exactly `SK_BYTES` long.
    pub fn sign(sk_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>> {
        let sk_arr: &[u8; SK_BYTES] = sk_bytes.try_into().map_err(|_| {
            anyhow!(
                "sign(): signing key is {} bytes, expected {}",
                sk_bytes.len(),
                SK_BYTES
            )
        })?;
        let sk = SigningKey::<MlDsa87>::decode(sk_arr.into());
        Ok(sk.sign(message).to_bytes().to_vec())
    }

    /// Verifies an ML-DSA-87 `signature` over `message` with `pk_bytes`.
    /// Returns `false` (not `Err`) on any size/decode failure so hot-path
    /// callers can use it as a simple boolean gate.
    pub fn verify(pk_bytes: &[u8], message: &[u8], sig_bytes: &[u8]) -> bool {
        let pk_arr: &[u8; PK_BYTES] = match pk_bytes.try_into() {
            Ok(arr) => arr,
            Err(_) => return false,
        };
        let sig_arr: &[u8; SIG_BYTES] = match sig_bytes.try_into() {
            Ok(arr) => arr,
            Err(_) => return false,
        };

        let pk = VerifyingKey::<MlDsa87>::decode(pk_arr.into());
        match ml_dsa::Signature::<MlDsa87>::decode(sig_arr.into()) {
            Some(sig) => pk.verify(message, &sig).is_ok(),
            None => false,
        }
    }

    // ── Legacy shim (kept for existing call-sites) ────────────────────────────
    #[allow(dead_code)]
    pub fn generate_architect_key_from_seed(phrase: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let seed = Self::sha3_256(phrase.as_bytes());
        let mut rng = DeterministicQuantumRng::new(seed);
        let kp = MlDsa87::key_gen(&mut rng);
        Ok((
            kp.verifying_key().encode().to_vec(),
            kp.signing_key().encode().to_vec(),
        ))
    }
}
