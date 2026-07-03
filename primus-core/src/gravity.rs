// =============================================================================
// gravity.rs — Deterministic Physics Engine (Mainnet-Ready)
//
// BUG 1 FIX — get_system_entropy() removed from the block-synthesis path.
//
// PROBLEM (original code):
//   generate_roll() called get_system_entropy(), which concatenated:
//     - SystemTime::now().as_nanos()  — different on every node, every call
//     - sys.cpus().cpu_usage()        — live CPU load, unique per machine
//     - sys.total_memory()            — hardware-specific constant
//   This made the evaporation filter in synthesize_with_gravity() produce
//   a different surviving transaction set on every node from an identical
//   mempool. Crystal #2 was the first block with enough transactions to
//   expose the divergence.
//
// FIX:
//   generate_roll() now accepts the full block-context parameters it needs to
//   be deterministic: prev_hash, crystal_index, and a per-user nonce.
//   The roll is SHA3-256(server_seed || prev_hash || crystal_index || user_id || nonce).
//   Every node running the same block will produce the same roll for the
//   same (user_id, nonce) pair.
//
// ENTROPY SEPARATION (unchanged invariant):
//   - GravityEngine is NEVER used for key derivation (crypto.rs owns that).
//   - get_system_entropy() is retained as a private method for use ONLY in
//     external tooling / RTP auditing, and is explicitly gated behind
//     #[cfg(feature = "rtp_audit")]. It must not be called from any code
//     path that participates in block synthesis or state root computation.
//
// WHY server_seed is still included:
//   The server_seed provides operator-level replay protection between
//   independent chains. It does NOT break determinism because it is the same
//   constant string on all nodes of the same network ("primus_alpha_seed_2026").
// =============================================================================

use crate::crypto::Crypto;

pub struct GravityEngine {
    pub server_seed: String,
    pub rtp: f32,
}

impl GravityEngine {
    pub fn new(server_seed: &str, rtp: f32) -> Self {
        Self {
            server_seed: server_seed.to_string(),
            rtp,
        }
    }

    /// Generate a deterministic roll for a specific (user_id, nonce) pair
    /// within a specific block context.
    ///
    /// # Determinism contract
    ///
    /// Given identical inputs, every node on the network will produce the
    /// exact same u64. The roll is:
    ///
    ///   SHA3-256(server_seed || prev_crystal_hash || crystal_index_le || user_id || nonce_le)
    ///
    /// Fields:
    ///   - `user_id`        — the sender's public key bytes
    ///   - `nonce`          — the sender's atom nonce (replay protection)
    ///   - `prev_hash`      — the parent crystal's density hash (chain-binds the roll)
    ///   - `crystal_index`  — current block height (additional domain separation)
    ///
    /// # Why prev_hash is required
    ///
    /// Without it, a roll for (user_id=X, nonce=5) would be identical in every
    /// block where X has nonce 5, making the evaporation filter predictable and
    /// gameable. Binding to prev_hash makes each block's rolls unique even when
    /// the same transactions are resubmitted.
    pub fn generate_roll(
        &self,
        user_id: &[u8],
        nonce: u64,
        prev_hash: &[u8; 32],
        crystal_index: u64,
    ) -> u64 {
        let mut data = Vec::with_capacity(self.server_seed.len() + 32 + 8 + user_id.len() + 8);
        data.extend_from_slice(self.server_seed.as_bytes());
        data.extend_from_slice(prev_hash);
        data.extend_from_slice(&crystal_index.to_le_bytes());
        data.extend_from_slice(user_id);
        data.extend_from_slice(&nonce.to_le_bytes());

        let hash = Crypto::sha3_256(&data);
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&hash[0..8]);
        u64::from_be_bytes(bytes)
    }

    /// Collect live system entropy for external RTP auditing ONLY.
    ///
    /// This method MUST NOT be called from any code path that participates
    /// in block synthesis, transaction ordering, or state root computation.
    /// It is gated behind the "rtp_audit" feature flag to make accidental
    /// use a compile error in production builds.
    #[cfg(feature = "rtp_audit")]
    pub fn get_system_entropy(&self) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        use sysinfo::System;

        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        let cpu_load: f32 = sys.cpus().iter().map(|c| c.cpu_usage()).sum();
        let mem_total = sys.total_memory();

        let seed_hash = hex::encode(crate::crypto::Crypto::sha3_256(
            self.server_seed.as_bytes()
        ));
        format!(
            "{}|{:.2}|{}|{}|{}",
            ts, cpu_load, mem_total, self.rtp, seed_hash
        )
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> GravityEngine {
        GravityEngine::new("primus_alpha_seed_2026", 0.92)
    }

    /// Same inputs must always produce the same roll.
    #[test]
    fn roll_is_deterministic() {
        let e = engine();
        let uid = vec![0xde, 0xad, 0xbe, 0xef];
        let prev = [1u8; 32];
        let r1 = e.generate_roll(&uid, 7, &prev, 42);
        let r2 = e.generate_roll(&uid, 7, &prev, 42);
        assert_eq!(r1, r2);
    }

    /// Different prev_hash must produce different rolls.
    #[test]
    fn different_prev_hash_changes_roll() {
        let e = engine();
        let uid = vec![0x01, 0x02];
        let h1 = [0u8; 32];
        let h2 = [1u8; 32];
        assert_ne!(
            e.generate_roll(&uid, 1, &h1, 5),
            e.generate_roll(&uid, 1, &h2, 5),
        );
    }

    /// Different nonces must produce different rolls.
    #[test]
    fn different_nonce_changes_roll() {
        let e = engine();
        let uid = vec![0xAA, 0xBB];
        let prev = [2u8; 32];
        assert_ne!(
            e.generate_roll(&uid, 0, &prev, 10),
            e.generate_roll(&uid, 1, &prev, 10),
        );
    }

    /// Different crystal_index must produce different rolls.
    #[test]
    fn different_crystal_index_changes_roll() {
        let e = engine();
        let uid = vec![0x11];
        let prev = [3u8; 32];
        assert_ne!(
            e.generate_roll(&uid, 5, &prev, 1),
            e.generate_roll(&uid, 5, &prev, 2),
        );
    }
}
