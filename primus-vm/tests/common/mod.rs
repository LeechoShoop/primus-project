use primus_vm::context::{CryptoVerifier, StateView};
use primus_types::atom::Atom;
use std::collections::HashMap;

pub struct MockCryptoVerifier;

impl CryptoVerifier for MockCryptoVerifier {
    fn verify(pk: &[u8], _digest: &[u8], sig: &[u8]) -> bool {
        // Simple mock: if sig is [1], it's valid. If [0], invalid.
        // Or if sig matches pk (for simplicity in some tests).
        if sig == b"valid_sig" {
            return true;
        }
        if sig.len() == pk.len() && sig == pk {
            return true;
        }
        false
    }
}

pub struct MockStateView {
    pub atoms: HashMap<Vec<u8>, Atom>,
    pub index: u64,
    pub contracts: HashMap<[u8; 32], Vec<u8>>,
}

impl MockStateView {
    pub fn new() -> Self {
        Self {
            atoms: HashMap::new(),
            index: 1,
            contracts: HashMap::new(),
        }
    }
}

impl StateView for MockStateView {
    fn get_atom(&self, pk: &[u8]) -> Option<Atom> {
        self.atoms.get(pk).cloned()
    }

    fn crystal_index(&self) -> u64 {
        self.index
    }

    fn load_contract(&self, code_hash: [u8; 32]) -> Option<Vec<u8>> {
        self.contracts.get(&code_hash).cloned()
    }
}
