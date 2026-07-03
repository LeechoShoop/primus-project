// =============================================================================
// primus-vm/src/wasm/gas.rs — Gas Metering
//
// Gas is the deterministic resource-accounting mechanism for WASM contracts.
// Every host function call and WASM instruction consumes gas. When gas is
// exhausted, the contract traps with PvmError::OutOfGas and zero state
// changes are applied (atomic failure).
//
// CRITICAL: gas.charge() MUST be called BEFORE the operation it meters.
// =============================================================================

pub mod costs {
    pub const GET_ATOM_MASS:      u64 = 100;
    pub const GET_ATOM_NONCE:     u64 = 100;
    pub const GET_CRYSTAL_INDEX:  u64 = 10;
    pub const GET_CALLER_PK:      u64 = 50;
    pub const TRANSFER_MASS:      u64 = 500;
    pub const EMIT_EVENT:         u64 = 200;
    pub const VERIFY_SIGNATURE:   u64 = 5_000;
    pub const REMAINING_GAS:      u64 = 0;
}

use crate::error::PvmError;

/// Base gas allocated to every contract invocation, even if energy is very low.
pub const BASE_CONTRACT_GAS: u64 = 10_000;

/// Gas units allocated per unit of energy in the reaction.
pub const GAS_PER_ENERGY: u64 = 100;

/// Hard cap on gas per single contract invocation.
pub const MAX_GAS_PER_REACTION: u64 = 1_000_000;

/// Tracks gas consumption during a single WASM contract execution.
pub struct GasMeter {
    /// Maximum gas this invocation may consume.
    pub limit: u64,
    /// Gas consumed so far.
    pub consumed: u64,
}

impl GasMeter {
    /// Create a gas meter from the reaction's energy field.
    ///
    /// The gas limit is `energy * GAS_PER_ENERGY`, clamped to
    /// `[BASE_CONTRACT_GAS, MAX_GAS_PER_REACTION]`.
    pub fn from_energy(energy: f32) -> Self {
        let limit = ((energy as u64).saturating_mul(GAS_PER_ENERGY))
            .min(MAX_GAS_PER_REACTION)
            .max(BASE_CONTRACT_GAS);
        Self { limit, consumed: 0 }
    }

    /// Charge gas BEFORE performing the metered operation.
    ///
    /// Returns `Err(PvmError::GasOverflow)` if the addition overflows u64.
    /// Returns `Err(PvmError::OutOfGas)` if consumed exceeds limit.
    pub fn charge(&mut self, amount: u64) -> Result<(), PvmError> {
        self.consumed = self
            .consumed
            .checked_add(amount)
            .ok_or(PvmError::GasOverflow)?;
        if self.consumed > self.limit {
            Err(PvmError::OutOfGas {
                limit: self.limit,
                consumed: self.consumed,
            })
        } else {
            Ok(())
        }
    }

    /// Gas remaining before the limit is hit.
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gas_meter_from_energy_clamps_to_base() {
        let meter = GasMeter::from_energy(0.0);
        assert_eq!(meter.limit, BASE_CONTRACT_GAS);
    }

    #[test]
    fn gas_meter_from_energy_clamps_to_max() {
        let meter = GasMeter::from_energy(100_000.0);
        assert_eq!(meter.limit, MAX_GAS_PER_REACTION);
    }

    #[test]
    fn gas_meter_charge_succeeds() {
        let mut meter = GasMeter::from_energy(100.0);
        assert!(meter.charge(5_000).is_ok());
        assert_eq!(meter.consumed, 5_000);
        assert_eq!(meter.remaining(), meter.limit - 5_000);
    }

    #[test]
    fn gas_meter_charge_exceeds_limit() {
        let mut meter = GasMeter::from_energy(100.0);
        let result = meter.charge(meter.limit + 1);
        assert!(result.is_err());
        match result.unwrap_err() {
            PvmError::OutOfGas { limit, consumed } => {
                assert_eq!(limit, meter.limit);
                assert_eq!(consumed, meter.limit + 1);
            }
            other => panic!("Expected OutOfGas, got: {:?}", other),
        }
    }
}
