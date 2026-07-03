// =============================================================================
// chamber.rs — Reaction Chamber (Refactored to use primus-types)
// =============================================================================

use crate::atom::AtomCoreExt;
use crate::crystal::Crystal;
use crate::gravity::GravityEngine;
use crate::kinetic::SignedReaction;
use std::collections::HashMap;

pub struct ReactionChamber {
    pub volatile_reactions: Vec<SignedReaction>,
    pub temperature: f32,
    pub entropy: f32,
    pub expansion_factor: f32,
    pub total_binding_energy: f32,
    pub base_difficulty: f32,
}

impl ReactionChamber {
    pub fn new() -> Self {
        Self {
            volatile_reactions: Vec::new(),
            temperature: 0.0,
            entropy: 0.0,
            expansion_factor: 1.0,
            total_binding_energy: 0.0,
            base_difficulty: 1.0,
        }
    }

    pub fn inherit_history(&mut self, last_crystal: &Crystal) {
        self.temperature = last_crystal.final_temp * 0.7;
        self.entropy = last_crystal.final_entropy * 0.4;

        if last_crystal.cumulative_energy > 50.0 {
            let compression = (last_crystal.cumulative_energy * 0.0001).min(0.05);
            self.expansion_factor = (1.10 - compression).max(1.0);
        }

        self.base_difficulty = 1.0 + (self.temperature * 0.01);

        println!(
            "🔥 Chamber inherited history: Temp {:.2} K, Base Difficulty: {:.2}",
            self.temperature, self.base_difficulty
        );
    }

    pub fn inject_reaction(&mut self, reaction: SignedReaction) {
        self.volatile_reactions.push(reaction);
    }

    pub fn calculate_surface_tension(&self, cluster: &[SignedReaction]) -> f32 {
        if cluster.is_empty() {
            return 0.0;
        }
        let total_viscosity: f32 = cluster
            .iter()
            .map(|rx| rx.sender.get_viscosity_factor())
            .sum();
        let avg_viscosity = total_viscosity / cluster.len() as f32;
        (cluster.len() as f32).sqrt() * avg_viscosity / (self.temperature + 1.0)
    }

    pub fn calibrate_thermodynamics(&mut self) {
        let reaction_count = self.volatile_reactions.len() as f32;
        let incremental_temp = reaction_count * 0.15 * self.base_difficulty;
        self.temperature += incremental_temp;
        self.entropy = (self.temperature * 1.5 * self.expansion_factor).ln_1p();
    }

    pub fn synthesize_with_gravity(
        &mut self,
        gravity: &GravityEngine,
        prev_hash: &[u8; 32],
        crystal_index: u64,
    ) -> Vec<SignedReaction> {
        self.calibrate_thermodynamics();

        self.volatile_reactions
            .sort_by(|a, b| b.sender.mass.cmp(&a.sender.mass));

        let tension = self.calculate_surface_tension(&self.volatile_reactions);
        println!(
            "--- Primus Fluid Dynamics | Temp: {:.2} | Tension: {:.2} | Entropy: {:.2} ---",
            self.temperature, tension, self.entropy
        );

        let mut final_cluster = Vec::new();
        let mut state_locks: HashMap<Vec<u8>, bool> = HashMap::new();
        let mut current_binding_energy = 0.0;
        let total_count = self.volatile_reactions.len();

        for (index, rx) in self.volatile_reactions.drain(..).enumerate() {
            let sender_key = rx.sender.public_key.clone();

            if let std::collections::hash_map::Entry::Vacant(e) = state_locks.entry(sender_key) {
                let relative_radius = index as f32 / total_count as f32;

                let roll =
                    gravity.generate_roll(e.key(), rx.sender.nonce, prev_hash, crystal_index);
                let roll_factor = (roll % 1000) as f32 / 1000.0;

                let radius_penalty = (1.0 + relative_radius) * self.base_difficulty;
                let stability_threshold = (0.1 / (tension + 0.1)) * radius_penalty;

                if self.entropy > 1.2 && roll_factor < stability_threshold {
                    println!(
                        "Centrifugal Loss: Atom {} at radius {:.2} evaporated due to Heat/Entropy!",
                        rx.sender.short_address(),
                        relative_radius
                    );
                    continue;
                }

                current_binding_energy += rx.sender.get_binding_potential();
                e.insert(true);
                final_cluster.push(rx);
            }
        }

        self.total_binding_energy = current_binding_energy;

        let compression = (self.total_binding_energy * 0.001).min(0.08);
        self.expansion_factor = (self.expansion_factor - compression).max(1.0);
        self.temperature += self.total_binding_energy * 0.03;
        self.entropy += self.total_binding_energy * 0.01;

        final_cluster
    }

    pub fn update_expansion(&mut self, new_factor: f32) {
        self.expansion_factor = new_factor;
    }
}
