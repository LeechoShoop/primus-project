pub use primus_types::Crystal;
use primus_types::physics::PhysicsCanon;

pub trait CrystalExt {
    fn calculate_density(&self) -> [u8; 32];
    fn check_proof_of_work(&self, chamber_temp: f32, difficulty: f32) -> bool;
    fn calculate_target(&self, temp: f32, diff: f32) -> [u8; 32];
    fn optimize_lattice(&mut self);
}

impl CrystalExt for Crystal {
    fn calculate_density(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.index.to_le_bytes());
        hasher.update(&self.prev_hash);
        hasher.update(&self.timestamp.to_le_bytes());
        hasher.update(&self.nonce.to_le_bytes());
        hasher.update(&self.state_root);

        for rx_hash in &self.reactions {
            hasher.update(rx_hash);
        }

        hasher.update(&PhysicsCanon::encode(self.cumulative_energy).to_le_bytes());
        hasher.update(&PhysicsCanon::encode(self.final_temp).to_le_bytes());
        hasher.update(&PhysicsCanon::encode(self.final_entropy).to_le_bytes());

        *hasher.finalize().as_bytes()
    }

    fn check_proof_of_work(&self, chamber_temp: f32, difficulty: f32) -> bool {
        let hash = self.calculate_density();
        let target = self.calculate_target(chamber_temp, difficulty);

        for i in 0..32 {
            if hash[i] < target[i] {
                return true;
            }
            if hash[i] > target[i] {
                return false;
            }
        }
        true
    }

    fn calculate_target(&self, temp: f32, diff: f32) -> [u8; 32] {
        let mut target = [0u8; 32];
        let base_difficulty = (diff * (1.0 + temp / 500.0)).max(1.0);
        let zeros = (base_difficulty as usize).min(4);

        for i in 0..zeros {
            target[i] = 0;
        }
        for i in zeros..32 {
            target[i] = 0xFF;
        }

        target[zeros] = (0xFF as f32 / (base_difficulty - zeros as f32 + 1.0)) as u8;
        target
    }

    fn optimize_lattice(&mut self) {
        // No-op placeholder
    }
}
