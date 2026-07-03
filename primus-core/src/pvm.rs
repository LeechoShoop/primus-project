// =============================================================================
// pvm.rs — Bridge Module (Delegates to primus-vm)
//
// This module provides the CryptoVerifier implementation for ML-DSA-87 and
// the StateView implementation for StateTree, bridging primus-core's concrete
// types to primus-vm's trait abstractions.
//
// All PVM logic has been migrated to primus-vm. This module provides:
//   1. CoreCrypto — CryptoVerifier implementation using ML-DSA-87
//   2. StateView implementation for StateTree
//   3. PVM — re-exported from primus-vm with a convenience wrapper
// =============================================================================


use crate::state::StateTree;
use primus_types::atom::Atom;

// ── Re-export PVM and physics from primus-vm ─────────────────────────────────

pub use primus_vm::PVM;
pub use primus_vm::physics::get_galactic_drift;

// ── CryptoVerifier implementation ────────────────────────────────────────────

pub use crate::crypto_shim::CoreCryptoVerifier;

use crate::storage::PrimusStorage;

// ── StateView implementation ──────────────────────────────────────────────────

pub struct CoreStateView<'a> {
    pub tree: &'a StateTree,
    pub storage: &'a PrimusStorage,
}

impl<'a> primus_vm::StateView for CoreStateView<'a> {
    fn get_atom(&self, pk: &[u8]) -> Option<Atom> {
        self.tree.atoms.get(pk).cloned()
    }

    fn crystal_index(&self) -> u64 {
        self.tree.current_crystal_index
    }

    fn load_contract(&self, code_hash: [u8; 32]) -> Option<Vec<u8>> {
        self.storage.load_contract(code_hash).ok().flatten()
    }
}

// ── Convenience functions ────────────────────────────────────────────────────

/// Build an ExecutionContext from primus-core's StateTree and architect key.
pub fn make_execution_context<'a>(
    state_view: &'a CoreStateView<'a>,
    current_temp: f32,
    architect_pk: &'a [u8],
    wasm_runtime: Option<&'a dyn primus_vm::WasmRuntime>,
) -> primus_vm::ExecutionContext<'a, CoreCryptoVerifier> {
    primus_vm::ExecutionContext {
        state: state_view,
        architect_pk,
        current_temp: crate::physics_shim::to_vm_thermal(current_temp as f64) as f32,
        crystal_index: state_view.tree.current_crystal_index,
        wasm_runtime,
        _crypto: std::marker::PhantomData,
    }
}

/// Execute a batch of reactions using the CoreCrypto verifier.
pub fn execute_payload(
    state: &StateTree,
    storage: &PrimusStorage,
    payload: &[primus_types::reaction::SignedReaction],
    current_temp: f32,
    architect_pk: &[u8],
    wasm_runtime: Option<&dyn primus_vm::WasmRuntime>,
) -> Result<(primus_storage::Changeset, f32), primus_vm::PvmError> {
    let state_view = CoreStateView { tree: state, storage };
    let ctx = make_execution_context(&state_view, current_temp, architect_pk, wasm_runtime);
    PVM::execute_payload::<CoreCryptoVerifier>(&ctx, payload, &state.atoms)
}
