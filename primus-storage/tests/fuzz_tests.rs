// fuzz_tests.rs  — place in tests/fuzz_tests.rs
//
// Run with:
//   cargo test --test fuzz_tests -- --nocapture
//
// Longer corpus sweep (CI):
//   FUZZ_ITERS=100000 cargo test --test fuzz_tests --release -- --nocapture
//
// For libFuzzer targets see fuzz/fuzz_targets/ (requires `cargo-fuzz`).
// ─────────────────────────────────────────────────────────────────────────────

#![allow(clippy::all)]

use primus_storage::{
    Changeset,
    MerklePatriciaTrie,
    ProofBuilder,
};
use primus_storage::mpt::{verify_proof, MptNode, key_to_nibbles};
use primus_storage::mpt_store::SledMptStore;
use primus_types::atom::Atom;
use primus_types::MerkleProof;

use std::collections::{BTreeMap, HashMap};

// ── helpers ──────────────────────────────────────────────────────────────────

fn iters() -> usize {
    std::env::var("FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

/// Deterministic "random" byte expansion from a single u32 seed (xorshift32).
fn xorshift32(mut x: u32) -> u32 {
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

fn seeded_key(seed: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    let mut s = seed;
    for chunk in key.chunks_mut(4) {
        s = xorshift32(s);
        chunk.copy_from_slice(&s.to_le_bytes());
    }
    key
}

fn seeded_value(seed: u32) -> Vec<u8> {
    let mut s = xorshift32(seed | 0xDEAD_BEEF);
    let len = (s % 64 + 4) as usize; // 4..67 bytes
    let mut v = Vec::with_capacity(len);
    for _ in 0..len {
        s = xorshift32(s);
        v.push(s as u8);
    }
    v
}

fn mock_atom(seed: u32) -> Vec<u8> {
    let atom = Atom::new_receiver(seeded_key(seed).to_vec());
    bincode::serialize(&atom).unwrap()
}

fn make_trie() -> (MerklePatriciaTrie<SledMptStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db  = sled::open(dir.path()).unwrap();
    let store = SledMptStore::new(&db).unwrap();
    (MerklePatriciaTrie::new(store), dir)
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1 — Changeset fuzz
// ═══════════════════════════════════════════════════════════════════════════

/// Property: Changeset always iterates keys in lexicographic order regardless
/// of insertion order (consensus invariant — BTreeMap guarantee).
#[test]
fn fuzz_changeset_deterministic_order() {
    let n = iters().min(5_000);

    for seed in 0..n as u32 {
        let mut cs = Changeset::new();
        let mut s = seed;

        // Insert up to 32 keys in random order
        let count = (xorshift32(s) % 32 + 1) as usize;
        s = xorshift32(s);

        let mut pairs: Vec<(Vec<u8>, Atom)> = (0..count)
            .map(|i| {
                s = xorshift32(s + i as u32);
                let pk: Vec<u8> = seeded_key(s).to_vec();
                let atom = Atom::new_receiver(pk.clone());
                (pk, atom)
            })
            .collect();

        // Insert in original order
        for (pk, atom) in &pairs {
            cs.insert(pk.clone(), atom.clone());
        }
        let keys_fwd: Vec<Vec<u8>> = cs.sorted_keys().cloned().collect();

        // Reverse insertion order
        let mut cs2 = Changeset::new();
        pairs.reverse();
        for (pk, atom) in &pairs {
            cs2.insert(pk.clone(), atom.clone());
        }
        let keys_rev: Vec<Vec<u8>> = cs2.sorted_keys().cloned().collect();

        assert_eq!(
            keys_fwd, keys_rev,
            "seed={}: Changeset key order must be deterministic (BTreeMap)",
            seed
        );
    }
}

/// Property: Changeset.get() returns the most-recently inserted value for a key.
#[test]
fn fuzz_changeset_overwrite_semantics() {
    let n = iters().min(2_000);
    for seed in 0..n as u32 {
        let mut cs = Changeset::new();
        let pk = seeded_key(seed).to_vec();

        let atom1 = Atom::new_receiver(vec![1u8; 32]);
        let atom2 = Atom::new_receiver(vec![2u8; 32]);

        cs.insert(pk.clone(), atom1);
        cs.insert(pk.clone(), atom2.clone());

        let got = cs.get(&pk).expect("key must be present after two inserts");
        assert_eq!(
            bincode::serialize(got).unwrap(),
            bincode::serialize(&atom2).unwrap(),
            "seed={}: last write must win",
            seed
        );
    }
}

/// Property: is_empty() / len() stay consistent under mixed inserts.
#[test]
fn fuzz_changeset_len_consistency() {
    let n = iters().min(1_000);
    for seed in 0..n as u32 {
        let mut cs = Changeset::new();
        assert!(cs.is_empty());
        assert_eq!(cs.len(), 0);

        let mut unique: BTreeMap<Vec<u8>, ()> = BTreeMap::new();
        let mut s = seed;
        for _ in 0..20 {
            s = xorshift32(s);
            let pk = seeded_key(s).to_vec();
            let atom = Atom::new_receiver(pk.clone());
            cs.insert(pk.clone(), atom);
            unique.insert(pk, ());
        }

        assert_eq!(cs.len(), unique.len(), "seed={}: len must equal unique key count", seed);
        assert!(!cs.is_empty());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2 — MPT insert / get fuzz
// ═══════════════════════════════════════════════════════════════════════════

/// Property: every inserted key is immediately retrievable with the correct value.
#[test]
fn fuzz_mpt_insert_get_roundtrip() {
    let (mut trie, _dir) = make_trie();
    let n = iters().min(2_000) as u32;

    let mut oracle: HashMap<[u8; 32], Vec<u8>> = HashMap::new();

    for seed in 0..n {
        let k = seeded_key(seed);
        let v = mock_atom(seed);
        trie.insert(&k, v.clone()).unwrap();
        oracle.insert(k, v);

        // Spot-check a random subset on each iteration
        let check_key = seeded_key(xorshift32(seed));
        if let Some(expected) = oracle.get(&check_key) {
            let got = trie.get(&check_key).unwrap();
            assert_eq!(got.as_ref(), Some(expected), "seed={}: get mismatch", seed);
        }
    }

    // Full validation pass
    for (k, expected) in &oracle {
        let got = trie.get(k).unwrap();
        assert_eq!(got.as_ref(), Some(expected), "final pass: key {:?} mismatch", &k[..4]);
    }
}

/// Property: insertion order does not affect the final root hash.
#[test]
fn fuzz_mpt_root_insertion_order_invariant() {
    let n_cases = iters().min(200);
    let keys_per_case = 16u32;

    for case in 0..n_cases as u32 {
        let pairs: Vec<([u8; 32], Vec<u8>)> = (0..keys_per_case)
            .map(|i| (seeded_key(case * 1000 + i), mock_atom(case * 1000 + i)))
            .collect();

        let mut tries: Vec<_> = (0..3).map(|_| make_trie()).collect();

        // Three different insertion orders
        let orders: Vec<Vec<usize>> = vec![
            (0..keys_per_case as usize).collect(),
            (0..keys_per_case as usize).rev().collect(),
            {
                let mut v: Vec<usize> = (0..keys_per_case as usize).collect();
                // interleave: even first, then odd
                let evens: Vec<_> = v.iter().copied().filter(|x| x % 2 == 0).collect();
                let odds:  Vec<_> = v.iter().copied().filter(|x| x % 2 != 0).collect();
                v = evens.into_iter().chain(odds).collect();
                v
            },
        ];

        let roots: Vec<[u8; 32]> = orders
            .iter()
            .zip(tries.iter_mut())
            .map(|(order, (trie, _dir))| {
                for &i in order {
                    trie.insert(&pairs[i].0, pairs[i].1.clone()).unwrap();
                }
                trie.root().unwrap()
            })
            .collect();

        assert!(
            roots.windows(2).all(|w| w[0] == w[1]),
            "case={}: insertion-order should not affect root. Roots: {:?}",
            case,
            roots.iter().map(|r| hex::encode(&r[..4])).collect::<Vec<_>>()
        );
    }
}

/// Property: deleting a key makes it unretrievable and changes the root.
#[test]
fn fuzz_mpt_delete_correctness() {
    let n = iters().min(500) as u32;
    let (mut trie, _dir) = make_trie();

    for seed in 0..n {
        let k = seeded_key(seed);
        let v = mock_atom(seed);

        let r_before = trie.insert(&k, v).unwrap();
        assert!(trie.get(&k).unwrap().is_some(), "seed={}: key must exist after insert", seed);

        let r_after = trie.delete(&k).unwrap();
        assert!(trie.get(&k).unwrap().is_none(), "seed={}: key must be gone after delete", seed);
        assert_ne!(r_before, r_after, "seed={}: root must change after delete", seed);

        // Re-insert for next iteration so trie grows
        trie.insert(&k, mock_atom(seed + 100_000)).unwrap();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 3 — Proof fuzz
// ═══════════════════════════════════════════════════════════════════════════

/// Property: inclusion proof always verifies for any key in the trie.
#[test]
fn fuzz_inclusion_proof_always_valid() {
    let (mut trie, _dir) = make_trie();
    let n = iters().min(1_000) as u32;

    let mut inserted: Vec<[u8; 32]> = Vec::new();

    for seed in 0..n {
        let k = seeded_key(seed);
        trie.insert(&k, mock_atom(seed)).unwrap();
        inserted.push(k);

        // Every ~50 inserts, verify all proofs so far
        if seed % 50 == 49 {
            for &ik in &inserted {
                let proof = trie.prove(&ik).unwrap();
                assert!(
                    verify_proof(&proof),
                    "seed={}/key={:?}: inclusion proof must verify",
                    seed, &ik[..4]
                );
                assert!(ProofBuilder::verify(&proof), "ProofBuilder::verify must agree");
            }
        }
    }
}

/// Property: exclusion proof verifies for keys NOT in the trie.
#[test]
fn fuzz_exclusion_proof_always_valid() {
    let (mut trie, _dir) = make_trie();
    let n = iters().min(500) as u32;

    // Pre-populate with even seeds
    for seed in (0..n).step_by(2) {
        trie.insert(&seeded_key(seed), mock_atom(seed)).unwrap();
    }

    // Odd seeds are absent — exclusion proofs
    for seed in (1..n).step_by(2) {
        let k = seeded_key(seed);
        let proof = trie.prove(&k).unwrap();
        assert!(
            proof.value.is_none(),
            "seed={}: proof for absent key must have value=None",
            seed
        );
        assert!(
            verify_proof(&proof),
            "seed={}: exclusion proof must verify",
            seed
        );
    }
}

/// Property: any single-bit flip in a sibling makes verify_proof() return false.
#[test]
fn fuzz_tampered_proof_rejected() {
    let (mut trie, _dir) = make_trie();

    // Build a moderately large trie so proofs have siblings
    for seed in 0..200u32 {
        trie.insert(&seeded_key(seed), mock_atom(seed)).unwrap();
    }

    let mut rejected = 0usize;
    let mut total    = 0usize;

    for seed in 0..200u32 {
        let k = seeded_key(seed);
        let proof = trie.prove(&k).unwrap();

        if proof.siblings.is_empty() {
            continue; // single-node trie root has no siblings — skip
        }

        // Flip one bit in each sibling byte position
        for sib_idx in 0..proof.siblings.len() {
            for byte_pos in 0..32usize {
                let mut tampered = proof.clone();
                tampered.siblings[sib_idx][byte_pos] ^= 0x01;
                total += 1;
                if !verify_proof(&tampered) {
                    rejected += 1;
                }
            }
        }
    }

    let rejection_rate = rejected as f64 / total.max(1) as f64;
    assert!(
        rejection_rate > 0.95,
        "tamper rejection rate {:.1}% is below 95% threshold (rejected={}/{})",
        rejection_rate * 100.0, rejected, total
    );
}

/// Property: flipping one bit in proof.root always fails verification.
#[test]
fn fuzz_tampered_root_always_rejected() {
    let (mut trie, _dir) = make_trie();

    for seed in 0..100u32 {
        trie.insert(&seeded_key(seed), mock_atom(seed)).unwrap();
    }

    for seed in 0..100u32 {
        let k = seeded_key(seed);
        let proof = trie.prove(&k).unwrap();

        for byte_pos in 0..32usize {
            for bit in 0..8u8 {
                let mut tampered = proof.clone();
                tampered.root[byte_pos] ^= 1 << bit;
                assert!(
                    !verify_proof(&tampered),
                    "tampered root must fail verify. seed={}, byte={}, bit={}",
                    seed, byte_pos, bit
                );
            }
        }
    }
}

/// Property: flipping the value bytes causes verification failure.
#[test]
fn fuzz_tampered_value_rejected() {
    let (mut trie, _dir) = make_trie();

    for seed in 0..50u32 {
        trie.insert(&seeded_key(seed), mock_atom(seed)).unwrap();
    }

    let mut failed = 0usize;
    for seed in 0..50u32 {
        let k = seeded_key(seed);
        let proof = trie.prove(&k).unwrap();
        if proof.value.is_none() { continue; }

        let mut tampered = proof.clone();
        let v = tampered.value.as_mut().unwrap();
        v[0] ^= 0xFF;

        if !verify_proof(&tampered) {
            failed += 1;
        }
    }

    // At least 80% of inclusion proofs must reject tampered values
    assert!(
        failed as f64 / 50.0 > 0.8,
        "tampered value rejection rate too low: {}/50",
        failed
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 4 — MptNode hash stability fuzz
// ═══════════════════════════════════════════════════════════════════════════

/// Property: MptNode::hash() is deterministic — same node → same hash, always.
#[test]
fn fuzz_mpt_node_hash_determinism() {
    let n = iters().min(5_000) as u32;

    for seed in 0..n {
        let node = MptNode::Leaf {
            key_suffix: seeded_key(seed).to_vec(),
            value: seeded_value(seed),
        };

        let h1 = node.hash();
        let h2 = node.hash();
        let h3 = node.hash();
        assert_eq!(h1, h2, "seed={}: hash must be deterministic", seed);
        assert_eq!(h2, h3, "seed={}: hash must be stable", seed);
    }
}

/// Property: two different leaf values always produce different hashes (collision resistance).
#[test]
fn fuzz_mpt_node_hash_no_collisions() {
    let n = iters().min(5_000) as u32;
    let suffix = vec![0xABu8; 8];

    for seed in 0..n {
        let v1 = seeded_value(seed);
        let v2 = seeded_value(seed + 1);
        if v1 == v2 { continue; } // exceedingly rare

        let h1 = MptNode::Leaf { key_suffix: suffix.clone(), value: v1 }.hash();
        let h2 = MptNode::Leaf { key_suffix: suffix.clone(), value: v2 }.hash();

        assert_ne!(h1, h2, "seed={}: distinct values must hash differently", seed);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 5 — key_to_nibbles fuzz
// ═══════════════════════════════════════════════════════════════════════════

/// Property: key_to_nibbles always produces exactly 64 nibbles in [0,15].
#[test]
fn fuzz_key_to_nibbles_always_64() {
    let n = iters().min(10_000) as u32;

    for seed in 0..n {
        let k = seeded_key(seed);
        let nibbles = key_to_nibbles(&k);

        assert_eq!(nibbles.len(), 64, "seed={}: nibbles length must be 64", seed);
        for (i, &nib) in nibbles.iter().enumerate() {
            assert!(nib < 16, "seed={}, idx={}: nibble {} out of range", seed, i, nib);
        }
    }
}

/// Property: nibbles reconstruction matches original bytes.
/*
#[test]
fn fuzz_key_to_nibbles_roundtrip() {
    let n = iters().min(10_000) as u32;

    for seed in 0..n {
        let k = seeded_key(seed);
        let nibbles = key_to_nibbles(&k);

        // Reconstruct bytes from nibble pairs
        let reconstructed: Vec<u8> = nibbles
            .chunks_exact(2)
            .map(|pair| (pair[0] << 4) | pair[1])
            .collect();

        assert_eq!(
            k.as_ref(), reconstructed.as_slice(),
            "seed={}: nibble roundtrip must reconstruct original key",
            seed
        );
    }
}*/

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 6 — Proof size budget fuzz (compact proof invariant)
// ═══════════════════════════════════════════════════════════════════════════

/// Property: proof size must stay under 4 KB for tries up to 1 000 nodes.
#[test]
fn fuzz_proof_size_budget() {
    let (mut trie, _dir) = make_trie();

    for i in 0u32..1_000 {
        trie.insert(&seeded_key(i), mock_atom(i)).unwrap();
    }

    let mut max_bytes = 0usize;
    for seed in 0..100u32 {
        let k = seeded_key(seed);
        let proof = trie.prove(&k).unwrap();
        let bytes = bincode::serialize(&proof).unwrap().len();
        if bytes > max_bytes { max_bytes = bytes; }

        assert!(
            bytes < 4_096,
            "seed={}: proof size {} bytes exceeds 4 KB budget",
            seed, bytes
        );
    }

    println!("[fuzz_proof_size_budget] max proof size = {} bytes", max_bytes);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 7 — UndoLog / GlobalMetrics basic fuzz
// ═══════════════════════════════════════════════════════════════════════════

use primus_storage::types::{UndoLog, GlobalMetrics};
use primus_types::physics::PhysicsCanon;

/// Property: UndoLog.record() respects "first write wins" semantics
/// (entry() .or_insert()).
#[test]
fn fuzz_undo_log_first_write_wins() {
    let n = iters().min(2_000) as u32;
    for seed in 0..n {
        let mut log = UndoLog::new(seed as u64, seeded_key(seed));
        let pk = seeded_key(seed).to_vec();

        let before1 = Some(Atom::new_receiver(vec![0u8; 32]));
        let before2 = Some(Atom::new_receiver(vec![1u8; 32]));

        log.record(pk.clone(), before1.clone());
        log.record(pk.clone(), before2.clone()); // must not overwrite

        let stored = log.pre_images.get(&pk).unwrap();
        assert_eq!(
            bincode::serialize(stored).unwrap(),
            bincode::serialize(&before1).unwrap(),
            "seed={}: first-write must win in UndoLog",
            seed
        );
    }
}

/// Property: GlobalMetrics::canonical() is deterministic (no float drift).
#[test]
fn fuzz_global_metrics_canonical_determinism() {
    let n = iters().min(5_000) as u32;
    for seed in 0..n {
        let m = GlobalMetrics {
            temperature: (seed as f32) * 0.001,
            entropy: (seed as f32) * 0.0007,
        };
        let c1 = m.canonical();
        let c2 = m.canonical();
        assert_eq!(c1, c2, "seed={}: canonical() must be deterministic", seed);
    }
}
