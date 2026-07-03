// =============================================================================
// primus-vm/tests/pvm_integration.rs — PVM Integration Tests
// =============================================================================

use primus_vm::pvm::PVM;
use primus_vm::context::{CryptoVerifier, StateView, ExecutionContext};
use primus_types::atom::{Atom, Element, QuantumState};
use primus_types::reaction::SignedReaction;
use primus_types::payload::Payload;
use primus_vm::error::PvmError;
use std::collections::BTreeMap;

struct MockCrypto;
impl CryptoVerifier for MockCrypto {
    fn verify(_pk: &[u8], _digest: &[u8], _sig: &[u8]) -> bool {
        // In mock mode, we assume all signatures are valid if they are not empty
        !_sig.is_empty()
    }
}

struct MockState {
    atoms: BTreeMap<Vec<u8>, Atom>,
}

impl StateView for MockState {
    fn get_atom(&self, pk: &[u8]) -> Option<Atom> {
        self.atoms.get(pk).cloned()
    }

    fn crystal_index(&self) -> u64 {
        100
    }

    fn load_contract(&self, _code_hash: [u8; 32]) -> Option<Vec<u8>> {
        None
    }
}

fn make_test_atom(pk: Vec<u8>, mass: u64, nonce: u64) -> Atom {
    Atom {
        public_key: pk,
        mass,
        nonce,
        last_reaction_hash: [0u8; 32],
        last_active_index: 0,
        element: Element::Hydrogen,
        charge: 0.0,
        neutron_count: 1,
        quantum_state: QuantumState::Stable,
    }
}

#[test]
fn test_simple_transfer() {
    let alice_pk = vec![1u8; 32];
    let bob_pk = vec![2u8; 32];
    let arch_pk = vec![0u8; 32];

    let alice = make_test_atom(alice_pk.clone(), 1000, 0);
    let bob = make_test_atom(bob_pk.clone(), 500, 0);

    let mut state = MockState {
        atoms: BTreeMap::new(),
    };
    state.atoms.insert(alice_pk.clone(), alice.clone());
    state.atoms.insert(bob_pk.clone(), bob.clone());

    let ctx = ExecutionContext::<MockCrypto> {
        state: &state,
        architect_pk: &arch_pk,
        current_temp: 20.0,
        crystal_index: 100,
        wasm_runtime: None,
        _crypto: std::marker::PhantomData,
    };

    let rx = SignedReaction {
        sender: alice,
        receiver: bob,
        reaction_hash: [0xaa; 32],
        energy: 10.0,
        timestamp: 123456789,
        signature: vec![0xff; 64],
        payload: Payload::Transfer { amount: 100 },
    };

    let reactions = vec![rx];
    let (changeset, heat) = PVM::execute_payload::<MockCrypto>(&ctx, &reactions, &state.atoms).unwrap();

    assert!(heat > 0.0);
    let alice_new = changeset.inner.get(&alice_pk).expect("Alice not in changeset");
    let bob_new = changeset.inner.get(&bob_pk).expect("Bob not in changeset");

    assert_eq!(alice_new.mass, 1000 - 100 - 10 - 1); // mass - amount - fee - decay(1)
    assert_eq!(bob_new.mass, 500 + 100);
    assert_eq!(alice_new.nonce, 1);
}

#[test]
fn test_insufficient_mass() {
    let alice_pk = vec![1u8; 32];
    let bob_pk = vec![2u8; 32];
    let arch_pk = vec![0u8; 32];

    let alice = make_test_atom(alice_pk.clone(), 50, 0);
    let bob = make_test_atom(bob_pk.clone(), 500, 0);

    let mut state = MockState {
        atoms: BTreeMap::new(),
    };
    state.atoms.insert(alice_pk.clone(), alice.clone());
    state.atoms.insert(bob_pk.clone(), bob.clone());

    let ctx = ExecutionContext::<MockCrypto> {
        state: &state,
        architect_pk: &arch_pk,
        current_temp: 20.0,
        crystal_index: 100,
        wasm_runtime: None,
        _crypto: std::marker::PhantomData,
    };

    let rx = SignedReaction {
        sender: alice,
        receiver: bob,
        reaction_hash: [0xaa; 32],
        energy: 100.0, // Fee more than balance
        timestamp: 123456789,
        signature: vec![0xff; 64],
        payload: Payload::Transfer { amount: 100 },
    };

    let reactions = vec![rx];
    let res = PVM::execute_payload::<MockCrypto>(&ctx, &reactions, &state.atoms);
    
    assert!(matches!(res, Err(PvmError::InsufficientMass { .. })));
}

#[test]
fn test_nonce_mismatch() {
    let alice_pk = vec![1u8; 32];
    let bob_pk = vec![2u8; 32];
    let arch_pk = vec![0u8; 32];

    let alice_on_chain = make_test_atom(alice_pk.clone(), 1000, 5); // on-chain nonce is 5
    let alice_tx = make_test_atom(alice_pk.clone(), 1000, 0); // tx has nonce 0
    let bob = make_test_atom(bob_pk.clone(), 500, 0);

    let mut state = MockState {
        atoms: BTreeMap::new(),
    };
    state.atoms.insert(alice_pk.clone(), alice_on_chain);
    state.atoms.insert(bob_pk.clone(), bob.clone());

    let ctx = ExecutionContext::<MockCrypto> {
        state: &state,
        architect_pk: &arch_pk,
        current_temp: 20.0,
        crystal_index: 100,
        wasm_runtime: None,
        _crypto: std::marker::PhantomData,
    };

    let rx = SignedReaction {
        sender: alice_tx,
        receiver: bob,
        reaction_hash: [0xaa; 32],
        energy: 10.0,
        timestamp: 123456789,
        signature: vec![0xff; 64],
        payload: Payload::Transfer { amount: 100 },
    };

    let reactions = vec![rx];
    let res = PVM::execute_payload::<MockCrypto>(&ctx, &reactions, &state.atoms);
    
    assert!(matches!(res, Err(PvmError::NonceMismatch { .. })));
}
