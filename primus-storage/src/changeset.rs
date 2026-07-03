use primus_types::atom::Atom;
use std::collections::BTreeMap;

/// Write-set for a single Crystal.
/// BTreeMap enforces deterministic iteration order — consensus invariant.
/// NEVER change to HashMap.
#[derive(Clone, Default, Debug)]
pub struct Changeset {
    pub inner: BTreeMap<Vec<u8>, Atom>,
    pub contracts: BTreeMap<[u8; 32], Vec<u8>>,
}

impl Changeset {
    pub fn new() -> Self { Self { inner: BTreeMap::new(), contracts: BTreeMap::new() } }
    pub fn insert(&mut self, pk: Vec<u8>, atom: Atom) { self.inner.insert(pk, atom); }
    pub fn insert_contract(&mut self, hash: [u8; 32], code: Vec<u8>) { self.contracts.insert(hash, code); }
    pub fn get(&self, pk: &[u8]) -> Option<&Atom> { self.inner.get(pk) }
    pub fn sorted_keys(&self) -> impl Iterator<Item = &Vec<u8>> { self.inner.keys() }
    pub fn is_empty(&self) -> bool { self.inner.is_empty() && self.contracts.is_empty() }
    pub fn len(&self) -> usize { self.inner.len() }
}
