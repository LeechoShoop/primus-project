// =============================================================================
// primus-vm/src/lib.rs — Public API for the Primus Virtual Machine
//
// DEPENDENCY INVARIANT: primus-vm depends ONLY on primus-types and
// primus-storage. It must NEVER import from primus-core, primus-net-opt,
// primus-cli, or primus-sdk.
//
// The dependency graph flows strictly downward:
//   primus-types → primus-storage → primus-vm → primus-core
// =============================================================================

pub mod context;
pub mod dispatch;
pub mod error;
pub mod physics;
pub mod pvm;
pub mod wasm;

// ── Re-exports — the stable public surface ───────────────────────────────────

pub use context::{CryptoVerifier, ExecutionContext, StateView};
pub use dispatch::PayloadDispatcher;
pub use error::PvmError;
pub use physics::{
    GRAVITY_SHIELD_GATE, MACRO_SHIFT_CRITICAL, MAX_GRAVITY_PULL, THERMAL_CAPACITY,
    calculate_entropy_tax, calculate_gravity_assist_from_iter, calculate_macro_shift,
    calculate_orbital_resonance, get_galactic_drift, get_spacetime_curvature,
};
pub use pvm::PVM;
pub use wasm::gas::{BASE_CONTRACT_GAS, GAS_PER_ENERGY, GasMeter, MAX_GAS_PER_REACTION};
pub use wasm::limits::*;
pub use wasm::{ContractOutput, WasmRuntime};
