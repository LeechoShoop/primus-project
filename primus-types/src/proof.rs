use serde::{Serialize, Deserialize};
use crate::Vec;

/// Merkle-Patricia Trie inclusion or exclusion proof.
///
/// # Format change v2 (compact siblings)
///
/// `nodes: Vec<Vec<u8>>` (full serialized MptNode bytes, ~25 KB per proof)
/// is replaced by two fields:
///   - `path`     — the sequence of node-type tags + nibble indices traversed
///   - `siblings` — the sibling hashes at each branch point
///
/// Proof size: O(depth × 32) ≈ 2 KB for a balanced 64-level trie.
/// The old `nodes` field is removed. This is a **wire format break**.
/// Bump the protocol version constant in primus-types::constants.
#[derive(
    Serialize, Deserialize, Debug, Clone, PartialEq,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
pub struct MerkleProof {
    /// SHA3-256(public_key) — the MPT trie key being proved.
    pub trie_key: [u8; 32],

    /// The atom value (bincode-serialized) for inclusion proofs.
    /// None for exclusion proofs.
    pub value: Option<Vec<u8>>,

    /// The trie root this proof was generated against.
    pub root: [u8; 32],

    /// Sibling hashes at each decision point from root to leaf.
    ///
    /// For a Branch node at nibble N: sibling = hash of the OTHER child
    /// that is NOT on the path to our key. If the branch child is None,
    /// sibling = [0u8; 32] (sentinel for empty slot).
    /// For an Extension node: no sibling (extension has only one child).
    /// For a Leaf node: no sibling needed.
    pub siblings: Vec<[u8; 32]>,

    /// Path metadata for reconstruction. One entry per node on the path.
    /// Encodes: node type + nibble taken (for Branch) + prefix length (for Extension).
    pub path: Vec<PathStep>,
}

/// A single step on the proof path from root to leaf.
#[derive(
    Serialize, Deserialize, Debug, Clone, PartialEq,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
pub enum PathStep {
    /// Traversed a Branch node via nibble `n`.
    Branch { nibble: u8 },
    /// Traversed an Extension node with a shared prefix of `len` nibbles.
    Extension { len: u8 },
    /// Reached a Leaf node (always the final step).
    Leaf,
}

