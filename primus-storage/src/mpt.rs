use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

pub type Hash32 = [u8; 32];


/// A node in the Merkle-Patricia Trie.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum MptNode {
    /// Leaf node: stores the remaining key nibbles + the atom value.
    Leaf {
        key_suffix: Vec<u8>,
        value: Vec<u8>,
    },
    /// Extension node: shared prefix pointing to one child.
    Extension {
        prefix: Vec<u8>,
        child:  Hash32,
    },
    /// Branch node: 16 nibble-indexed children + optional value at this node.
    Branch {
        children: Box<[Option<Hash32>; 16]>,
        value:    Option<Vec<u8>>,
    },
}


impl MptNode {
    pub fn hash(&self) -> Hash32 {
        match self {
            MptNode::Leaf { .. } | MptNode::Extension { .. } => {
                let bytes = bincode::serialize(self).expect("MptNode serialization is infallible");
                *blake3::hash(&bytes).as_bytes()
            }
            MptNode::Branch { children, value } => {
                let mut combined_xor = [0u8; 32];
                for h in children.iter().flatten() {
                    for (a, b) in combined_xor.iter_mut().zip(h.iter()) {
                        *a ^= *b;
                    }
                }
                // Include value hash in the XOR to keep proofs compact
                if let Some(v) = value {
                    let v_hash = blake3::hash(v);
                    for (a, b) in combined_xor.iter_mut().zip(v_hash.as_bytes().iter()) {
                        *a ^= *b;
                    }
                }
                
                let mut hasher = blake3::Hasher::new();
                hasher.update(b"BRANCH");
                hasher.update(&combined_xor);
                *hasher.finalize().as_bytes()
            }
        }
    }
}

pub fn key_to_nibbles(key: &[u8; 32]) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(64);
    for byte in key {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0F);
    }
    nibbles
}

pub trait MptStore {
    fn get_node(&self, hash: &Hash32) -> Result<Option<MptNode>>;
    fn put_node(&self, node: &MptNode) -> Result<Hash32>;
    fn delete_node(&self, hash: &Hash32) -> Result<()>;
}

pub struct MerklePatriciaTrie<S: MptStore> {

    root: Option<Hash32>,
    store: S,
}

impl<S: MptStore> MerklePatriciaTrie<S> {
    pub fn new(store: S) -> Self {
        Self { root: None, store }
    }

    pub fn with_root(store: S, root: Hash32) -> Self {
        Self { root: Some(root), store }
    }

    pub fn with_root_opt(store: S, root: Option<Hash32>) -> Self {
        Self { root, store }
    }

    pub fn root(&self) -> Option<Hash32> {
        self.root
    }

    pub fn gc_since(&mut self, old_root: Hash32) -> Result<usize> {
        let current_root = match self.root {
            Some(r) => r,
            None    => return Ok(0), // trie is empty — nothing to GC
        };

        // Collect all hashes reachable from current root
        let mut live = std::collections::HashSet::new();
        self.collect_reachable(Some(current_root), &mut live)?;

        // Walk old root subtree, delete nodes not in live set
        let mut deleted = 0usize;
        self.delete_unreachable(Some(old_root), &live, &mut deleted)?;

        Ok(deleted)
    }

    fn collect_reachable(
        &self,
        root: Option<Hash32>,
        live: &mut std::collections::HashSet<Hash32>,
    ) -> Result<()> {
        let hash = match root {
            Some(h) => h,
            None    => return Ok(()),
        };
        if !live.insert(hash) {
            return Ok(()); // already visited
        }
        let node = match self.store.get_node(&hash)? {
            Some(n) => n,
            None    => return Ok(()), // missing node — skip
        };
        match &node {
            MptNode::Leaf { .. }           => {}
            MptNode::Extension { child, .. } => {
                self.collect_reachable(Some(*child), live)?;
            }
            MptNode::Branch { children, .. } => {
                for child in children.iter().flatten() {
                    self.collect_reachable(Some(*child), live)?;
                }
            }
        }
        Ok(())
    }

    fn delete_unreachable(
        &mut self,
        root: Option<Hash32>,
        live: &std::collections::HashSet<Hash32>,
        deleted: &mut usize,
    ) -> Result<()> {
        let hash = match root {
            Some(h) => h,
            None    => return Ok(()),
        };
        if live.contains(&hash) {
            return Ok(()); // still reachable from current root — keep
        }
        let node = match self.store.get_node(&hash)? {
            Some(n) => n,
            None    => return Ok(()), // already deleted
        };
        // Recurse first, then delete this node
        match &node {
            MptNode::Leaf { .. }             => {}
            MptNode::Extension { child, .. } => {
                self.delete_unreachable(Some(*child), live, deleted)?;
            }
            MptNode::Branch { children, .. } => {
                for child in children.iter().flatten() {
                    self.delete_unreachable(Some(*child), live, deleted)?;
                }
            }
        }
        self.store.delete_node(&hash)?;
        *deleted += 1;
        Ok(())
    }

    pub fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        let nibbles = key_to_nibbles(key);
        self.get_recursive(self.root, &nibbles)
    }

    fn get_recursive(&self, root: Option<Hash32>, nibbles: &[u8]) -> Result<Option<Vec<u8>>> {
        let root_hash = match root {
            Some(h) => h,
            None => return Ok(None),
        };

        let node = self.store.get_node(&root_hash)?
            .ok_or_else(|| anyhow!("Node not found"))?;

        match &node {
            MptNode::Leaf { key_suffix, value } => {
                if key_suffix == nibbles {
                    Ok(Some(value.clone()))
                } else {
                    Ok(None)
                }
            }
            MptNode::Extension { prefix, child } => {
                if nibbles.starts_with(prefix) {
                    self.get_recursive(Some(*child), &nibbles[prefix.len()..])
                } else {
                    Ok(None)
                }
            }
            MptNode::Branch { children, value } => {
                if nibbles.is_empty() {
                    Ok(value.clone())
                } else {
                    let nibble = nibbles[0] as usize;
                    self.get_recursive(children[nibble], &nibbles[1..])
                }
            }
        }

    }

    pub fn insert(&mut self, key: &[u8; 32], value: Vec<u8>) -> Result<Hash32> {
        let nibbles = key_to_nibbles(key);
        let new_root = self.insert_recursive(self.root, &nibbles, value)?;
        self.root = Some(new_root);
        Ok(new_root)
    }

    fn insert_recursive(&mut self, root: Option<Hash32>, nibbles: &[u8], value: Vec<u8>) -> Result<Hash32> {
        if root.is_none() {
            return self.store.put_node(&MptNode::Leaf {
                key_suffix: nibbles.to_vec(),
                value,
            });
        }

        let root_hash = root.unwrap();
        let node = self.store.get_node(&root_hash)?
            .ok_or_else(|| anyhow!("Node not found"))?;

        match &node {
            MptNode::Leaf { key_suffix, value: old_value } => {
                let match_len = common_prefix(key_suffix, nibbles);
                if match_len == key_suffix.len() && match_len == nibbles.len() {
                    return self.store.put_node(&MptNode::Leaf { key_suffix: key_suffix.clone(), value });
                }

                let mut branch_children = Box::new([None; 16]);
                let mut branch_value = None;

                if match_len == key_suffix.len() {
                    branch_value = Some(old_value.clone());
                } else {
                    let old_nibble = key_suffix[match_len] as usize;
                    let old_leaf_hash = self.store.put_node(&MptNode::Leaf {
                        key_suffix: key_suffix[match_len + 1..].to_vec(),
                        value: old_value.clone(),
                    })?;
                    branch_children[old_nibble] = Some(old_leaf_hash);
                }

                if match_len == nibbles.len() {
                    branch_value = Some(value);
                } else {
                    let new_nibble = nibbles[match_len] as usize;
                    let new_leaf_hash = self.store.put_node(&MptNode::Leaf {
                        key_suffix: nibbles[match_len + 1..].to_vec(),
                        value,
                    })?;
                    branch_children[new_nibble] = Some(new_leaf_hash);
                }

                let branch_hash = self.store.put_node(&MptNode::Branch {
                    children: branch_children,
                    value: branch_value,
                })?;

                if match_len > 0 {
                    self.store.put_node(&MptNode::Extension {
                        prefix: nibbles[..match_len].to_vec(),
                        child: branch_hash,
                    })
                } else {
                    Ok(branch_hash)
                }
            }
            MptNode::Extension { prefix, child } => {
                let match_len = common_prefix(prefix, nibbles);
                if match_len == prefix.len() {
                    let new_child_hash = self.insert_recursive(Some(*child), &nibbles[match_len..], value)?;
                    return self.store.put_node(&MptNode::Extension { prefix: prefix.clone(), child: new_child_hash });
                }

                let mut branch_children = Box::new([None; 16]);
                let old_nibble = prefix[match_len] as usize;
                let old_child_hash = if match_len + 1 == prefix.len() {
                    *child
                } else {
                    self.store.put_node(&MptNode::Extension {
                        prefix: prefix[match_len + 1..].to_vec(),
                        child: *child,
                    })?
                };
                branch_children[old_nibble] = Some(old_child_hash);

                let mut branch_value = None;
                if match_len == nibbles.len() {
                    branch_value = Some(value);
                } else {
                    let new_nibble = nibbles[match_len] as usize;
                    let new_leaf_hash = self.store.put_node(&MptNode::Leaf {
                        key_suffix: nibbles[match_len + 1..].to_vec(),
                        value,
                    })?;
                    branch_children[new_nibble] = Some(new_leaf_hash);
                }

                let branch_hash = self.store.put_node(&MptNode::Branch {
                    children: branch_children,
                    value: branch_value,
                })?;

                if match_len > 0 {
                    self.store.put_node(&MptNode::Extension {
                        prefix: nibbles[..match_len].to_vec(),
                        child: branch_hash,
                    })
                } else {
                    Ok(branch_hash)
                }
            }
            MptNode::Branch { children, value: old_value } => {
                let mut new_children = children.clone();
                if nibbles.is_empty() {
                    self.store.put_node(&MptNode::Branch { children: new_children, value: Some(value) })
                } else {
                    let nibble = nibbles[0] as usize;
                    let new_child_hash = self.insert_recursive(new_children[nibble], &nibbles[1..], value)?;
                    new_children[nibble] = Some(new_child_hash);
                    self.store.put_node(&MptNode::Branch { children: new_children, value: old_value.clone() })
                }
            }
        }

    }

    pub fn delete(&mut self, key: &[u8; 32]) -> Result<Hash32> {
        let nibbles = key_to_nibbles(key);
        let new_root = self.delete_recursive(self.root, &nibbles)?;
        self.root = new_root;
        Ok(new_root.unwrap_or([0u8; 32]))
    }

    fn delete_recursive(&mut self, root: Option<Hash32>, nibbles: &[u8]) -> Result<Option<Hash32>> {
        let root_hash = match root {
            Some(h) => h,
            None => return Ok(None),
        };

        let node = self.store.get_node(&root_hash)?
            .ok_or_else(|| anyhow!("Node not found"))?;

        let new_node = match &node {
            MptNode::Leaf { key_suffix, .. } => {
                if key_suffix == nibbles {
                    None
                } else {
                    Some(root_hash)
                }
            }
            MptNode::Extension { prefix, child } => {
                if nibbles.starts_with(prefix) {
                    let new_child_hash = self.delete_recursive(Some(*child), &nibbles[prefix.len()..])?;
                    match new_child_hash {
                        Some(h) => {
                            // Canonicalization: If the new child is an Extension or Leaf, merge them.
                            let child_node = self.store.get_node(&h)?;
                            match &child_node {
                                Some(MptNode::Leaf { key_suffix, value }) => {
                                    let mut new_suffix = prefix.clone();
                                    new_suffix.extend_from_slice(key_suffix);
                                    Some(self.store.put_node(&MptNode::Leaf { key_suffix: new_suffix, value: value.clone() })?)
                                }
                                Some(MptNode::Extension { prefix: child_prefix, child: grandchild }) => {
                                    let mut new_prefix = prefix.clone();
                                    new_prefix.extend_from_slice(child_prefix);
                                    Some(self.store.put_node(&MptNode::Extension { prefix: new_prefix, child: *grandchild })?)
                                }
                                _ => Some(self.store.put_node(&MptNode::Extension { prefix: prefix.clone(), child: h })?),
                            }

                        }
                        None => None,
                    }
                } else {
                    Some(root_hash)
                }
            }
            MptNode::Branch { children, value } => {
                let mut new_value = value.clone();
                let mut new_children = children.clone();
                if nibbles.is_empty() {
                    new_value = None;
                } else {
                    let nibble = nibbles[0] as usize;
                    new_children[nibble] = self.delete_recursive(new_children[nibble], &nibbles[1..])?;
                }

                // Canonicalization: Count remaining children
                let remaining: Vec<(usize, Hash32)> = new_children.iter().enumerate()
                    .filter_map(|(i, c)| c.map(|h| (i, h)))
                    .collect();

                if remaining.is_empty() && new_value.is_none() {
                    None
                } else if remaining.is_empty() && new_value.is_some() {
                    // Convert branch with only a value to a leaf with empty suffix
                    Some(self.store.put_node(&MptNode::Leaf {
                        key_suffix: vec![],
                        value: new_value.unwrap(),
                    })?)
                } else if remaining.len() == 1 && new_value.is_none() {
                    // Convert branch with only one child and no value to Extension or merged Leaf
                    let (nibble, child_hash) = remaining[0];
                    let child_node = self.store.get_node(&child_hash)?;
                    match &child_node {
                        Some(MptNode::Leaf { key_suffix, value }) => {
                            let mut new_suffix = vec![nibble as u8];
                            new_suffix.extend_from_slice(key_suffix);
                            Some(self.store.put_node(&MptNode::Leaf { key_suffix: new_suffix, value: value.clone() })?)
                        }
                        Some(MptNode::Extension { prefix, child }) => {
                            let mut new_prefix = vec![nibble as u8];
                            new_prefix.extend_from_slice(prefix);
                            Some(self.store.put_node(&MptNode::Extension { prefix: new_prefix, child: *child })?)
                        }
                        _ => {
                            Some(self.store.put_node(&MptNode::Extension {
                                prefix: vec![nibble as u8],
                                child: child_hash,
                            })?)
                        }
                    }

                } else {
                    Some(self.store.put_node(&MptNode::Branch { children: new_children, value: new_value })?)
                }
            }
        };


        Ok(new_node)
    }


    pub fn prove(&self, key: &[u8; 32]) -> Result<primus_types::MerkleProof> {
        let nibbles  = key_to_nibbles(key);
        let mut siblings = Vec::new();
        let mut path     = Vec::new();

        let value = self.get_with_proof(self.root, &nibbles, &mut siblings, &mut path)?;

        Ok(primus_types::MerkleProof {
            trie_key: *key,
            value,
            root:     self.root.unwrap_or([0u8; 32]),
            siblings,
            path,
        })
    }

    fn get_with_proof(
        &self,
        root:     Option<Hash32>,
        nibbles:  &[u8],
        siblings: &mut Vec<[u8; 32]>,
        path:     &mut Vec<primus_types::PathStep>,
    ) -> Result<Option<Vec<u8>>> {
        use primus_types::PathStep;
        let hash = match root {
            Some(h) => h,
            None    => return Ok(None),
        };

        let node = self.store.get_node(&hash)?
            .ok_or_else(|| anyhow!("Node not found: {:02x?}", &hash[..4]))?;

        match &node {
            MptNode::Leaf { key_suffix, value } => {
                path.push(PathStep::Leaf);
                if key_suffix == nibbles {
                    Ok(Some(value.clone()))
                } else {
                    // Exclusion: reached wrong leaf. Push this leaf's hash to siblings.
                    siblings.push(node.hash());
                    Ok(None)
                }
            }

            MptNode::Extension { prefix, child } => {
                path.push(PathStep::Extension { len: prefix.len() as u8 });
                if nibbles.starts_with(prefix) {
                    self.get_with_proof(Some(*child), &nibbles[prefix.len()..], siblings, path)
                } else {
                    // Exclusion: prefix diverges. Push this extension's hash to siblings.
                    siblings.push(node.hash());
                    Ok(None)
                }
            }

            MptNode::Branch { children, value } => {
                if nibbles.is_empty() {
                    // Key ends exactly at this branch — value lives in branch.value
                    path.push(PathStep::Branch { nibble: 16 }); // 16 = "value slot"
                    
                    // Sibling is XOR(all 16 children) — needed to reconstruct branch hash
                    let mut combined_xor = [0u8; 32];
                    for child in children.iter().flatten() {
                        for (a, b) in combined_xor.iter_mut().zip(child.iter()) {
                            *a ^= *b;
                        }
                    }
                    siblings.push(combined_xor);
                    
                    return Ok(value.clone());
                }

                let nibble     = nibbles[0] as usize;
                let sibling_hash = self.branch_sibling_hash(&node, nibble);
                siblings.push(sibling_hash);
                path.push(PathStep::Branch { nibble: nibble as u8 });

                self.get_with_proof(children[nibble], &nibbles[1..], siblings, path)
            }
        }
    }

    /// XOR-combine all sibling hashes in a Branch except the one at `skip_nibble`.
    /// Also includes the branch's value hash in the XOR.
    fn branch_sibling_hash(&self, node: &MptNode, skip_nibble: usize) -> Hash32 {
        let MptNode::Branch { children, value } = node else { return [0u8; 32]; };

        let mut combined_xor = [0u8; 32];
        for (i, child) in children.iter().enumerate() {
            if i == skip_nibble { continue; }
            if let Some(h) = child {
                for (a, b) in combined_xor.iter_mut().zip(h.iter()) {
                    *a ^= *b;
                }
            }
        }
        if let Some(v) = value {
            let v_hash = blake3::hash(v);
            for (a, b) in combined_xor.iter_mut().zip(v_hash.as_bytes().iter()) {
                *a ^= *b;
            }
        }
        combined_xor
    }
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] { i += 1; }
    i
}

pub fn verify_proof(proof: &primus_types::MerkleProof) -> bool {
    use primus_types::PathStep;

    if proof.path.is_empty() {
        return proof.root == [0u8; 32] && proof.value.is_none();
    }

    let nibbles = key_to_nibbles(&proof.trie_key);
    let mut sibling_idx = proof.siblings.len();

    // Start with the hash of the terminal node
    let mut current_hash = match proof.path.last() {
        Some(PathStep::Leaf) => {
            match &proof.value {
                Some(v) => {
                    // Inclusion at leaf
                    let suffix_start = proof.path.iter().fold(0usize, |acc, step| match step {
                        PathStep::Branch { nibble } if *nibble < 16 => acc + 1,
                        PathStep::Extension { len } => acc + *len as usize,
                        _ => acc,
                    });
                    if suffix_start > nibbles.len() { return false; }
                    let key_suffix = nibbles[suffix_start..].to_vec();
                    MptNode::Leaf { key_suffix, value: v.clone() }.hash()
                }
                None => {
                    // Exclusion at leaf
                    if sibling_idx == 0 { return false; }
                    sibling_idx -= 1;
                    proof.siblings[sibling_idx]
                }
            }
        }
        Some(PathStep::Extension { .. }) => {
            // Exclusion at extension (divergent prefix)
            if proof.value.is_some() || sibling_idx == 0 { return false; }
            sibling_idx -= 1;
            proof.siblings[sibling_idx]
        }
        Some(PathStep::Branch { nibble }) => {
            // Either inclusion of value at branch (nibble 16) or exclusion (child was None)
            if sibling_idx == 0 { return false; }
            sibling_idx -= 1;
            let sibling_xor = proof.siblings[sibling_idx];
            
            let mut combined_xor = sibling_xor;
            if *nibble == 16 {
                if let Some(v) = &proof.value {
                    let v_hash = blake3::hash(v);
                    for (a, b) in combined_xor.iter_mut().zip(v_hash.as_bytes().iter()) {
                        *a ^= *b;
                    }
                }
            } else {
                // Exclusion: nibble < 16, child was None (hash 0). XOR is already correct.
                if proof.value.is_some() { return false; }
            }
            
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"BRANCH");
            hasher.update(&combined_xor);
            *hasher.finalize().as_bytes()
        }
        None => return false,
    };

    // Walk up the path
    for step in proof.path.iter().rev().skip(1) {
        match step {
            PathStep::Branch { nibble: _ } => {
                if sibling_idx == 0 { return false; }
                sibling_idx -= 1;
                let sibling = proof.siblings[sibling_idx];
                
                let mut combined_xor = sibling;
                for (a, b) in combined_xor.iter_mut().zip(current_hash.iter()) {
                    *a ^= *b;
                }
                
                let mut hasher = blake3::Hasher::new();
                hasher.update(b"BRANCH");
                hasher.update(&combined_xor);
                current_hash = *hasher.finalize().as_bytes();
            }
            PathStep::Extension { len } => {
                let mut prefix_start = 0usize;
                for s in proof.path.iter() {
                    if s == step { break; }
                    match s {
                        PathStep::Branch { nibble } if *nibble < 16 => prefix_start += 1,
                        PathStep::Extension { len: l } => prefix_start += *l as usize,
                        _ => {}
                    }
                }
                if prefix_start + *len as usize > nibbles.len() { return false; }
                let prefix = &nibbles[prefix_start..prefix_start + *len as usize];
                current_hash = MptNode::Extension { prefix: prefix.to_vec(), child: current_hash }.hash();
            }
            PathStep::Leaf => return false, // Leaf can only be terminal
        }
    }

    current_hash == proof.root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpt_store::SledMptStore;
    use primus_types::Atom;

    fn setup_trie() -> (MerklePatriciaTrie<SledMptStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = sled::open(dir.path()).unwrap();
        let store = SledMptStore::new(&db).unwrap();
        (MerklePatriciaTrie::new(store), dir)
    }

    fn mock_atom(i: u8) -> Vec<u8> {
        let atom = Atom::new_receiver(vec![i; 32]);
        bincode::serialize(&atom).unwrap()
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let (mut trie, _dir) = setup_trie();
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        let k3 = [3u8; 32];
        let v1 = mock_atom(1);
        let v2 = mock_atom(2);
        let v3 = mock_atom(3);

        trie.insert(&k1, v1.clone()).unwrap();
        trie.insert(&k2, v2.clone()).unwrap();
        trie.insert(&k3, v3.clone()).unwrap();

        assert_eq!(trie.get(&k1).unwrap(), Some(v1));
        assert_eq!(trie.get(&k2).unwrap(), Some(v2));
        assert_eq!(trie.get(&k3).unwrap(), Some(v3));
    }

    #[test]
    fn gc_reduces_node_count() {
        let dir = tempfile::tempdir().unwrap();
        let db = sled::open(dir.path()).unwrap();
        let store = SledMptStore::new(&db).unwrap();
        let mut trie = MerklePatriciaTrie::new(store);
        let mpt_tree = db.open_tree("mpt_nodes").unwrap();

        // Insert k1, record old root
        let k1 = [0xAAu8; 32];
        trie.insert(&k1, mock_atom(1)).unwrap();
        let old_root = trie.root().unwrap();
        let count_before = mpt_tree.len();

        // Insert k2 — creates new path nodes, old k1-only nodes become orphans
        let k2 = [0xBBu8; 32];
        trie.insert(&k2, mock_atom(2)).unwrap();
        let count_after_insert = mpt_tree.len();
        assert!(count_after_insert > count_before, "insert should add nodes");

        // GC orphans from old_root
        let freed = trie.gc_since(old_root).unwrap();
        assert!(freed > 0, "GC should have freed some nodes");
        assert!(mpt_tree.len() < count_after_insert, "node count should decrease after GC");

        // Trie must still be correct
        assert!(trie.get(&k1).unwrap().is_some());
        assert!(trie.get(&k2).unwrap().is_some());
    }

    #[test]
    fn root_changes_on_insert() {
        let (mut trie, _dir) = setup_trie();
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        
        let r1 = trie.insert(&k1, mock_atom(1)).unwrap();
        let r2 = trie.insert(&k2, mock_atom(2)).unwrap();
        
        assert_ne!(r1, r2);
    }

    #[test]
    fn same_state_same_root() {
        let (mut trie1, _dir1) = setup_trie();
        let (mut trie2, _dir2) = setup_trie();
        
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        let k3 = [3u8; 32];
        
        trie1.insert(&k1, mock_atom(1)).unwrap();
        trie1.insert(&k2, mock_atom(2)).unwrap();
        trie1.insert(&k3, mock_atom(3)).unwrap();

        trie2.insert(&k3, mock_atom(3)).unwrap();
        trie2.insert(&k1, mock_atom(1)).unwrap();
        trie2.insert(&k2, mock_atom(2)).unwrap();

        assert_eq!(trie1.root(), trie2.root());
    }

    #[test]
    fn inclusion_proof_verifies() {
        let (mut trie, _dir) = setup_trie();
        let k1 = [1u8; 32];
        let v1 = mock_atom(1);
        trie.insert(&k1, v1).unwrap();

        let proof = trie.prove(&k1).unwrap();
        assert!(verify_proof(&proof));
    }

    #[test]
    fn exclusion_proof_verifies() {
        let (mut trie, _dir) = setup_trie();
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        trie.insert(&k1, mock_atom(1)).unwrap();

        let proof = trie.prove(&k2).unwrap();
        assert!(proof.value.is_none());
        assert!(verify_proof(&proof));
    }

    #[test]
    fn tampered_proof_fails() {
        let (mut trie, _dir) = setup_trie();
        let k1 = [1u8; 32];
        trie.insert(&k1, mock_atom(1)).unwrap();

        let mut proof = trie.prove(&k1).unwrap();
        if !proof.siblings.is_empty() {
            proof.siblings[0][0] ^= 0xFF; // Flip one byte
        } else {
            proof.root[0] ^= 0xFF;
        }
        assert!(!verify_proof(&proof));
    }

    #[test]
    fn delete_removes_key() {
        let (mut trie, _dir) = setup_trie();
        let k1 = [1u8; 32];
        let v1 = mock_atom(1);
        
        let r1 = trie.insert(&k1, v1).unwrap();
        assert!(trie.get(&k1).unwrap().is_some());
        
        let r2 = trie.delete(&k1).unwrap();
        assert!(trie.get(&k1).unwrap().is_none());
        assert_ne!(r1, r2);
    }

    #[test]
    fn compact_proof_is_small() {
        let (mut trie, _dir) = setup_trie();
        // Insert 1000 atoms to build a realistic trie depth
        for i in 0u16..1000 {
            let mut k = [0u8; 32];
            k[0] = (i >> 8) as u8;
            k[1] = (i & 0xFF) as u8;
            trie.insert(&k, mock_atom(i as u8)).unwrap();
        }
        let proof = trie.prove(&[1u8; 32]).unwrap();
        let proof_bytes = bincode::serialize(&proof).unwrap();
        // Compact proof must be under 4 KB for a 1000-node trie
        assert!(
            proof_bytes.len() < 4096,
            "Proof size {} bytes exceeds 4 KB limit", proof_bytes.len()
        );
    }
}
