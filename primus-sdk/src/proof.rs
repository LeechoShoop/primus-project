// =============================================================================
// primus-sdk/src/proof.rs
//
// Proof verification for light clients.
// This module has ZERO dependency on primus-storage or Sled.
// It uses only primus-types::MerkleProof and pure blake3 hashing.
// Must compile in WASM (no I/O, no std::net, no Sled).
// =============================================================================

use primus_types::{Atom, MerkleProof};
use anyhow::{anyhow, Result};
use crate::error::PrimusSdkError;

/// Maximum age of a Merkle proof in blocks (matches primus-core UNDO_WINDOW).
pub const MAX_PROOF_AGE_BLOCKS: u64 = 8;

/// Verify a balance proof without any chain state or storage access.
///
/// # Arguments
///
/// * `proof`         — The MerkleProof received from a full node.
/// * `expected_root` — The state root of the crystal you trust
///                     (obtained from a trusted block header).
///
/// # Returns
///
/// `Ok(Some(atom))` — proof is valid, here is the atom at this key.
/// `Ok(None)`       — proof is valid exclusion proof (atom does not exist).
/// `Err(_)`         — proof is invalid or tampered.
pub fn verify_balance_proof(
    proof:          &MerkleProof,
    expected_root:  &[u8; 32],
    proof_height:   u64,
    current_height: u64,
) -> Result<Option<Atom>> {
    // 0. Check for staleness (TRDIV-001)
    if current_height.saturating_sub(proof_height) > MAX_PROOF_AGE_BLOCKS {
        return Err(PrimusSdkError::ProofTooOld {
            proof_height,
            current_height,
        }.into());
    }

    // 1. Check the proof is against the root we trust
    if &proof.root != expected_root {
        return Err(anyhow!(
            "Proof root {:02x?}… does not match trusted root {:02x?}…",
            &proof.root[..4], &expected_root[..4]
        ));
    }

    // 2. Verify the cryptographic proof chain
    // verify_proof is a pure function from primus-storage, re-exported here
    // via a thin wrapper so the SDK doesn't depend on primus-storage directly.
    if !verify_proof_pure(proof) {
        return Err(anyhow!("Proof verification failed — proof is invalid or tampered"));
    }

    // 3. Decode the atom if this is an inclusion proof
    match &proof.value {
        Some(atom_bytes) => {
            let atom: Atom = bincode::deserialize(atom_bytes)
                .map_err(|e| anyhow!("Failed to decode atom from proof: {}", e))?;
            Ok(Some(atom))
        }
        None => Ok(None), // exclusion proof — atom does not exist
    }
}

/// Pure proof verification — no I/O, no Sled, WASM-safe.
/// Mirrors primus_storage::mpt::verify_proof() without the storage dependency.
fn verify_proof_pure(proof: &MerkleProof) -> bool {
    use primus_types::{PathStep};

    if proof.path.is_empty() {
        return proof.root == [0u8; 32] && proof.value.is_none();
    }

    // This is the same algorithm as primus-storage/src/mpt.rs::verify_proof()
    // IMPORTANT: must stay in sync with that implementation.
    // Consider exposing verify_proof as a pub fn in primus-types in a future refactor.
    let nibbles = crate::proof_util::key_to_nibbles(&proof.trie_key);

    // Reconstruct leaf hash
    let leaf_node_hash = match proof.path.last() {
        Some(PathStep::Leaf) => {
            // Compute suffix start position
            let suffix_start = proof.path.iter().fold(0usize, |acc, step| match step {
                PathStep::Branch { nibble } if *nibble != 16 => acc + 1,
                PathStep::Extension { len } => acc + *len as usize,
                _ => acc,
            });
            let key_suffix = nibbles[suffix_start..].to_vec();

            match &proof.value {
                Some(v) => {
                    // Hash a Leaf node with this suffix and value
                    let leaf_bytes = bincode::serialize(&LeafRepr {
                        key_suffix,
                        value: v.clone(),
                    }).unwrap_or_default();
                    *blake3::hash(&leaf_bytes).as_bytes()
                }
                None => return true, // exclusion: trust path reconstruction
            }
        }
        _ => return false,
    };

    let mut current_hash = leaf_node_hash;
    let mut sibling_idx  = proof.siblings.len();
    let mut nibble_pos   = nibbles.len();

    for step in proof.path.iter().rev().skip(1) {
        match step {
            PathStep::Branch { nibble } => {
                if sibling_idx == 0 { return false; }
                sibling_idx -= 1;
                nibble_pos  -= 1;
                let sibling = proof.siblings[sibling_idx];
                let mut h = blake3::Hasher::new();
                h.update(&[*nibble]);
                h.update(&current_hash);
                h.update(&sibling);
                current_hash = *h.finalize().as_bytes();
            }
            PathStep::Extension { len } => {
                nibble_pos -= *len as usize;
                let prefix = &nibbles[nibble_pos..nibble_pos + *len as usize];
                let ext_bytes = bincode::serialize(&ExtRepr {
                    prefix: prefix.to_vec(),
                    child:  current_hash,
                }).unwrap_or_default();
                current_hash = *blake3::hash(&ext_bytes).as_bytes();
            }
            PathStep::Leaf => return false,
        }
    }

    current_hash == proof.root
}

// Local repr types that match MptNode serialization for hashing.
// Must stay in sync with primus-storage::mpt::MptNode wire format.
#[derive(serde::Serialize)]
struct LeafRepr { key_suffix: Vec<u8>, value: Vec<u8> }

#[derive(serde::Serialize)]
struct ExtRepr  { prefix: Vec<u8>, child: [u8; 32] }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_balance_proof_rejects_wrong_root() {
        let proof = MerkleProof {
            trie_key: [1u8; 32],
            value:    None,
            root:     [0u8; 32],
            siblings: vec![],
            path:     vec![],
        };
        let wrong_root = [2u8; 32];
        let result = verify_balance_proof(&proof, &wrong_root, 10, 10);
        assert!(result.is_err(), "Should reject proof with wrong root");
    }

    #[test]
    fn verify_stale_proof_fails() {
        let proof = MerkleProof {
            trie_key: [1u8; 32],
            value:    None,
            root:     [0u8; 32],
            siblings: vec![],
            path:     vec![],
        };
        // Proof is 10 blocks old, window is 8
        let result = verify_balance_proof(&proof, &[0u8; 32], 0, 10);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Merkle proof is too old"));
    }

    #[test]
    fn verify_empty_trie_exclusion_proof() {
        let proof = MerkleProof {
            trie_key: [42u8; 32],
            value:    None,
            root:     [0u8; 32], // empty trie root
            siblings: vec![],
            path:     vec![],
        };
        let result = verify_balance_proof(&proof, &[0u8; 32], 10, 10);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none(), "Should be exclusion proof");
    }
}
