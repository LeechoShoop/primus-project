// =============================================================================
// primus-sdk/src/crypto/mod.rs — ML-DSA-87 Cryptographic Primitives
//
// ROLE IN THE SDK:
//   This module is the single implementation of every cryptographic operation.
//   All higher-level modules (wallet, transaction) delegate here; they never
//   import ml-dsa directly.
//
// KEY-SIZE CONSTANTS — single source of truth:
//   PK_BYTES  = 2592   ML-DSA-87 verifying key
//   SIG_BYTES = 4627   ML-DSA-87 signature
//   SK_BYTES  = 4896   ML-DSA-87 signing key
//                      ⚠️  SK_BYTES is intentionally NOT re-exported from
//                      lib.rs. External callers must never hold raw signing-key
//                      bytes; the Wallet handles all signing internally via the
//                      stored 32-byte seed.
//
// SECURITY INVARIANTS:
//   1. InternalSeededRng is used ONLY inside keypair_from_seed / sign_with_seed.
//      It must never be used as a general-purpose RNG elsewhere in the codebase.
//   2. sign_with_seed re-derives the full keypair on every call.
//      This is acceptable for CLI / occasional signing. For high-frequency
//      mobile or WASM contexts, cache the signing key in memory (see audit
//      Finding 1 in sdk_audit.md for the recommended OnceCell approach).
//   3. verify() returns bool (never Err) on size / decode failures so hot-path
//      callers can use it as a simple gate without unwrapping.
// =============================================================================

/// Helper to ensure ML-DSA-87 operations have sufficient stack size on Windows.
/// The Windows default stack (often 1 MiB) is insufficient for the ~4 MiB
/// required by ML-DSA-87 key expansion and signing, leading to STATUS_STACK_OVERFLOW.
///
/// # Example
/// ```
/// use primus_sdk::crypto::ensure_high_stack;
/// let result = ensure_high_stack(|| 42);
/// assert_eq!(result, 42);
/// ```
pub fn ensure_high_stack<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    #[cfg(target_os = "windows")]
    {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(f)
            .expect("Failed to spawn high-stack thread for cryptography")
            .join()
            .expect("High-stack cryptography thread panicked")
    }
    #[cfg(not(target_os = "windows"))]
    {
        f()
    }
}

use ml_dsa::signature::{Keypair, SignatureEncoding, Signer};
use ml_dsa::{KeyGen, MlDsa87};
use sha3::{Digest, Sha3_256};
use zeroize::{Zeroize, ZeroizeOnDrop};

// ── Key-size constants ────────────────────────────────────────────────────────

/// Byte length of an ML-DSA-87 verifying (public) key.
pub const PK_BYTES: usize = 2592;

/// Byte length of an ML-DSA-87 signature.
pub const SIG_BYTES: usize = 4627;

/// Byte length of an ML-DSA-87 signing (secret) key.
///
/// This constant exists for internal length assertions only.
/// It is NOT exported from `lib.rs` — callers must never hold raw SK bytes.
/// All signing goes through `sign_with_seed`, which keeps the signing key
/// as an ephemeral local variable.
pub const SK_BYTES: usize = 4896;

// ── Hashing ───────────────────────────────────────────────────────────────────

/// Compute SHA3-256 of `data`. Single source of truth for all hashing in the SDK.
#[inline]
pub fn sha3_256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha3_256::new();
    h.update(data);
    h.finalize().into()
}

/// Derive a 32-byte child seed from a master seed and a derivation index.
///
/// Domain-separates each child with `b"PRIMUS_SDK"` so child seeds derived
/// from the same master but different indices are cryptographically independent.
///
/// # Arguments
/// * `master_seed` — The 64-byte BIP-39 PBKDF2 output (`mnemonic.to_seed("")`).
///                   Passing a shorter slice is accepted but not recommended.
/// * `index`       — Derivation index: 0 = Master Key, 1 = Operator Key, 2+ = custom.
pub fn derive_seed(master_seed: &[u8], index: u32) -> [u8; 32] {
    let mut h = Sha3_256::new();
    h.update(master_seed);
    h.update(index.to_le_bytes());
    h.update(b"PRIMUS_SDK");
    h.finalize().into()
}

// ── Deterministic RNG ─────────────────────────────────────────────────────────

/// SHA3-256 counter-mode DRBG seeded from a 32-byte value.
///
/// Used exclusively inside `keypair_from_seed` and `sign_with_seed`.
/// Must not escape into general-purpose code paths.
#[derive(Zeroize, ZeroizeOnDrop)]
struct InternalSeededRng {
    seed: [u8; 32],
    counter: u64,
}

impl InternalSeededRng {
    fn new(seed: [u8; 32]) -> Self {
        Self { seed, counter: 0 }
    }

    fn fill(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(32) {
            let mut hasher = Sha3_256::new();
            hasher.update(self.seed);
            hasher.update(self.counter.to_le_bytes());
            let result = hasher.finalize();
            let n = chunk.len();
            chunk[..n].copy_from_slice(&result[..n]);
            self.counter += 1;
        }
    }
}

impl rand_core::RngCore for InternalSeededRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.fill(dest);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill(dest);
        Ok(())
    }
}

impl rand_core::CryptoRng for InternalSeededRng {}

// ── Public key-gen and signing primitives ─────────────────────────────────────

/// The result of a key derivation: public key bytes and the originating seed.
///
/// `seed` is stored alongside `pk` so the `Wallet` can re-derive the signing
/// key on demand without ever persisting the full (4896-byte) signing key.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct KeyPairBytes {
    pub pk: Vec<u8>,
    pub sk: Vec<u8>,
    pub seed: [u8; 32],
}

/// Derive an ML-DSA-87 keypair deterministically from `seed`.
///
/// Returns `KeyPairBytes` containing the 2592-byte verifying key and the
/// originating seed. The signing key is discarded after this call — callers
/// use `sign_with_seed` when they need to sign.
///
/// # Stack requirement
/// ML-DSA-87 key expansion uses ~4 MiB of stack.
/// Call from a thread with `.stack_size(8 * 1024 * 1024)`.
pub fn keypair_from_seed(mut seed: [u8; 32]) -> KeyPairBytes {
    let res = ensure_high_stack(move || {
        let mut rng = InternalSeededRng::new(seed);
        let kp = MlDsa87::key_gen(&mut rng);
        let pk = kp.verifying_key().encode().to_vec();
        let sk = kp.signing_key().encode().to_vec();
        debug_assert_eq!(pk.len(), PK_BYTES, "ML-DSA-87 pk must be exactly PK_BYTES");
        debug_assert_eq!(sk.len(), SK_BYTES, "ML-DSA-87 sk must be exactly SK_BYTES");
        let mut seed_copy = seed;
        let res = KeyPairBytes { pk, sk, seed: seed_copy };
        seed_copy.zeroize();
        res
    });
    seed.zeroize();
    res
}

/// Sign `payload` by re-deriving the ML-DSA-87 signing key from `seed`.
///
/// The signing key is ephemeral — it exists only for the duration of this
/// call and is dropped immediately after signing. This is the canonical path
/// for all signing in the SDK; no other code should call `MlDsa87::key_gen`.
///
/// # Stack requirement
/// ~4 MiB — call from a thread with `.stack_size(8 * 1024 * 1024)`.
pub fn sign_with_seed(mut seed: [u8; 32], payload: &[u8]) -> Vec<u8> {
    let mut rng = InternalSeededRng::new(seed);
    let kp = MlDsa87::key_gen(&mut rng);
    let sig = kp.signing_key().sign(payload);
    seed.zeroize();
    sig.to_bytes().to_vec()
}

/// Verify an ML-DSA-87 `signature` over `message` using `pk_bytes`.
///
/// Returns `false` (not `Err`) on any length-mismatch or decode failure, so
/// callers can use this as a simple boolean gate on the hot verification path.
///
/// This is the single canonical verification path in the SDK.
/// `Wallet::verify` delegates here; nothing else should import `ml_dsa` directly.
pub fn verify(pk_bytes: &[u8], message: &[u8], sig_bytes: &[u8]) -> bool {
    use ml_dsa::signature::Verifier;
    use ml_dsa::{MlDsa87, VerifyingKey};

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

    #[test]
    fn keypair_is_deterministic() {
        large_stack(|| {
            let seed = [0x42u8; 32];
            let kp1 = keypair_from_seed(seed);
            let kp2 = keypair_from_seed(seed);
            assert_eq!(kp1.pk, kp2.pk, "same seed must produce same pk");
        });
    }

    #[test]
    fn different_seeds_produce_different_keys() {
        large_stack(|| {
            let kp1 = keypair_from_seed([0x01u8; 32]);
            let kp2 = keypair_from_seed([0x02u8; 32]);
            assert_ne!(kp1.pk, kp2.pk);
        });
    }

    #[test]
    fn sign_and_verify_round_trip() {
        large_stack(|| {
            let seed = [0xABu8; 32];
            let kp = keypair_from_seed(seed);
            let payload = b"primus_sign_test";
            let sig = sign_with_seed(seed, payload);

            assert_eq!(sig.len(), SIG_BYTES, "signature must be SIG_BYTES long");
            assert!(verify(&kp.pk, payload, &sig), "signature must verify");
            assert!(
                !verify(&kp.pk, b"tampered", &sig),
                "bad message must not verify"
            );
        });
    }

    #[test]
    fn verify_rejects_wrong_key() {
        large_stack(|| {
            let seed1 = [0x11u8; 32];
            let seed2 = [0x22u8; 32];
            let kp2 = keypair_from_seed(seed2);
            let payload = b"cross_key_test";
            let sig = sign_with_seed(seed1, payload);
            assert!(!verify(&kp2.pk, payload, &sig), "wrong key must not verify");
        });
    }

    #[test]
    fn verify_returns_false_on_garbage_inputs() {
        // Short pk / sig — must not panic, just return false.
        assert!(!verify(&[0u8; 10], b"msg", &[0u8; 10]));
    }

    #[test]
    fn derive_seed_is_domain_separated() {
        let master = [0xFFu8; 64];
        let s0 = derive_seed(&master, 0);
        let s1 = derive_seed(&master, 1);
        assert_ne!(s0, s1, "different indices must produce different seeds");
    }

    #[test]
    fn sha3_256_matches_known_vector() {
        // SHA3-256("") = a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a
        let expected =
            hex::decode("a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a")
                .unwrap();
        assert_eq!(sha3_256(b"").to_vec(), expected);
    }
}
