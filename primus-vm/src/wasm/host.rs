// =============================================================================
// primus-vm/src/wasm/host.rs — WASM Host Functions
//
// Host functions allow WASM contracts to interact with the Primus state machine.
// All state access is metered (gas charged) and safe (no raw pointers).
// =============================================================================

use crate::context::StateView;
use crate::error::PvmError;

/// The internal state of a WASM execution session.
/// This is stored in the Wasmtime Store and passed to every host function.
pub struct HostState<'a> {
    /// Read-only view of on-chain atoms (from StateView).
    pub atoms: &'a dyn StateView,
    /// The caller's public key (sender of the SignedReaction).
    pub caller_pk: Vec<u8>,
    /// Current crystal index.
    pub crystal_index: u64,
    /// Accumulated state changes written by this contract invocation.
    /// This is the ONLY write path — never write to atoms directly.
    pub delta: ContractDelta,
    /// Gas meter for this invocation.
    pub gas: crate::wasm::gas::GasMeter,
    /// The architect's public key (for authorization checks).
    pub architect_pk: Vec<u8>,
    /// Signature verification function pointer.
    pub verify_fn: fn(pk: &[u8], msg: &[u8], sig: &[u8]) -> bool,
}

#[derive(Debug, Default, Clone)]
pub struct ContractDelta {
    /// Mass transfers requested by the contract.
    /// Each entry: (from_pk, to_pk, amount).
    /// Applied in order by PayloadDispatcher after WASM returns.
    pub transfers: Vec<(Vec<u8>, Vec<u8>, u64)>,
    /// Events emitted by the contract.
    pub events: Vec<ContractEvent>,
}

#[derive(Debug, Clone)]
pub struct ContractEvent {
    pub topic: Vec<u8>,
    pub data: Vec<u8>,
}

// ── Host Function Implementations ─────────────────────────────────────────────
// ── Resource Limiter ──────────────────────────────────────────────────────────

impl<'a> wasmtime::ResourceLimiter for HostState<'a> {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, anyhow::Error> {
        let max_bytes = (crate::wasm::limits::MAX_WASM_MEMORY_PAGES as usize) * 65536;
        if desired > max_bytes {
            return Ok(false);
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: u32,
        desired: u32,
        _maximum: Option<u32>,
    ) -> Result<bool, anyhow::Error> {
        if desired > 1024 {
            return Ok(false);
        }
        Ok(true)
    }
}


/// Helper to charge gas and return a Result.
pub fn charge_gas(state: &mut HostState, amount: u64) -> Result<(), PvmError> {
    state.gas.charge(amount)
}
