// =============================================================================
// primus-vm/src/wasm/limits.rs — WASM Execution Limits
//
// These constants enforce the 16 MiB Mandate and bound resource usage for
// all WASM contract executions. They MUST be applied in both the Wasmtime
// and Wasmer backend configurations.
// =============================================================================

/// Maximum linear memory pages a WASM module may request.
/// 256 pages × 64 KiB = 16 MiB — the 16 MiB Mandate.
pub const MAX_WASM_MEMORY_PAGES: u32 = 256;

/// Maximum call stack size for WASM execution.
pub const MAX_WASM_STACK_SIZE: usize = 512 * 1024; // 512 KiB

/// Maximum size of a WASM module binary before compilation.
pub const MAX_MODULE_SIZE_BYTES: usize = 4 * 1024 * 1024; // 4 MiB

/// LRU cache capacity for compiled WASM modules.
pub const MODULE_CACHE_SIZE: usize = 256;

/// Maximum number of events a single contract invocation may emit.
pub const MAX_CONTRACT_EVENTS: usize = 64;

/// Maximum size of a single contract event (topic + data combined).
pub const MAX_EVENT_SIZE_BYTES: usize = 1024;

/// Divisor for converting gas usage to crystal heat contribution.
/// `heat_contribution = gas_used as f32 / GAS_HEAT_DIVISOR`
pub const GAS_HEAT_DIVISOR: f32 = 1000.0;

/// Maximum complexity value safe for f64 entropy tax calculation.
/// Above 2^53 even f64 loses integer precision.
pub const MAX_SAFE_COMPLEXITY: u64 = 1u64 << 53;
