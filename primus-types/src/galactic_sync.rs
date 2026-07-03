// =============================================================================
// primus-types/src/galactic_sync.rs
//
// GalacticStatus and SyncMessage are wire-protocol types shared between:
//   - primus-net-opt  (produces/consumes them in network.rs)
//   - primus-core     (galactic_sync.rs re-exports from here)
//
// Moving them here breaks the circular import that arose when primus-net-opt
// tried to import `primus_types::galactic_sync` which did not exist.
// =============================================================================

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GalacticStatus {
    pub crystal_index: u64,
    pub current_entropy: f32,
    pub sector_drift: u8,
    /// Cumulative binding energy carried by the heaviest chain known to this node.
    /// Used by `is_more_dominant_than` as a tiebreaker when `crystal_index` is equal.
    #[serde(default)]
    pub cumulative_energy: f32,
}

impl GalacticStatus {
    /// Construct from raw state scalars (legacy / internal use).
    pub fn from_state(index: u64, entropy: f32) -> Self {
        Self {
            crystal_index: index,
            current_entropy: entropy,
            sector_drift: (index % 256) as u8,
            cumulative_energy: 0.0,
        }
    }

    /// Construct from the full engine state including thermodynamic weight.
    /// `cum_energy` is the `cumulative_energy` field of the latest Crystal.
    pub fn from_engine(index: u64, entropy: f32, cum_energy: f32) -> Self {
        Self {
            crystal_index: index,
            current_entropy: entropy,
            sector_drift: (index % 256) as u8,
            cumulative_energy: cum_energy,
        }
    }

    /// Rule: Higher crystal_index wins.
    /// If equal, higher cumulative_energy (thermodynamic weight) wins.
    /// If still equal, higher current_entropy (Binding Energy) wins.
    pub fn is_more_dominant_than(&self, other: &Self) -> bool {
        if self.crystal_index > other.crystal_index {
            return true;
        }
        if self.crystal_index == other.crystal_index {
            if self.cumulative_energy > other.cumulative_energy {
                return true;
            }
            if (self.cumulative_energy - other.cumulative_energy).abs() < f32::EPSILON
                && self.current_entropy > other.current_entropy
            {
                return true;
            }
        }
        false
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SyncMessage {
    Handshake(GalacticStatus),
    RequestCrystals { start: u64, end: u64 },
    InventoryResponse(Vec<Vec<u8>>), // Serialized crystals
}
