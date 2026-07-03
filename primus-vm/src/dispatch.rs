// =============================================================================
// primus-vm/src/dispatch.rs — Payload Dispatcher
//
// Phase 1: Simple pass-through to PVM::execute_payload for all non-WASM
// payload types. WASM contract routing will be added in Phase 2.
// =============================================================================

use primus_types::atom::Atom;
use primus_types::reaction::SignedReaction;
use primus_storage::Changeset;

use crate::context::{CryptoVerifier, ExecutionContext};
use crate::error::PvmError;
use crate::pvm::PVM;
use primus_types::payload::Payload;
use crate::wasm::{WasmRuntime, host::HostState, host::ContractDelta};
use crate::wasm::gas::GasMeter;
use std::sync::Arc;

/// The PayloadDispatcher is the single entry-point for executing a batch
/// of reactions. In Phase 1 it delegates entirely to `PVM::execute_payload`.
/// In Phase 2 it will additionally route `Payload::Contract` reactions to
/// the WASM runtime.
pub struct PayloadDispatcher {
    pub wasm_runtime: Option<Arc<dyn WasmRuntime>>,
}

impl PayloadDispatcher {
    /// Create a new dispatcher.
    pub fn new(wasm_runtime: Option<Arc<dyn WasmRuntime>>) -> Self {
        Self { wasm_runtime }
    }

    fn dispatch_contract<C: CryptoVerifier>(
        &self,
        ctx: &ExecutionContext<C>,
        rx: &SignedReaction,
        code_hash: [u8; 32],
        calldata: &[u8],
        running_changeset: &mut Changeset,
    ) -> Result<f32, PvmError> {
        // 1. Load WASM bytecode from storage
        let wasm_bytes = ctx.state
            .load_contract(code_hash)
            .ok_or_else(|| PvmError::ContractNotFound {
                code_hash: hex::encode(code_hash),
            })?;

        // 2. Build HostState
        let gas_meter = GasMeter::from_energy(rx.energy);
        let gas_limit = gas_meter.limit;
        let host_state = HostState {
            atoms: ctx.state,
            caller_pk: rx.sender.public_key.clone(),
            crystal_index: ctx.crystal_index,
            delta: ContractDelta::default(),
            gas: gas_meter,
            architect_pk: ctx.architect_pk.to_vec(),
            verify_fn: C::verify,
        };

        // 3. Execute in WASM runtime
        let runtime = self.wasm_runtime.as_ref()
            .ok_or_else(|| PvmError::WasmTrap("No WASM runtime configured".into()))?;

        runtime.load_module(code_hash, &wasm_bytes)?;
        let output = runtime.execute(code_hash, calldata, host_state, gas_limit)?;

        // 4. Apply ContractDelta to running Changeset
        //    Each transfer: debit from_pk, credit to_pk.
        //    Read current mass from running_changeset first, then ctx.state.
        for (from_pk, to_pk, amount) in &output.state_delta.transfers {
            // Debit sender
            let mut sender = running_changeset
                .get(from_pk)
                .cloned()
                .or_else(|| ctx.state.get_atom(from_pk))
                .ok_or_else(|| PvmError::AtomNotFound)?;

            sender.mass = sender.mass.checked_sub(*amount)
                .ok_or(PvmError::InsufficientMass { has: sender.mass, needs: *amount })?;

            running_changeset.insert(from_pk.clone(), sender);

            // Credit receiver — auto-materialise at 0 mass if not found
            let mut receiver = running_changeset
                .get(to_pk)
                .cloned()
                .or_else(|| ctx.state.get_atom(to_pk))
                .unwrap_or_else(|| {
                    primus_types::atom::Atom::new_receiver(to_pk.clone())
                });

            receiver.mass = receiver.mass.saturating_add(*amount);
            running_changeset.insert(to_pk.clone(), receiver);
        }

        // 5. Increment sender nonce (Contract reactions advance nonce like Transfer)
        let mut sender = running_changeset
            .get(&rx.sender.public_key)
            .cloned()
            .or_else(|| ctx.state.get_atom(&rx.sender.public_key))
            .ok_or(PvmError::AtomNotFound)?;
        sender.nonce = sender.nonce.saturating_add(1);
        sender.last_reaction_hash = rx.reaction_hash;
        running_changeset.insert(rx.sender.public_key.clone(), sender);

        // 6. Calculate heat contribution from gas consumed
        let contract_heat = output.gas_used as f32 / crate::wasm::limits::GAS_HEAT_DIVISOR;

        Ok(contract_heat)
    }

    /// Execute a batch of reactions, returning the merged changeset and
    /// total consumed entropy.
    pub fn execute<C: CryptoVerifier>(
        &self,
        ctx: &ExecutionContext<C>,
        reactions: &[SignedReaction],
        atoms_iter: &std::collections::BTreeMap<Vec<u8>, Atom>,
    ) -> Result<(Changeset, f32), PvmError> {
        let mut total_crystal_heat = 0.0_f32;
        let mut total_consumed_entropy = 0.0_f32;
        let mut changeset = Changeset::new();
        let galactic_drift = crate::physics::get_galactic_drift(ctx.crystal_index);

        for rx in reactions {
            match &rx.payload {
                Payload::ContractCall { address, data } => {
                    let code_hash: [u8; 32] = address.as_slice().try_into()
                        .map_err(|_| PvmError::WasmBackendError("Invalid contract address".into()))?;

                    let contract_heat = self.dispatch_contract(
                        ctx,
                        rx,
                        code_hash,
                        data,
                        &mut changeset,
                    )?;

                    total_crystal_heat += contract_heat;
                    if total_crystal_heat > crate::physics::THERMAL_CAPACITY {
                        return Err(PvmError::ThermalLimitExceeded);
                    }
                }
                _ => {
                    // Delegate native payloads to PVM
                    PVM::execute_single::<C>(
                        ctx,
                        rx,
                        &mut changeset,
                        atoms_iter,
                        &mut total_crystal_heat,
                        &mut total_consumed_entropy,
                        galactic_drift,
                        reactions,
                    )?;
                }
            }
        }

        Ok((changeset, total_consumed_entropy))
    }
}
