use criterion::{black_box, criterion_group, criterion_main, Criterion};
use primus_vm::pvm::PVM;
use primus_vm::context::{CryptoVerifier, ExecutionContext, StateView};
use primus_types::atom::{Atom, Element, QuantumState};
use primus_types::payload::Payload;
use primus_types::reaction::SignedReaction;
use primus_storage::Changeset;
use std::collections::{HashMap, BTreeMap};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier, Signature};
use rand::rngs::OsRng;
use rand::RngCore;

// ── Real Implementation (ED25519 Proxy) ──────────────────────────────────────

pub struct RealCryptoVerifier;

impl CryptoVerifier for RealCryptoVerifier {
    fn verify(pk: &[u8], digest: &[u8], sig: &[u8]) -> bool {
        let Ok(pk_arr) = <[u8; 32]>::try_from(pk) else { return false; };
        let Ok(verifying_key) = VerifyingKey::from_bytes(&pk_arr) else {
            return false;
        };
        let Ok(sig_arr) = <[u8; 64]>::try_from(sig) else { return false; };
        let signature = Signature::from_bytes(&sig_arr);
        verifying_key.verify(digest, &signature).is_ok()
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

fn create_realistic_transfer(
    sender_sk: &SigningKey,
    receiver_pk: Vec<u8>,
    amount: u64,
    nonce: u64
) -> SignedReaction {
    let sender_pk = sender_sk.verifying_key().to_bytes().to_vec();
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
        signature: vec![],
        payload: Payload::Transfer { amount },
    };
    rx.reaction_hash = rx.compute_reaction_hash();
    let sig = sender_sk.sign(&rx.reaction_hash);
    rx.signature = sig.to_bytes().to_vec();
    rx
}

// ── Benchmarks ───────────────────────────────────────────────────────────────

fn bench_realistic_pvm(c: &mut Criterion) {
    let mut rng = OsRng;
    
    let mut arch_bytes = [0u8; 32]; rng.fill_bytes(&mut arch_bytes);
    let architect_sk = SigningKey::from_bytes(&arch_bytes);
    let architect_pk = architect_sk.verifying_key().to_bytes().to_vec();
    
    let mut sender_bytes = [0u8; 32]; rng.fill_bytes(&mut sender_bytes);
    let sender_sk = SigningKey::from_bytes(&sender_bytes);
    let sender_pk = sender_sk.verifying_key().to_bytes().to_vec();
    
    let mut receiver_bytes = [0u8; 32]; rng.fill_bytes(&mut receiver_bytes);
    let receiver_sk = SigningKey::from_bytes(&receiver_bytes);
    let receiver_pk = receiver_sk.verifying_key().to_bytes().to_vec();

    let mut state = MockStateView::new();
    state.atoms.insert(sender_pk.clone(), Atom {
        public_key: sender_pk.clone(),
        element: Element::Hydrogen,
        neutron_count: 0,
        mass: 10_000_000,
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

    let ctx = ExecutionContext::<RealCryptoVerifier> {
        state: &state,
        architect_pk: &architect_pk,
        current_temp: -100.0,
        crystal_index: 100,
        wasm_runtime: None,
        _crypto: std::marker::PhantomData,
    };

    let atoms_iter = BTreeMap::new();
    let galactic_drift = 0;

    let mut group = c.benchmark_group("realistic");

    // "realistic/transfer_ok_1rx"
    let rx_ok = create_realistic_transfer(&sender_sk, receiver_pk.clone(), 500, 10);
    group.bench_function("transfer_ok_1rx", |b| {
        b.iter(|| {
            let mut changeset = Changeset::new();
            let mut heat = 0.0;
            let mut entropy = 0.0;
            PVM::execute_single::<RealCryptoVerifier>(
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

    // "realistic/transfer_ok_batch_10"
    let batch_10: Vec<SignedReaction> = (0..10)
        .map(|i| create_realistic_transfer(&sender_sk, receiver_pk.clone(), 10, 10 + i))
        .collect();

    group.bench_function("transfer_ok_batch_10", |b| {
        b.iter(|| {
            let _ = PVM::execute_payload::<RealCryptoVerifier>(
                black_box(&ctx),
                black_box(&batch_10),
                &atoms_iter,
            ).unwrap();
        })
    });

    // "realistic/transfer_ok_batch_100"
    let batch_100: Vec<SignedReaction> = (0..100)
        .map(|i| create_realistic_transfer(&sender_sk, receiver_pk.clone(), 10, 10 + i))
        .collect();

    group.bench_function("transfer_ok_batch_100", |b| {
        b.iter(|| {
            let _ = PVM::execute_payload::<RealCryptoVerifier>(
                black_box(&ctx),
                black_box(&batch_100),
                &atoms_iter,
            ).unwrap();
        })
    });

    // "realistic/verify_twice_per_rx"
    let digest = [0u8; 32];
    let sig = sender_sk.sign(&digest).to_bytes().to_vec();
    group.bench_function("verify_twice_per_rx", |b| {
        b.iter(|| {
            // owner verify
            let v1 = RealCryptoVerifier::verify(black_box(&sender_pk), black_box(&digest), black_box(&sig));
            // architect fallback verify
            let v2 = RealCryptoVerifier::verify(black_box(&architect_pk), black_box(&digest), black_box(&sig));
            black_box(v1);
            black_box(v2);
        })
    });

    // "realistic/mining_reward"
    let mut rx_reward = SignedReaction {
        sender: Atom::new_receiver(vec![0; 32]),
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
            PVM::execute_single::<RealCryptoVerifier>(
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

    group.finish();
}

criterion_group!(benches, bench_realistic_pvm);
criterion_main!(benches);
