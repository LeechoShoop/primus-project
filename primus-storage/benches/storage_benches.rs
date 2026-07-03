use criterion::{black_box, criterion_group, criterion_main, Criterion, BatchSize};
use primus_storage::{Changeset, MerklePatriciaTrie};
use primus_storage::mpt::{MptNode, verify_proof};
use primus_storage::mpt_store::SledMptStore;
use primus_storage::mempool_v2::SectoralMempool;
use primus_types::atom::Atom;
use primus_types::reaction::SignedReaction;
use primus_types::payload::Payload;
use std::collections::BTreeMap;

fn temp_db() -> (tempfile::TempDir, sled::Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = sled::open(dir.path()).unwrap();
    (dir, db)
}

fn seeded_key(seed: u32) -> Vec<u8> {
    let mut key = vec![0u8; 2592];
    let mut s = seed;
    for chunk in key.chunks_mut(4) {
        s = s ^ (s << 13);
        s = s ^ (s >> 17);
        s = s ^ (s << 5);
        chunk.copy_from_slice(&s.to_le_bytes());
    }
    key
}

fn seeded_hash(seed: u32) -> [u8; 32] {
    let mut h = [0u8; 32];
    let mut s = seed;
    for chunk in h.chunks_mut(4) {
        s = s ^ (s << 13);
        s = s ^ (s >> 17);
        s = s ^ (s << 5);
        chunk.copy_from_slice(&s.to_le_bytes());
    }
    h
}

fn mock_atom(seed: u32) -> Vec<u8> {
    let pk = seeded_key(seed);
    let atom = Atom::new_receiver(pk);
    bincode::serialize(&atom).unwrap()
}

fn mock_reaction(seed: u32) -> SignedReaction {
    let mut rx = SignedReaction {
        sender: Atom::new_receiver(seeded_key(seed)),
        receiver: Atom::new_receiver(seeded_key(seed + 1)),
        reaction_hash: [0u8; 32],
        energy: 10.0,
        timestamp: 123456789,
        signature: vec![],
        payload: Payload::MiningReward { amount: 1000 },
    };
    rx.reaction_hash = rx.compute_reaction_hash();
    rx
}

// ── MPT Benches ─────────────────────────────────────────────────────────────

fn bench_mpt_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("mpt_insert");
    for size in [1, 100, 1000].iter() {
        group.bench_with_input(criterion::BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter_batched(
                || {
                    let (_dir, db) = temp_db();
                    let store = SledMptStore::new(&db).unwrap();
                    let mut trie = MerklePatriciaTrie::new(store);
                    for i in 0..size {
                        trie.insert(&seeded_hash(i as u32), mock_atom(i as u32)).unwrap();
                    }
                    (db, trie)
                },
                |(_db, mut trie)| {
                    trie.insert(&seeded_hash(size as u32 + 1), mock_atom(size as u32 + 1)).unwrap();
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_mpt_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("mpt_get");
    let (_dir, db) = temp_db();
    let store = SledMptStore::new(&db).unwrap();
    let mut trie = MerklePatriciaTrie::new(store);
    for i in 0..1000 {
        trie.insert(&seeded_hash(i as u32), mock_atom(i as u32)).unwrap();
    }

    group.bench_function("hit", |b| {
        b.iter(|| {
            black_box(trie.get(black_box(&seeded_hash(500))).unwrap());
        });
    });
    group.bench_function("miss", |b| {
        b.iter(|| {
            black_box(trie.get(black_box(&seeded_hash(2000))).unwrap());
        });
    });
    group.finish();
}

fn bench_mpt_prove(c: &mut Criterion) {
    let (_dir, db) = temp_db();
    let store = SledMptStore::new(&db).unwrap();
    let mut trie = MerklePatriciaTrie::new(store);
    for i in 0..1000 {
        trie.insert(&seeded_hash(i as u32), mock_atom(i as u32)).unwrap();
    }
    c.bench_function("mpt_prove", |b| {
        b.iter(|| {
            black_box(trie.prove(black_box(&seeded_hash(500))).unwrap());
        });
    });
}

fn bench_verify_proof(c: &mut Criterion) {
    let (_dir, db) = temp_db();
    let store = SledMptStore::new(&db).unwrap();
    let mut trie = MerklePatriciaTrie::new(store);
    for i in 0..1000 {
        trie.insert(&seeded_hash(i as u32), mock_atom(i as u32)).unwrap();
    }
    let proof = trie.prove(&seeded_hash(500)).unwrap();
    c.bench_function("verify_proof", |b| {
        b.iter(|| {
            black_box(verify_proof(black_box(&proof)));
        });
    });
}

fn bench_mpt_delete(c: &mut Criterion) {
    c.bench_function("mpt_delete", |b| {
        b.iter_batched(
            || {
                let (_dir, db) = temp_db();
                let store = SledMptStore::new(&db).unwrap();
                let mut trie = MerklePatriciaTrie::new(store);
                for i in 0..1000 {
                    trie.insert(&seeded_hash(i as u32), mock_atom(i as u32)).unwrap();
                }
                (db, trie)
            },
            |(_db, mut trie)| {
                trie.delete(black_box(&seeded_hash(500))).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_gc_since(c: &mut Criterion) {
    c.bench_function("gc_since", |b| {
        b.iter_batched(
            || {
                let (_dir, db) = temp_db();
                let store = SledMptStore::new(&db).unwrap();
                let mut trie = MerklePatriciaTrie::new(store);
                for i in 0..500 {
                    trie.insert(&seeded_hash(i as u32), mock_atom(i as u32)).unwrap();
                }
                let old_root = trie.root().unwrap();
                for i in 500..1000 {
                    trie.insert(&seeded_hash(i as u32), mock_atom(i as u32)).unwrap();
                }
                (db, trie, old_root)
            },
            |(_db, mut trie, old_root)| {
                trie.gc_since(black_box(old_root)).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

// ── Changeset Benches ───────────────────────────────────────────────────────

fn bench_changeset_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("changeset_insert");
    group.bench_function("x1", |b| {
        b.iter_batched(
            || (Changeset::new(), seeded_hash(1).to_vec(), Atom::new_receiver(vec![0; 2592])),
            |(mut cs, key, atom)| {
                cs.insert(key, atom);
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("x1000", |b| {
        b.iter_batched(
            || {
                let mut cs = Changeset::new();
                for i in 0..1000 {
                    cs.insert(seeded_hash(i as u32).to_vec(), Atom::new_receiver(seeded_key(i as u32)));
                }
                (cs, seeded_hash(1001).to_vec(), Atom::new_receiver(seeded_key(1002)))
            },
            |(mut cs, key, atom)| {
                cs.insert(key, atom);
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_changeset_get(c: &mut Criterion) {
    let mut cs = Changeset::new();
    for i in 0..1000 {
        cs.insert(seeded_hash(i as u32).to_vec(), Atom::new_receiver(seeded_key(i as u32)));
    }
    let key = seeded_hash(500).to_vec();
    c.bench_function("changeset_get", |b| {
        b.iter(|| {
            black_box(cs.get(black_box(&key)));
        });
    });
}

// ── Mempool Benches ─────────────────────────────────────────────────────────

fn bench_mempool(c: &mut Criterion) {
    let (_dir, db) = temp_db();
    let mempool = SectoralMempool::new(&db).unwrap();
    let rx = mock_reaction(123);
    let rx_bytes = rkyv::to_bytes::<SignedReaction, 256>(&rx).unwrap().into_vec();

    c.bench_function("mempool_push_bytes", |b| {
        b.iter(|| {
            black_box(mempool.push_bytes(black_box(&rx_bytes))).unwrap();
        });
    });

    c.bench_function("mempool_push", |b| {
        b.iter(|| {
            black_box(mempool.push(black_box(rx.clone()))).unwrap();
        });
    });
}

// ── Node Hash Benches ───────────────────────────────────────────────────────

fn bench_node_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_hash");
    let leaf = MptNode::Leaf {
        key_suffix: vec![1, 2, 3, 4],
        value: vec![5; 32],
    };
    let branch = MptNode::Branch {
        children: Box::new([
            Some([1; 32]), None, Some([2; 32]), None,
            None, None, None, None,
            None, None, None, None,
            None, None, None, Some([15; 32]),
        ]),
        value: Some(vec![9; 32]),
    };

    group.bench_function("leaf", |b| {
        b.iter(|| {
            black_box(leaf.hash());
        });
    });
    group.bench_function("branch", |b| {
        b.iter(|| {
            black_box(branch.hash());
        });
    });
    group.finish();
}

#[test]
fn run_benchmarks() {
    let mut c = Criterion::default();
    bench_mpt_insert(&mut c);
    bench_mpt_get(&mut c);
    bench_mpt_prove(&mut c);
    bench_verify_proof(&mut c);
    bench_mpt_delete(&mut c);
    bench_gc_since(&mut c);
    bench_changeset_insert(&mut c);
    bench_changeset_get(&mut c);
    bench_mempool(&mut c);
    bench_node_hash(&mut c);
}

fn main() {
    let mut c = Criterion::default().configure_from_args();
    bench_mpt_insert(&mut c);
    bench_mpt_get(&mut c);
    bench_mpt_prove(&mut c);
    bench_verify_proof(&mut c);
    bench_mpt_delete(&mut c);
    bench_gc_since(&mut c);
    bench_changeset_insert(&mut c);
    bench_changeset_get(&mut c);
    bench_mempool(&mut c);
    bench_node_hash(&mut c);
}
