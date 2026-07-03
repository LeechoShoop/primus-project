// =============================================================================
// primus-vm/src/wasm/mod.rs — WASM Runtime Abstraction Layer
//
// Phase 1: Stub definitions only. The WasmRuntime trait and RuntimeKind enum
// will be fully implemented in Phase 2 (wasmtime) and Phase 3 (wasmer).
// =============================================================================

pub mod gas;
pub mod limits;
pub mod host;

#[cfg(feature = "wasmtime-backend")]
pub mod wasmtime_backend;

use crate::error::PvmError;

/// Trait for WASM execution backends.
///
/// Both WasmtimeRuntime (Phase 2) and WasmerRuntime (Phase 3) implement this
/// trait. The PayloadDispatcher holds a `dyn WasmRuntime` to execute
/// `Payload::Contract` reactions.
pub trait WasmRuntime: Send + Sync {
    /// Pre-compile and cache a WASM module.
    fn load_module(&self, code_hash: [u8; 32], wasm_bytes: &[u8]) -> Result<(), PvmError>;

    /// Execute a previously loaded WASM contract.
    fn execute(
        &self,
        code_hash: [u8; 32],
        calldata: &[u8],
        host_state: crate::wasm::host::HostState,
        gas_limit: u64,
    ) -> Result<ContractOutput, PvmError>;

    /// Human-readable name of the backend engine (e.g. "wasmtime", "wasmer").
    fn engine_name(&self) -> &'static str;
}

/// Output from a WASM contract execution.
pub struct ContractOutput {
    /// Data returned by the contract's `call` export.
    pub return_data: Vec<u8>,
    /// Total gas consumed during execution.
    pub gas_used: u64,
    /// Accumulated state changes written by this contract invocation.
    pub state_delta: crate::wasm::host::ContractDelta,
}
