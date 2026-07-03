#![allow(unexpected_cfgs)]
#![cfg_attr(fuzzing, no_main)]
use libfuzzer_sys::fuzz_target;
use primus_vm::*;
use primus_vm::context::{CryptoVerifier, StateView};
use primus_types::atom::{Atom, Element};
use primus_types::reaction::SignedReaction;
use primus_types::payload::Payload;
use primus_storage::changeset::Changeset;
use std::collections::{BTreeMap, HashMap};
use arbitrary::Arbitrary;

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    sender_pk: [u8; 32],
    receiver_pk: [u8; 32],
    energy: f32,
    nonce: u64,
    amount: u64,
    current_temp: f32,
    galactic_drift: u8,
}

struct FuzzCrypto;
impl CryptoVerifier for FuzzCrypto {
    fn verify(_pk: &[u8], _digest: &[u8], sig: &[u8]) -> bool {
        sig == b"fuzz_sig"
    }
}

struct FuzzState {
    atoms: HashMap<Vec<u8>, Atom>,
}
impl StateView for FuzzState {
    fn get_atom(&self, pk: &[u8]) -> Option<Atom> {
        self.atoms.get(pk).cloned()
    }
    fn crystal_index(&self) -> u64 { 1 }
    fn load_contract(&self, _hash: [u8; 32]) -> Option<Vec<u8>> { None }
}

// ATTACK SURFACE: PVM::execute_single — full pipeline
// INVARIANT: No panic, no UB. changeset consistent or empty on error.
// SEVERITY: Critical
fuzz_target!(|input: FuzzInput| {
    let mut state = FuzzState { atoms: HashMap::new() };
    let mut sender = Atom::new_materialized(input.sender_pk.to_vec(), Element::Hydrogen);
    sender.mass = 1_000_000_000;
    sender.nonce = input.nonce;
    state.atoms.insert(input.sender_pk.to_vec(), sender.clone());

    let mut rx = SignedReaction {
        sender,
        receiver: Atom::new_materialized(input.receiver_pk.to_vec(), Element::Hydrogen),
        payload: Payload::Transfer { amount: input.amount },
        energy: input.energy,
        signature: b"fuzz_sig".to_vec(),
        reaction_hash: [0; 32],
        timestamp: 0,
    };
    rx.sender.nonce = input.nonce;

    let ctx = ExecutionContext::<FuzzCrypto> {
        state: &state,
        architect_pk: &[0; 32],
        current_temp: input.current_temp,
        crystal_index: 1,
        wasm_runtime: None,
        _crypto: std::marker::PhantomData,
    };

    let mut changeset = Changeset::new();
    let mut heat = 0.0;
    let mut entropy = 0.0;
    let atoms_iter = BTreeMap::new();

    let res = PVM::execute_single::<FuzzCrypto>(
        &ctx, &rx, &mut changeset, &atoms_iter, &mut heat, &mut entropy, input.galactic_drift, &[]
    );

    if res.is_ok() {
        if let Some(s) = changeset.inner.get(input.sender_pk.as_slice()) {
            assert!(s.mass <= 1_000_000_000);
        }
    } else {
        assert!(changeset.is_empty());
    }
});

#[cfg(not(fuzzing))]
fn main() {}
