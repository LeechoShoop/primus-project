use primus_vm::*;
use primus_vm::context::{CryptoVerifier, StateView};
use primus_types::atom::{Atom, Element};
use primus_types::reaction::SignedReaction;
use primus_types::payload::Payload;
use primus_storage::Changeset;
use std::collections::{BTreeMap, HashMap};
use proptest::prelude::*;

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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10000))]
    #[test]
    fn prop_fuzz_harness_shim(
        sender_pk in prop::array::uniform32(0u8..255),
        receiver_pk in prop::array::uniform32(0u8..255),
        energy in prop::num::f32::ANY,
        nonce in 0u64..u64::MAX,
        amount in 0u64..u64::MAX,
        current_temp in prop::num::f32::ANY,
        galactic_drift in 0u8..255,
    ) {
        let mut state = FuzzState { atoms: HashMap::new() };
        let mut sender = Atom::new_materialized(sender_pk.to_vec(), Element::Hydrogen);
        sender.mass = 1_000_000_000;
        sender.nonce = nonce;
        state.atoms.insert(sender_pk.to_vec(), sender.clone());

        let rx = SignedReaction {
            sender,
            receiver: Atom::new_materialized(receiver_pk.to_vec(), Element::Hydrogen),
            payload: Payload::Transfer { amount },
            energy,
            signature: b"fuzz_sig".to_vec(),
            reaction_hash: [0; 32],
            timestamp: 0,
        };

        let ctx = ExecutionContext::<FuzzCrypto> {
            state: &state,
            architect_pk: &[0; 32],
            current_temp: current_temp,
            crystal_index: 1,
            wasm_runtime: None,
            _crypto: std::marker::PhantomData,
        };

        let mut changeset = Changeset::new();
        let mut heat = 0.0;
        let mut entropy = 0.0;
        let atoms_iter = BTreeMap::new();

        let res = PVM::execute_single::<FuzzCrypto>(
            &ctx, &rx, &mut changeset, &atoms_iter, &mut heat, &mut entropy, galactic_drift, &[]
        );

        if res.is_ok() {
            if let Some(s) = changeset.inner.get(&sender_pk.to_vec()) {
                assert!(s.mass <= 1_000_000_000);
            }
        } else {
            assert!(changeset.is_empty());
        }
    }
}
