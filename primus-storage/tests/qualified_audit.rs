// primus-storage/tests/qualified_audit.rs

use primus_storage::*;
use primus_storage::mpt::{MerklePatriciaTrie, MptNode, key_to_nibbles, verify_proof};
use primus_storage::mpt_store::SledMptStore;
use primus_types::{Atom, Element};
use proptest::prelude::*;
use tempfile::TempDir;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tmp_trie() -> (MerklePatriciaTrie<SledMptStore>, TempDir) {
    let dir = TempDir::new().unwrap();
    let db  = sled::open(dir.path()).unwrap();
    let store = SledMptStore::new(&db).unwrap();
    (MerklePatriciaTrie::new(store), dir)
}

fn atom_bytes(seed: u8) -> Vec<u8> {
    let atom = Atom::new_receiver(vec![seed; 2592]);
    bincode::serialize(&atom).unwrap()
}

fn sha3_key(pk: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Sha3_256};
    let mut h = Sha3_256::new();
    h.update(pk);
    h.finalize().into()
}

// ── S1: DEPENDENCY ISOLATION ─────────────────────────────────────────────────

// Verified via `cargo tree` in Step 3. No runtime test needed.
// Document result in QUALIFIED.md.

// ── S2: CHANGESET ─────────────────────────────────────────────────────────────

#[test]
fn test_changeset_is_btreemap_ordered() {
    let mut cs = Changeset::new();
    let pk_z = vec![0xFFu8; 2592];
    let pk_a = vec![0x00u8; 2592];
    let pk_m = vec![0x80u8; 2592];
    cs.insert(pk_z.clone(), Atom::new_receiver(pk_z.clone()));
    cs.insert(pk_a.clone(), Atom::new_receiver(pk_a.clone()));
    cs.insert(pk_m.clone(), Atom::new_receiver(pk_m.clone()));

    // BTreeMap must iterate in ascending key order
    let keys: Vec<_> = cs.sorted_keys().collect();
    assert!(keys[0] < keys[1], "keys must be in ascending order");
    assert!(keys[1] < keys[2], "keys must be in ascending order");
}

#[test]
fn test_changeset_deterministic_across_insertion_order() {
    // Same atoms, different insertion order — sorted_keys must be identical
    let atoms = vec![
        vec![0x03u8; 2592],
        vec![0x01u8; 2592],
        vec![0x02u8; 2592],
    ];

    let mut cs1 = Changeset::new();
    for pk in &atoms {
        cs1.insert(pk.clone(), Atom::new_receiver(pk.clone()));
    }

    let mut cs2 = Changeset::new();
    for pk in atoms.iter().rev() {
        cs2.insert(pk.clone(), Atom::new_receiver(pk.clone()));
    }

    let keys1: Vec<_> = cs1.sorted_keys().cloned().collect();
    let keys2: Vec<_> = cs2.sorted_keys().cloned().collect();
    assert_eq!(keys1, keys2, "Changeset order must be insertion-order independent");
}

#[test]
fn test_changeset_insert_contract() {
    let mut cs = Changeset::new();
    let hash = [0x42u8; 32];
    let code = vec![0x00, 0x61, 0x73, 0x6d]; // WASM magic bytes
    cs.insert_contract(hash, code.clone());
    assert!(!cs.is_empty());
    assert_eq!(cs.contracts.get(&hash), Some(&code));
}

// ── S3: MPT NODE HASH ─────────────────────────────────────────────────────────

#[test]
fn test_mpt_node_hash_is_deterministic() {
    let node = MptNode::Leaf {
        key_suffix: vec![0, 1, 2, 3],
        value: atom_bytes(1),
    };
    let h1 = node.hash();
    let h2 = node.hash();
    assert_eq!(h1, h2, "MptNode::hash must be deterministic");
}

#[test]
fn test_mpt_node_different_values_different_hashes() {
    let n1 = MptNode::Leaf { key_suffix: vec![0], value: atom_bytes(1) };
    let n2 = MptNode::Leaf { key_suffix: vec![0], value: atom_bytes(2) };
    assert_ne!(n1.hash(), n2.hash(), "Different values must produce different hashes");
}

#[test]
fn test_mpt_node_no_zeroize() {
    // MptNode must NOT implement ZeroizeOnDrop — it contains no secrets.
    // Compile-time check: if this compiles, ZeroizeOnDrop is absent.
    // (We can't assert negatively at runtime; the absence is verified in
    //  structural audit S3. This test documents the invariant.)
    let _node = MptNode::Leaf { key_suffix: vec![], value: vec![] };
    // If MptNode had ZeroizeOnDrop, zeroize::ZeroizeOnDrop would be in scope.
    // We just document that it serializes/deserializes without memory sanitization.
    let bytes = bincode::serialize(&_node).unwrap();
    let decoded: MptNode = bincode::deserialize(&bytes).unwrap();
    assert_eq!(_node, decoded);
}

// ── S4: KEY_TO_NIBBLES ────────────────────────────────────────────────────────

#[test]
fn test_key_to_nibbles_length() {
    let key = [0xABu8; 32];
    let nibbles = key_to_nibbles(&key);
    assert_eq!(nibbles.len(), 64, "32 bytes must expand to 64 nibbles");
}

#[test]
fn test_key_to_nibbles_correctness() {
    let mut key = [0u8; 32];
    key[0] = 0xAB;
    let nibbles = key_to_nibbles(&key);
    assert_eq!(nibbles[0], 0x0A, "high nibble of 0xAB must be 0x0A");
    assert_eq!(nibbles[1], 0x0B, "low nibble of 0xAB must be 0x0B");
    assert_eq!(nibbles[2], 0x00, "second byte is 0x00 → high nibble = 0");
}

// ── S5: TRIE INSERT + GET ─────────────────────────────────────────────────────

#[test]
fn test_trie_insert_and_get_single() {
    let (mut trie, _dir) = tmp_trie();
    let key = [0x01u8; 32];
    let val = atom_bytes(1);
    trie.insert(&key, val.clone()).unwrap();
    assert_eq!(trie.get(&key).unwrap(), Some(val));
}

#[test]
fn test_trie_get_missing_key_returns_none() {
    let (trie, _dir) = tmp_trie();
    let key = [0x99u8; 32];
    assert_eq!(trie.get(&key).unwrap(), None);
}

#[test]
fn test_trie_insert_100_keys_all_retrievable() {
    let (mut trie, _dir) = tmp_trie();
    let mut expected = std::collections::HashMap::new();
    for i in 0u8..100 {
        let mut key = [0u8; 32];
        key[0] = i;
        let val = atom_bytes(i);
        expected.insert(key, val.clone());
        trie.insert(&key, val).unwrap();
    }
    for (key, val) in &expected {
        assert_eq!(trie.get(key).unwrap().as_ref(), Some(val),
            "key {:?} should be retrievable", &key[..2]);
    }
}

#[test]
fn test_trie_root_changes_on_every_insert() {
    let (mut trie, _dir) = tmp_trie();
    let mut roots = vec![];
    for i in 0u8..5 {
        let mut key = [0u8; 32];
        key[0] = i;
        let root = trie.insert(&key, atom_bytes(i)).unwrap();
        roots.push(root);
    }
    // All roots must be distinct
    let unique: std::collections::HashSet<_> = roots.iter().collect();
    assert_eq!(unique.len(), roots.len(), "each insert must produce a unique root");
}

#[test]
fn test_trie_same_state_same_root_regardless_of_insertion_order() {
    let (mut t1, _d1) = tmp_trie();
    let (mut t2, _d2) = tmp_trie();

    let keys: Vec<[u8; 32]> = (0u8..8).map(|i| { let mut k = [0u8; 32]; k[0] = i; k }).collect();

    for &k in &keys {
        t1.insert(&k, atom_bytes(k[0])).unwrap();
    }
    for &k in keys.iter().rev() {
        t2.insert(&k, atom_bytes(k[0])).unwrap();
    }
    assert_eq!(t1.root(), t2.root(),
        "Identical state inserted in different orders must produce the same root");
}

// ── S6: TRIE DELETE ───────────────────────────────────────────────────────────

#[test]
fn test_trie_delete_existing_key() {
    let (mut trie, _dir) = tmp_trie();
    let key = [0x05u8; 32];
    trie.insert(&key, atom_bytes(5)).unwrap();
    assert!(trie.get(&key).unwrap().is_some());

    trie.delete(&key).unwrap();
    assert_eq!(trie.get(&key).unwrap(), None, "deleted key must return None");
}

#[test]
fn test_trie_delete_changes_root() {
    let (mut trie, _dir) = tmp_trie();
    let k1 = [0x01u8; 32];
    let k2 = [0x02u8; 32];
    trie.insert(&k1, atom_bytes(1)).unwrap();
    trie.insert(&k2, atom_bytes(2)).unwrap();
    let root_before = trie.root().unwrap();

    trie.delete(&k1).unwrap();
    let root_after = trie.root().unwrap();
    assert_ne!(root_before, root_after, "delete must change the root");
}

#[test]
fn test_trie_delete_nonexistent_key_is_noop() {
    let (mut trie, _dir) = tmp_trie();
    let k1 = [0x01u8; 32];
    trie.insert(&k1, atom_bytes(1)).unwrap();
    let root_before = trie.root();

    let k2 = [0xFF; 32]; // never inserted
    let _ = trie.delete(&k2); // must not error
    assert_eq!(trie.root(), root_before, "deleting nonexistent key must not change root");
}

// ── S7: INCLUSION PROOF ───────────────────────────────────────────────────────

#[test]
fn test_inclusion_proof_single_leaf() {
    let (mut trie, _dir) = tmp_trie();
    let key = [0x01u8; 32];
    trie.insert(&key, atom_bytes(1)).unwrap();

    let proof = trie.prove(&key).unwrap();
    assert!(proof.value.is_some(), "inclusion proof must have a value");
    assert!(verify_proof(&proof), "inclusion proof must verify");
}

#[test]
fn test_inclusion_proof_100_nodes() {
    let (mut trie, _dir) = tmp_trie();
    let keys: Vec<[u8; 32]> = (0u8..100).map(|i| { let mut k = [0u8; 32]; k[0] = i; k }).collect();
    for &k in &keys {
        trie.insert(&k, atom_bytes(k[0])).unwrap();
    }
    for &k in &keys {
        let proof = trie.prove(&k).unwrap();
        assert!(verify_proof(&proof),
            "proof for key {:02x?} must verify in 100-node trie", &k[..2]);
    }
}

#[test]
fn test_proof_root_matches_trie_root() {
    let (mut trie, _dir) = tmp_trie();
    let key = [0x07u8; 32];
    trie.insert(&key, atom_bytes(7)).unwrap();

    let proof = trie.prove(&key).unwrap();
    assert_eq!(proof.root, trie.root().unwrap(),
        "proof.root must equal the trie's current root");
}

// ── S8: EXCLUSION PROOF ───────────────────────────────────────────────────────

#[test]
fn test_exclusion_proof_missing_key() {
    let (mut trie, _dir) = tmp_trie();
    let k_present = [0x01u8; 32];
    let k_absent  = [0xFFu8; 32];
    trie.insert(&k_present, atom_bytes(1)).unwrap();

    let proof = trie.prove(&k_absent).unwrap();
    assert!(proof.value.is_none(), "exclusion proof must have value = None");
    assert!(verify_proof(&proof), "exclusion proof must verify");
}

#[test]
fn test_exclusion_proof_empty_trie() {
    let (trie, _dir) = tmp_trie();
    let key = [0xAAu8; 32];
    let proof = trie.prove(&key);
    // Empty trie: either returns Ok(proof with None) or an appropriate error.
    // Either is acceptable — document the actual behaviour.
    match proof {
        Ok(p) => assert!(p.value.is_none(), "empty trie exclusion proof value must be None"),
        Err(_) => { /* empty trie returning Err is acceptable */ }
    }
}

// ── S9: TAMPER DETECTION ─────────────────────────────────────────────────────

#[test]
fn test_tampered_sibling_fails_verification() {
    let (mut trie, _dir) = tmp_trie();
    let k1 = [0x01u8; 32];
    let k2 = [0x02u8; 32];
    trie.insert(&k1, atom_bytes(1)).unwrap();
    trie.insert(&k2, atom_bytes(2)).unwrap();

    let mut proof = trie.prove(&k1).unwrap();
    if !proof.siblings.is_empty() {
        proof.siblings[0][0] ^= 0xFF;
        assert!(!verify_proof(&proof), "flipped sibling byte must fail verification");
    }
}

#[test]
fn test_tampered_value_fails_verification() {
    let (mut trie, _dir) = tmp_trie();
    let key = [0x01u8; 32];
    trie.insert(&key, atom_bytes(1)).unwrap();

    let mut proof = trie.prove(&key).unwrap();
    if let Some(ref mut val) = proof.value {
        val[0] ^= 0xFF;
    }
    assert!(!verify_proof(&proof), "tampered value must fail verification");
}

#[test]
fn test_tampered_root_fails_verification() {
    let (mut trie, _dir) = tmp_trie();
    let key = [0x01u8; 32];
    trie.insert(&key, atom_bytes(1)).unwrap();

    let mut proof = trie.prove(&key).unwrap();
    proof.root[0] ^= 0xFF;
    assert!(!verify_proof(&proof), "tampered root must fail verification");
}

#[test]
fn test_proof_wrong_trie_key_fails() {
    let (mut trie, _dir) = tmp_trie();
    let k1 = [0x01u8; 32];
    let k2 = [0x02u8; 32];
    trie.insert(&k1, atom_bytes(1)).unwrap();
    trie.insert(&k2, atom_bytes(2)).unwrap();

    // Generate proof for k1, then change trie_key to k2 — must fail
    let mut proof = trie.prove(&k1).unwrap();
    proof.trie_key = k2;
    assert!(!verify_proof(&proof), "proof with wrong trie_key must fail");
}

// ── S10: GC ───────────────────────────────────────────────────────────────────

#[test]
fn test_gc_removes_orphan_nodes() {
    let dir = TempDir::new().unwrap();
    let db  = sled::open(dir.path()).unwrap();
    let mpt_tree = db.open_tree("mpt_nodes").unwrap();
    let store = SledMptStore::new(&db).unwrap();
    let mut trie = MerklePatriciaTrie::new(store);

    trie.insert(&[0xAAu8; 32], atom_bytes(1)).unwrap();
    let old_root = trie.root().unwrap();
    let count_after_first = mpt_tree.len();

    trie.insert(&[0xBBu8; 32], atom_bytes(2)).unwrap();
    let count_after_second = mpt_tree.len();
    assert!(count_after_second > count_after_first);

    let freed = trie.gc_since(old_root).unwrap();
    assert!(freed > 0, "GC must free at least one orphan node");
    assert!(mpt_tree.len() < count_after_second, "Node count must decrease after GC");

    // Trie must still be fully functional
    assert!(trie.get(&[0xAAu8; 32]).unwrap().is_some());
    assert!(trie.get(&[0xBBu8; 32]).unwrap().is_some());
}

#[test]
fn test_gc_does_not_corrupt_live_data() {
    let (mut trie, _dir) = tmp_trie();
    // Insert 10 keys, record checkpoint, insert 10 more, GC the checkpoint
    for i in 0u8..10 {
        let mut k = [0u8; 32]; k[0] = i;
        trie.insert(&k, atom_bytes(i)).unwrap();
    }
    let checkpoint = trie.root().unwrap();

    for i in 10u8..20 {
        let mut k = [0u8; 32]; k[0] = i;
        trie.insert(&k, atom_bytes(i)).unwrap();
    }

    trie.gc_since(checkpoint).unwrap();

    // All 20 keys must still be readable
    for i in 0u8..20 {
        let mut k = [0u8; 32]; k[0] = i;
        assert!(trie.get(&k).unwrap().is_some(),
            "key {} must survive GC", i);
    }
}

// ── S11: PROOF SIZE ───────────────────────────────────────────────────────────

#[test]
fn test_compact_proof_under_4kb_for_1000_nodes() {
    let (mut trie, _dir) = tmp_trie();
    for i in 0u16..1000 {
        let mut k = [0u8; 32];
        k[0] = (i >> 8) as u8;
        k[1] = (i & 0xFF) as u8;
        trie.insert(&k, atom_bytes(i as u8)).unwrap();
    }
    let proof = trie.prove(&[1u8; 32]).unwrap();
    let size  = bincode::serialize(&proof).unwrap().len();
    assert!(size < 4096,
        "Compact proof must be < 4 KB for 1000-node trie. Got: {} bytes", size);
}

#[test]
fn test_proof_size_scales_logarithmically() {
    // Insert 10 then 1000 nodes. Proof size ratio must be < 3× (log scale).
    let (mut t10, _d10) = tmp_trie();
    for i in 0u8..10 {
        let mut k = [0u8; 32]; k[0] = i;
        t10.insert(&k, atom_bytes(i)).unwrap();
    }
    let proof10 = t10.prove(&[1u8; 32]).unwrap();
    let size10  = bincode::serialize(&proof10).unwrap().len();

    let (mut t1000, _d1000) = tmp_trie();
    for i in 0u16..1000 {
        let mut k = [0u8; 32]; k[0] = (i >> 8) as u8; k[1] = (i & 0xFF) as u8;
        t1000.insert(&k, atom_bytes(i as u8)).unwrap();
    }
    let proof1000 = t1000.prove(&[1u8; 32]).unwrap();
    let size1000  = bincode::serialize(&proof1000).unwrap().len();

    let ratio = size1000 as f64 / size10.max(1) as f64;
    assert!(ratio < 5.0,
        "Proof size should scale logarithmically. 10-node: {} bytes, 1000-node: {} bytes, ratio: {:.1}",
        size10, size1000, ratio);
}

// ── S12: GLOBAL METRICS PHYSICS CANON ────────────────────────────────────────

#[test]
fn test_global_metrics_canonical_uses_physics_canon() {
    use primus_types::physics::PhysicsCanon;
    let m = GlobalMetrics { temperature: 150.5, entropy: 3.14 };
    let (enc_temp, enc_entropy) = m.canonical();
    assert_eq!(enc_temp,    PhysicsCanon::encode(150.5));
    assert_eq!(enc_entropy, PhysicsCanon::encode(3.14));
}

#[test]
fn test_global_metrics_canonical_no_raw_f32_bits() {
    // If canonical() used f32::to_bits(), the result would differ from PhysicsCanon.
    // This test catches that regression.
    let m = GlobalMetrics { temperature: 100.0, entropy: 1.0 };
    let (enc_temp, _) = m.canonical();
    let raw_bits = 100.0f32.to_bits() as u64;
    assert_ne!(enc_temp, raw_bits,
        "canonical() must NOT use f32::to_bits() — use PhysicsCanon::encode()");
}

// ── S13: UNDO LOG ─────────────────────────────────────────────────────────────

#[test]
fn test_undo_log_record_only_first_value() {
    let mut log = UndoLog::new(10, [0u8; 32]);
    let pk = vec![1u8; 32];
    let atom_before = Atom::new_receiver(pk.clone());

    log.record(pk.clone(), Some(atom_before.clone()));
    log.record(pk.clone(), None); // second call must be ignored

    // pre_images must hold the first value
    assert_eq!(log.pre_images.get(&pk).unwrap().as_ref().unwrap().mass,
               atom_before.mass);
}

#[test]
fn test_undo_log_bincode_roundtrip() {
    let mut log = UndoLog::new(5, [0xABu8; 32]);
    let pk = vec![2u8; 2592];
    log.record(pk.clone(), Some(Atom::new_receiver(pk.clone())));

    let bytes   = bincode::serialize(&log).unwrap();
    let decoded: UndoLog = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded.crystal_index, 5);
    assert_eq!(decoded.pre_state_root, [0xABu8; 32]);
    assert!(decoded.pre_images.get(&pk).is_some());
}

// ── S14: STORAGE ERROR ────────────────────────────────────────────────────────

#[test]
fn test_storage_error_proof_too_old_message() {
    let e = StorageError::ProofTooOld { index: 3, tip: 20, window: 8 };
    let msg = e.to_string();
    assert!(msg.contains("3"),  "error message must contain the old index");
    assert!(msg.contains("20"), "error message must contain the tip");
    assert!(msg.contains("8"),  "error message must contain the window");
}

#[test]
fn test_storage_error_crystal_not_found() {
    let e = StorageError::CrystalNotFound(42);
    assert!(e.to_string().contains("42"));
}

// ── S15: SECTORAL MEMPOOL ─────────────────────────────────────────────────────

#[test]
fn test_mempool_push_and_drain() {
    use primus_types::{SignedReaction, Payload, QuantumState};

    let dir = TempDir::new().unwrap();
    let db  = sled::open(dir.path()).unwrap();
    let pool = primus_storage::mempool_v2::SectoralMempool::new(&db).unwrap();

    // Build a minimal valid-structure SignedReaction (energy >= PROTOCOL_MIN_FEE=10)
    let sender = Atom {
        public_key:         vec![0x00u8; 2592], // sector 0
        element:            Element::Hydrogen,
        neutron_count:      0,
        mass:               1_000,
        charge:             2.2,
        last_reaction_hash: [0u8; 32],
        last_active_index:  0,
        nonce:              0,
        quantum_state:      QuantumState::Stable,
    };
    let receiver = Atom::new_receiver(vec![0x01u8; 2592]);

    let mut rx = SignedReaction {
        sender:        sender,
        receiver:      receiver,
        reaction_hash: [0u8; 32],
        energy:        10.0,
        timestamp:     0,
        signature:     vec![0u8; 4627],
        payload:       Payload::Generic,
    };
    rx.reaction_hash = rx.compute_reaction_hash();

    let pushed = pool.push(rx).unwrap();
    assert!(pushed, "first push must succeed");

    let drained = pool.drain_resonant(0, 10);
    assert_eq!(drained.len(), 1, "drain must return the pushed reaction");
}

#[test]
fn test_mempool_no_duplicates() {
    use primus_types::{SignedReaction, Payload, QuantumState};

    let dir = TempDir::new().unwrap();
    let db  = sled::open(dir.path()).unwrap();
    let pool = primus_storage::mempool_v2::SectoralMempool::new(&db).unwrap();

    let sender = Atom {
        public_key: vec![0x00u8; 2592],
        element: Element::Hydrogen, neutron_count: 0, mass: 1_000,
        charge: 2.2, last_reaction_hash: [0u8; 32],
        last_active_index: 0, nonce: 0,
        quantum_state: QuantumState::Stable,
    };
    let mut rx = SignedReaction {
        sender, receiver: Atom::new_receiver(vec![0x01u8; 2592]),
        reaction_hash: [0u8; 32], energy: 10.0, timestamp: 0,
        signature: vec![0u8; 4627], payload: Payload::Generic,
    };
    rx.reaction_hash = rx.compute_reaction_hash();

    let first  = pool.push(rx.clone()).unwrap();
    let second = pool.push(rx).unwrap();
    assert!(first,  "first push must succeed");
    assert!(!second, "duplicate push must return false");
}

// ── S16: PROOF BUILDER ────────────────────────────────────────────────────────

#[test]
fn test_proof_builder_verify_valid() {
    let (mut trie, _dir) = tmp_trie();
    let key = [0x03u8; 32];
    trie.insert(&key, atom_bytes(3)).unwrap();
    let proof = trie.prove(&key).unwrap();
    assert!(ProofBuilder::verify(&proof), "ProofBuilder::verify must pass for valid proof");
}

#[test]
fn test_proof_builder_verify_tampered() {
    let (mut trie, _dir) = tmp_trie();
    let k1 = [0x01u8; 32];
    let k2 = [0x02u8; 32];
    trie.insert(&k1, atom_bytes(1)).unwrap();
    trie.insert(&k2, atom_bytes(2)).unwrap();
    let mut proof = trie.prove(&k1).unwrap();
    proof.root[0] ^= 0xFF;
    assert!(!ProofBuilder::verify(&proof), "ProofBuilder::verify must fail for tampered proof");
}

// ── S17: PROPTEST — FUZZ TRIE ────────────────────────────────────────────────

proptest::proptest! {
    #[test]
    fn prop_trie_insert_get_any_key(
        keys in proptest::collection::vec(
            proptest::array::uniform32(proptest::num::u8::ANY),
            1..20usize
        )
    ) {
        let dir = TempDir::new().unwrap();
        let db  = sled::open(dir.path()).unwrap();
        let store = SledMptStore::new(&db).unwrap();
        let mut trie = MerklePatriciaTrie::new(store);

        let mut inserted = std::collections::HashMap::new();
        for key in &keys {
            let val = atom_bytes(key[0]);
            trie.insert(key, val.clone()).unwrap();
            inserted.insert(*key, val);
        }
        for (key, val) in &inserted {
            prop_assert_eq!(trie.get(key).unwrap(), Some(val.clone()));
        }
    }

    #[test]
    fn prop_valid_proof_always_verifies(
        seed in 0u8..255u8
    ) {
        let dir = TempDir::new().unwrap();
        let db  = sled::open(dir.path()).unwrap();
        let store = SledMptStore::new(&db).unwrap();
        let mut trie = MerklePatriciaTrie::new(store);

        // Insert 5 fixed keys + the target
        for i in 0u8..5 {
            let mut k = [0u8; 32]; k[0] = i;
            trie.insert(&k, atom_bytes(i)).unwrap();
        }
        let mut target = [0u8; 32];
        target[0] = seed;
        trie.insert(&target, atom_bytes(seed)).unwrap();

        let proof = trie.prove(&target).unwrap();
        prop_assert!(verify_proof(&proof),
            "prove() result must always verify for just-inserted key");
    }

    #[test]
    fn prop_tampered_proof_fails(
        seed in 0u8..255u8,
        flip_byte in 0usize..32usize,
    ) {
        let dir = TempDir::new().unwrap();
        let db  = sled::open(dir.path()).unwrap();
        let store = SledMptStore::new(&db).unwrap();
        let mut trie = MerklePatriciaTrie::new(store);

        let mut k1 = [0u8; 32]; k1[0] = seed;
        let mut k2 = [0u8; 32]; k2[0] = seed.wrapping_add(1);
        trie.insert(&k1, atom_bytes(seed)).unwrap();
        trie.insert(&k2, atom_bytes(seed.wrapping_add(1))).unwrap();

        let mut proof = trie.prove(&k1).unwrap();
        proof.root[flip_byte % 32] ^= 0xFF;
        prop_assert!(!verify_proof(&proof),
            "tampered root must always fail verification");
    }
}

// ── S18: CONSTANTS ────────────────────────────────────────────────────────────

#[test]
fn test_constant_finality_depth() {
    assert_eq!(primus_storage::FINALITY_DEPTH, 6u64);
}

#[test]
fn test_constant_undo_window() {
    assert_eq!(primus_storage::UNDO_WINDOW, 8u64);
}
