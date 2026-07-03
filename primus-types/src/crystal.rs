use crate::reaction::SignedReaction;
use crate::Vec;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Crystal {
    pub index: u64,
    pub prev_hash: [u8; 32],
    pub timestamp: u64,
    pub nonce: u64,
    pub reactions: Vec<[u8; 32]>,
    pub reactions_data: Vec<SignedReaction>,
    pub cumulative_energy: f32,
    pub state_root: [u8; 32],
    pub final_temp: f32,
    pub final_entropy: f32,
}

impl Crystal {
    pub fn new(index: u64, prev_hash: [u8; 32]) -> Self {
        Self {
            index,
            prev_hash,
            timestamp: 0,
            nonce: 0,
            reactions: Vec::new(),
            reactions_data: Vec::new(),
            cumulative_energy: 0.0,
            state_root: [0u8; 32],
            final_temp: 0.0,
            final_entropy: 0.0,
        }
    }
}
