// =============================================================================
// primus-vm/src/context.rs — Execution Context & Trait Abstractions
//
// Defines the CryptoVerifier and StateView traits that decouple primus-vm
// from primus-core's concrete Crypto and StateTree types. primus-core
// provides the real ML-DSA-87 implementation; tests use mocks.
// =============================================================================

use primus_types::atom::Atom;

/// Injected cryptography provider. primus-core implements this with ML-DSA-87.
/// Tests use a mock implementation that always returns true or false.
///
/// The methods are intentionally associated functions (not &self) to match
/// the static `Crypto::verify()` call pattern from the original pvm.rs.
pub trait CryptoVerifier: Send + Sync {
    /// Returns true if the ML-DSA-87 signature over `digest` is valid for `pk`.
    fn verify(pk: &[u8], digest: &[u8], sig: &[u8]) -> bool;
}

/// Read-only view of chain state. Allows testing PVM without a live Sled
/// instance or the full StateTree from primus-core.
pub trait StateView: Send + Sync {
    /// Look up an atom by its public key. Returns None if the atom has
    /// never appeared on-chain.
    fn get_atom(&self, pk: &[u8]) -> Option<Atom>;

    /// The current crystal (block) index.
    fn crystal_index(&self) -> u64;

    /// Load WASM bytecode by code_hash. Returns None if not deployed.
    fn load_contract(&self, code_hash: [u8; 32]) -> Option<Vec<u8>>;
}

/// Everything the PVM and WasmRuntime need to execute a batch of reactions.
///
/// The lifetime `'a` borrows the state view and architect key from the caller
/// (typically the PrimusEngine in primus-core). The generic `C` selects the
/// cryptographic verifier implementation.
pub struct ExecutionContext<'a, C: CryptoVerifier> {
    /// Read-only state snapshot for the current crystal.
    pub state: &'a dyn StateView,

    /// The Architect's ML-DSA-87 public key. MiningReward transactions are
    /// validated against this key.
    pub architect_pk: &'a [u8],

    /// Current chamber temperature — used for spacetime curvature and
    /// thermal capacity checks.
    pub current_temp: f32,

    /// The crystal index for the block being executed.
    pub crystal_index: u64,

    /// Optional WASM runtime for executing Payload::Contract and Payload::ContractCall.
    pub wasm_runtime: Option<&'a dyn crate::wasm::WasmRuntime>,

    /// Phantom data to carry the CryptoVerifier type without storing an instance.
    pub _crypto: std::marker::PhantomData<C>,
}
