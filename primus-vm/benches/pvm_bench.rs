use criterion::{black_box, criterion_group, criterion_main, Criterion};
use primus_vm::pvm::PVM;
use primus_vm::context::{CryptoVerifier, ExecutionContext, StateView};
use primus_types::atom::{Atom, Element, QuantumState};
use primus_types::payload::Payload;
use primus_types::reaction::SignedReaction;
use primus_storage::Changeset;
use std::collections::{HashMap, BTreeMap};

// ── Mock Implementation ──────────────────────────────────────────────────────

pub struct MockCryptoVerifier;

impl CryptoVerifier for MockCryptoVerifier {
    fn verify(pk: &[u8], _digest: &[u8], sig: &[u8]) -> bool {
        // Simple mock: if sig matches pk, it's valid.
        sig == pk
    }
}

pub struct MockStateView {
    pub atoms: HashMap<Vec<u8>, Atom>,
    pub index: u64,
}

impl MockStateView {
    pub fn new() -> Self {
        Self {
            atoms: HashMap::new(),
            index: 1,
        }
    }
}

impl StateView for MockStateView {
    fn get_atom(&self, pk: &[u8]) -> Option<Atom> {
        self.atoms.get(pk).cloned()
    }
    fn crystal_index(&self) -> u64 { self.index }
    fn load_contract(&self, _code_hash: [u8; 32]) -> Option<Vec<u8>> { None }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn create_valid_transfer(sender_pk: Vec<u8>, receiver_pk: Vec<u8>, amount: u64, nonce: u64) -> SignedReaction {
    let mut rx = SignedReaction {
        sender: Atom {
            public_key: sender_pk.clone(),
            element: Element::Hydrogen,
            neutron_count: 0,
            mass: 100_000,
            charge: 2.2,
            last_reaction_hash: [0; 32],
            last_active_index: 0,
            nonce,
            quantum_state: QuantumState::Stable,
        },
        receiver: Atom::new_receiver(receiver_pk),
        reaction_hash: [0; 32],
        energy: 10.0,
        timestamp: 0,
        signature: sender_pk, // MockCryptoVerifier accepts pk as sig
        payload: Payload::Transfer { amount },
    };
    rx.reaction_hash = rx.compute_reaction_hash();
    rx
}

fn create_valid_generic(sender_pk: Vec<u8>, receiver_pk: Vec<u8>, nonce: u64) -> SignedReaction {
    let mut rx = SignedReaction {
        sender: Atom {
            public_key: sender_pk.clone(),
            element: Element::Hydrogen,
            neutron_count: 0,
            mass: 100_000,
            charge: 2.2,
            last_reaction_hash: [0; 32],
            last_active_index: 0,
            nonce,
            quantum_state: QuantumState::Stable,
        },
        receiver: Atom::new_receiver(receiver_pk),
        reaction_hash: [0; 32],
        energy: 10.0,
        timestamp: 0,
        signature: sender_pk,
        payload: Payload::Generic,
    };
    rx.reaction_hash = rx.compute_reaction_hash();
    rx
}

// ── Benchmarks ───────────────────────────────────────────────────────────────

fn bench_execute_single(c: &mut Criterion) {
    let architect_pk = vec![0xCC; 2592];
    let sender_pk = vec![0xAA; 2592];
    let receiver_pk = vec![0xBB; 2592];

    let mut state = MockStateView::new();
    state.atoms.insert(sender_pk.clone(), Atom {
        public_key: sender_pk.clone(),
        element: Element::Hydrogen,
        neutron_count: 0,
        mass: 1_000_000,
        charge: 2.2,
        last_reaction_hash: [0; 32],
        last_active_index: 0,
        nonce: 10,
        quantum_state: QuantumState::Stable,
    });
    state.atoms.insert(architect_pk.clone(), Atom {
        public_key: architect_pk.clone(),
        element: Element::Hydrogen,
        neutron_count: 0,
        mass: 1_000_000,
        charge: 2.2,
        last_reaction_hash: [0; 32],
        last_active_index: 0,
        nonce: 0,
        quantum_state: QuantumState::Stable,
    });

    let ctx = ExecutionContext::<MockCryptoVerifier> {
        state: &state,
        architect_pk: &architect_pk,
        current_temp: -100.0,
        crystal_index: 100,
        wasm_runtime: None,
        _crypto: std::marker::PhantomData,
    };

    let atoms_iter = BTreeMap::new(); // empty stars
    let galactic_drift = 0;

    let mut group = c.benchmark_group("execute_single");

    // Ok path
    let rx_ok = create_valid_transfer(sender_pk.clone(), receiver_pk.clone(), 500, 10);
    group.bench_function("transfer_ok", |b| {
        b.iter(|| {
            let mut changeset = Changeset::new();
            let mut heat = 0.0;
            let mut entropy = 0.0;
            PVM::execute_single::<MockCryptoVerifier>(
                black_box(&ctx),
                black_box(&rx_ok),
                &mut changeset,
                &atoms_iter,
                &mut heat,
                &mut entropy,
                galactic_drift,
                &[],
            ).unwrap();
        })
    });

    // Insufficient mass
    let rx_poor = create_valid_transfer(sender_pk.clone(), receiver_pk.clone(), 2_000_000, 10);
    group.bench_function("transfer_insufficient", |b| {
        b.iter(|| {
            let mut changeset = Changeset::new();
            let mut heat = 0.0;
            let mut entropy = 0.0;
            let _ = PVM::execute_single::<MockCryptoVerifier>(
                black_box(&ctx),
                black_box(&rx_poor),
                &mut changeset,
                &atoms_iter,
                &mut heat,
                &mut entropy,
                galactic_drift,
                &[],
            );
        })
    });

    // Nonce mismatch
    let rx_bad_nonce = create_valid_transfer(sender_pk.clone(), receiver_pk.clone(), 500, 999);
    group.bench_function("nonce_mismatch", |b| {
        b.iter(|| {
            let mut changeset = Changeset::new();
            let mut heat = 0.0;
            let mut entropy = 0.0;
            let _ = PVM::execute_single::<MockCryptoVerifier>(
                black_box(&ctx),
                black_box(&rx_bad_nonce),
                &mut changeset,
                &atoms_iter,
                &mut heat,
                &mut entropy,
                galactic_drift,
                &[],
            );
        })
    });

    // Generic Ok
    let rx_gen = create_valid_generic(sender_pk.clone(), receiver_pk.clone(), 10);
    group.bench_function("generic_ok", |b| {
        b.iter(|| {
            let mut changeset = Changeset::new();
            let mut heat = 0.0;
            let mut entropy = 0.0;
            PVM::execute_single::<MockCryptoVerifier>(
                black_box(&ctx),
                black_box(&rx_gen),
                &mut changeset,
                &atoms_iter,
                &mut heat,
                &mut entropy,
                galactic_drift,
                &[],
            ).unwrap();
        })
    });

    // Mining Reward
    let mut rx_reward = SignedReaction {
        sender: Atom::new_receiver(vec![0; 2592]),
        receiver: Atom {
            public_key: architect_pk.clone(),
            element: Element::Hydrogen,
            neutron_count: 0,
            mass: 0,
            charge: 2.2,
            last_reaction_hash: [0; 32],
            last_active_index: 0,
            nonce: 0,
            quantum_state: QuantumState::Stable,
        },
        reaction_hash: [0; 32],
        energy: 0.0,
        timestamp: 0,
        signature: vec![],
        payload: Payload::MiningReward { amount: 10 },
    };
    rx_reward.reaction_hash = rx_reward.compute_reaction_hash();

    group.bench_function("mining_reward", |b| {
        b.iter(|| {
            let mut changeset = Changeset::new();
            let mut heat = 0.0;
            let mut entropy = 0.0;
            PVM::execute_single::<MockCryptoVerifier>(
                black_box(&ctx),
                black_box(&rx_reward),
                &mut changeset,
                &atoms_iter,
                &mut heat,
                &mut entropy,
                galactic_drift,
                &[],
            ).unwrap();
        })
    });

    // Negative energy
    let mut rx_neg = rx_ok.clone();
    rx_neg.energy = -1.0;
    group.bench_function("negative_energy", |b| {
        b.iter(|| {
            let mut changeset = Changeset::new();
            let mut heat = 0.0;
            let mut entropy = 0.0;
            let _ = PVM::execute_single::<MockCryptoVerifier>(
                black_box(&ctx),
                black_box(&rx_neg),
                &mut changeset,
                &atoms_iter,
                &mut heat,
                &mut entropy,
                galactic_drift,
                &[],
            );
        })
    });

    group.finish();
}

fn bench_execute_batch(c: &mut Criterion) {
    let architect_pk = vec![0xCC; 2592];
    let sender_pk = vec![0xAA; 2592];
    let receiver_pk = vec![0xBB; 2592];

    let mut state = MockStateView::new();
    state.atoms.insert(sender_pk.clone(), Atom {
        public_key: sender_pk.clone(),
        element: Element::Hydrogen,
        neutron_count: 0,
        mass: 10_000_000,
        charge: 2.2,
        last_reaction_hash: [0; 32],
        last_active_index: 0,
        nonce: 0,
        quantum_state: QuantumState::Stable,
    });
    state.atoms.insert(architect_pk.clone(), Atom {
        public_key: architect_pk.clone(),
        element: Element::Hydrogen,
        neutron_count: 0,
        mass: 1_000_000,
        charge: 2.2,
        last_reaction_hash: [0; 32],
        last_active_index: 0,
        nonce: 0,
        quantum_state: QuantumState::Stable,
    });

    let ctx = ExecutionContext::<MockCryptoVerifier> {
        state: &state,
        architect_pk: &architect_pk,
        current_temp: -100.0,
        crystal_index: 100,
        wasm_runtime: None,
        _crypto: std::marker::PhantomData,
    };

    let atoms_iter = BTreeMap::new();

    let mut group = c.benchmark_group("execute_payload");

    // Batch 10
    let batch_10: Vec<SignedReaction> = (0..10)
        .map(|i| create_valid_transfer(sender_pk.clone(), receiver_pk.clone(), 10, i))
        .collect();

    group.bench_function("batch_10", |b| {
        b.iter(|| {
            let _ = PVM::execute_payload::<MockCryptoVerifier>(
                black_box(&ctx),
                black_box(&batch_10),
                &atoms_iter,
            ).unwrap();
        })
    });

    // Batch 100
    let batch_100: Vec<SignedReaction> = (0..100)
        .map(|i| create_valid_transfer(sender_pk.clone(), receiver_pk.clone(), 10, i))
        .collect();

    group.bench_function("batch_100", |b| {
        b.iter(|| {
            let _ = PVM::execute_payload::<MockCryptoVerifier>(
                black_box(&ctx),
                black_box(&batch_100),
                &atoms_iter,
            ).unwrap();
        })
    });

    group.finish();
}

criterion_group!(benches, bench_execute_single, bench_execute_batch);
criterion_main!(benches);
