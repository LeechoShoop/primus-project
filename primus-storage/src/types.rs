use primus_types::atom::Atom;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GlobalMetrics {
    pub temperature: f32,
    pub entropy:     f32,
}

impl GlobalMetrics {
    pub fn canonical(&self) -> (u64, u64) {
        use primus_types::physics::PhysicsCanon;
        (
            PhysicsCanon::encode(self.temperature),
            PhysicsCanon::encode(self.entropy),
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UndoLog {
    pub crystal_index:  u64,
    pub pre_state_root: [u8; 32],
    #[serde(alias = "atom_deltas")]
    pub pre_images: BTreeMap<Vec<u8>, Option<Atom>>,
}

impl UndoLog {
    pub fn new(crystal_index: u64, pre_state_root: [u8; 32]) -> Self {
        Self { crystal_index, pre_state_root, pre_images: BTreeMap::new() }
    }
    pub fn record(&mut self, pk: Vec<u8>, before: Option<Atom>) {
        self.pre_images.entry(pk).or_insert(before);
    }
}
