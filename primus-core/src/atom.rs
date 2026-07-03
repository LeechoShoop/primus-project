// use serde::{Serialize, Deserialize};
use crate::crypto::Crypto;
pub use primus_types::atom::{Atom, Element};

/// Extension methods for Atom in the core context.
#[allow(dead_code)]
pub trait AtomCoreExt {
    fn short_address(&self) -> String;
    fn absorb_decay(&mut self, scattered_mass: u64);
    fn evolve(&mut self);
    fn get_binding_potential(&self) -> f32;
    fn apply_decay(&mut self, current_index: u64) -> u64;
    fn get_viscosity_factor(&self) -> f32;
}

impl AtomCoreExt for Atom {
    fn short_address(&self) -> String {
        let hash = Crypto::sha3_256(&self.public_key);
        hex::encode(&hash[0..4])
    }

    fn absorb_decay(&mut self, scattered_mass: u64) {
        if scattered_mass == 0 {
            return;
        }
        let efficiency = match self.element {
            Element::Hydrogen => 0.95,
            Element::Carbon => 0.80,
            Element::Oxygen => 0.70,
            Element::Gold => 0.40,
        };
        let gained_mass = (scattered_mass as f32 * efficiency) as u64;
        self.mass += gained_mass;
        println!(
            "♻️ Atom {} ({:?}) absorbed {} mass. New mass: {}",
            self.short_address(),
            self.element,
            gained_mass,
            self.mass
        );
        self.evolve();
    }

    fn evolve(&mut self) {
        let old_element = self.element;
        match self.element {
            Element::Hydrogen if self.mass >= 4000 => {
                self.element = Element::Oxygen;
                self.charge = 3.44;
            }
            Element::Oxygen if self.mass >= 6000 => {
                self.element = Element::Carbon;
                self.charge = 2.55;
            }
            Element::Carbon if self.mass >= 50000 => {
                self.element = Element::Gold;
                self.charge = 2.54;
            }
            _ => return,
        }
        if old_element != self.element {
            println!(
                "🧬 EVOLUTION: Atom {} evolved from {:?} to {:?}",
                self.short_address(),
                old_element,
                self.element
            );
        }
    }


    fn get_binding_potential(&self) -> f32 {
        let base = match self.element {
            Element::Hydrogen => 1.0,
            Element::Carbon => 4.5,
            Element::Oxygen => 6.0,
            Element::Gold => 25.0,
        };
        base + (self.neutron_count as f32 * 1.8)
    }

    fn apply_decay(&mut self, current_index: u64) -> u64 {
        let age = current_index.saturating_sub(self.last_active_index);
        let stability_threshold = 100_u64.saturating_sub(self.neutron_count as u64 * 5);
        if age > stability_threshold {
            let decay_rate = 0.001 + (self.neutron_count as f32 * 0.0005);
            let decay_amount = (self.mass as f32 * decay_rate) as u64;
            if decay_amount > 0 {
                self.mass = self.mass.saturating_sub(decay_amount);
                println!(
                    "☢️ Isotope {} (n={}) is decaying! Mass lost: {}",
                    self.short_address(),
                    self.neutron_count,
                    decay_amount
                );
                return decay_amount;
            }
        }
        0
    }

    fn get_viscosity_factor(&self) -> f32 {
        match self.element {
            Element::Hydrogen => 1.0,
            Element::Carbon => 2.5,
            Element::Oxygen => 1.8,
            Element::Gold => 10.0 + (self.neutron_count as f32 * 0.5),
        }
    }
}
