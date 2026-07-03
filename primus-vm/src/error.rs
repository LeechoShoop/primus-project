// =============================================================================
// primus-vm/src/error.rs — Unified PVM Error Type
//
// All error variants for both the native PVM execution path and WASM
// contract execution. Designed to replace ad-hoc anyhow::anyhow!() errors
// from the original primus-core/src/pvm.rs with typed, matchable errors.
// =============================================================================

/// Unified error type for the Primus Virtual Machine.
///
/// Native PVM errors map 1:1 to the error strings from the original
/// `primus-core::pvm` module to preserve backward-compatible error messages.
/// WASM errors are new additions for Phase 2+ contract execution.
#[derive(Debug, thiserror::Error)]
pub enum PvmError {
    // ── Native PVM errors ────────────────────────────────────────────────────
    #[error("Signature REJECTED: {reason}")]
    SignatureRejected { reason: String },

    #[error("Nonce Mismatch: on-chain={on_chain}, tx={tx_nonce}")]
    NonceMismatch { on_chain: u64, tx_nonce: u64 },

    #[error("Insufficient mass: has={has}, needs={needs}")]
    InsufficientMass { has: u64, needs: u64 },

    #[error("Thermal Limit Exceeded: crystal meltdown")]
    ThermalLimitExceeded,

    #[error("Quantum Collapse: missing entangled partner")]
    QuantumCollapse,

    #[error("MiningReward recipient is not the Architect")]
    InvalidRewardRecipient,

    #[error("Unknown payload variant — upgrade required")]
    UnknownPayload,

    #[error("Conservation of Energy violation — negative energy")]
    NegativeEnergy,

    #[error("Source atom not found in state")]
    AtomNotFound,

    #[error("Arithmetic overflow in payload")]
    ArithmeticOverflow,

    // ── WASM errors ──────────────────────────────────────────────────────────
    #[error("WASM Out of Gas: limit={limit}, consumed={consumed}")]
    OutOfGas { limit: u64, consumed: u64 },

    #[error("WASM execution trap: {0}")]
    WasmTrap(String),

    #[error("Contract not found: {code_hash}")]
    ContractNotFound { code_hash: String },

    #[error("Module compilation failed: {0}")]
    CompilationFailed(String),

    #[error("Invalid WASM module: {0}")]
    InvalidModule(String),

    #[error("Host API violation: {0}")]
    HostViolation(String),

    #[error("Gas overflow")]
    GasOverflow,

    #[error("WASM backend error: {0}")]
    WasmBackendError(String),

    // ── General ──────────────────────────────────────────────────────────────
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
