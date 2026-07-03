use wasmtime::*;
use crate::wasm::{WasmRuntime, ContractOutput, host::HostState, host::ContractEvent};
use crate::wasm::gas::costs;
use crate::error::PvmError;
use parking_lot::RwLock;
use lru::LruCache;
use std::sync::Arc;
use std::num::NonZeroUsize;

pub struct WasmtimeRuntime {
    engine: Engine,
    module_cache: Arc<RwLock<LruCache<[u8; 32], Module>>>,
}

impl WasmtimeRuntime {
    pub fn new() -> Result<Self, PvmError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.cranelift_opt_level(OptLevel::Speed);
        config.parallel_compilation(true);

        let engine = Engine::new(&config).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;
        
        Ok(Self {
            engine,
            module_cache: Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(256).unwrap()))),
        })
    }

    fn create_linker(&self) -> Result<Linker<HostState<'_>>, PvmError> {
        let mut linker = Linker::new(&self.engine);

        // --- primus_v1 Namespace ---

        // get_atom_mass(pk_ptr: i32, pk_len: i32) -> i64
        linker.func_wrap("primus_v1", "get_atom_mass", |mut caller: Caller<'_, HostState<'_>>, pk_ptr: i32, pk_len: i32| -> i64 {
            if caller.data_mut().gas.charge(costs::GET_ATOM_MASS).is_err() { return -1; }
            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m, None => return -1,
            };
            let pk = match read_wasm_bytes(&mem, &caller, pk_ptr, pk_len) {
                Some(b) => b, None => return -1,
            };
            caller.data().atoms.get_atom(&pk).map(|a| a.mass as i64).unwrap_or(-1)
        }).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;

        // get_atom_nonce(pk_ptr: i32, pk_len: i32) -> i64
        linker.func_wrap("primus_v1", "get_atom_nonce", |mut caller: Caller<'_, HostState<'_>>, pk_ptr: i32, pk_len: i32| -> i64 {
            if caller.data_mut().gas.charge(costs::GET_ATOM_NONCE).is_err() { return -1; }
            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m, None => return -1,
            };
            let pk = match read_wasm_bytes(&mem, &caller, pk_ptr, pk_len) {
                Some(b) => b, None => return -1,
            };
            caller.data().atoms.get_atom(&pk).map(|a| a.nonce as i64).unwrap_or(-1)
        }).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;

        // get_crystal_index() -> i64
        linker.func_wrap("primus_v1", "get_crystal_index", |mut caller: Caller<'_, HostState<'_>>| -> i64 {
            if caller.data_mut().gas.charge(costs::GET_CRYSTAL_INDEX).is_err() { return -1; }
            caller.data().crystal_index as i64
        }).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;

        // get_caller_pk(out_ptr: i32) -> i32
        linker.func_wrap("primus_v1", "get_caller_pk", |mut caller: Caller<'_, HostState<'_>>, out_ptr: i32| -> i32 {
            if caller.data_mut().gas.charge(costs::GET_CALLER_PK).is_err() { return -1; }
            let caller_pk = caller.data().caller_pk.clone();
            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m, None => return -1,
            };
            if write_wasm_bytes(&mem, &mut caller, out_ptr, &caller_pk) {
                caller_pk.len() as i32
            } else {
                -1
            }
        }).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;

        // transfer_mass(from_ptr: i32, from_len: i32, to_ptr: i32, to_len: i32, amount: i64) -> i32
        linker.func_wrap("primus_v1", "transfer_mass", |mut caller: Caller<'_, HostState<'_>>, 
            from_ptr: i32, from_len: i32, 
            to_ptr: i32, to_len: i32, 
            amount: i64| -> i32 
        {
            if caller.data_mut().gas.charge(costs::TRANSFER_MASS).is_err() { return 4; }
            if amount <= 0 { return 3; }
            let amount = amount as u64;

            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m, None => return 3,
            };
            let from_pk = match read_wasm_bytes(&mem, &caller, from_ptr, from_len) {
                Some(b) => b, None => return 3,
            };
            let to_pk = match read_wasm_bytes(&mem, &caller, to_ptr, to_len) {
                Some(b) => b, None => return 3,
            };

            let caller_pk = caller.data().caller_pk.clone();
            if from_pk != caller_pk { return 2; }

            let current_mass = {
                let state = caller.data();
                let base = state.atoms.get_atom(&from_pk).map(|a| a.mass).unwrap_or(0);
                let pending_out: u64 = state.delta.transfers.iter()
                    .filter(|(f, _, _)| f == &from_pk)
                    .map(|(_, _, amt)| *amt)
                    .fold(0u64, |acc, x| acc.saturating_add(x));
                base.saturating_sub(pending_out)
            };

            if current_mass < amount { return 1; }

            caller.data_mut().delta.transfers.push((from_pk, to_pk, amount));
            0
        }).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;

        // emit_event(topic_ptr: i32, topic_len: i32, data_ptr: i32, data_len: i32) -> i32
        linker.func_wrap("primus_v1", "emit_event", |mut caller: Caller<'_, HostState<'_>>, 
            topic_ptr: i32, topic_len: i32, 
            data_ptr: i32, data_len: i32| -> i32 
        {
            if caller.data_mut().gas.charge(costs::EMIT_EVENT).is_err() { return 1; }
            
            const MAX_CONTRACT_EVENTS: usize = 64;
            const MAX_EVENT_SIZE_BYTES: usize = 1024;

            if caller.data().delta.events.len() >= MAX_CONTRACT_EVENTS { return 2; }

            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m, None => return 3,
            };
            let topic = match read_wasm_bytes(&mem, &caller, topic_ptr, topic_len) {
                Some(b) => b, None => return 3,
            };
            let data = match read_wasm_bytes(&mem, &caller, data_ptr, data_len) {
                Some(b) => b, None => return 3,
            };

            if topic.len() + data.len() > MAX_EVENT_SIZE_BYTES { return 2; }

            caller.data_mut().delta.events.push(ContractEvent { topic, data });
            0
        }).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;

        // verify_signature(pk_ptr: i32, pk_len: i32, msg_ptr: i32, msg_len: i32, sig_ptr: i32, sig_len: i32) -> i32
        linker.func_wrap("primus_v1", "verify_signature", |mut caller: Caller<'_, HostState<'_>>, 
            pk_ptr: i32, pk_len: i32, 
            msg_ptr: i32, msg_len: i32, 
            sig_ptr: i32, sig_len: i32| -> i32 
        {
            if caller.data_mut().gas.charge(costs::VERIFY_SIGNATURE).is_err() { return -1; }

            let mem = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m, None => return -1,
            };
            let pk = match read_wasm_bytes(&mem, &caller, pk_ptr, pk_len) {
                Some(b) => b, None => return -1,
            };
            let msg = match read_wasm_bytes(&mem, &caller, msg_ptr, msg_len) {
                Some(b) => b, None => return -1,
            };
            let sig = match read_wasm_bytes(&mem, &caller, sig_ptr, sig_len) {
                Some(b) => b, None => return -1,
            };

            let result = (caller.data().verify_fn)(&pk, &msg, &sig);
            if result { 1 } else { 0 }
        }).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;

        // remaining_gas() -> i64
        linker.func_wrap("primus_v1", "remaining_gas", |caller: Caller<'_, HostState<'_>>| -> i64 {
            caller.data().gas.remaining() as i64
        }).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;

        Ok(linker)
    }
}

impl WasmRuntime for WasmtimeRuntime {
    fn engine_name(&self) -> &'static str {
        "wasmtime"
    }

    fn load_module(&self, code_hash: [u8; 32], wasm_bytes: &[u8]) -> Result<(), PvmError> {
        if self.module_cache.read().contains(&code_hash) {
            return Ok(());
        }

        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| PvmError::WasmBackendError(format!("Module compilation failed: {}", e)))?;

        self.module_cache.write().put(code_hash, module);
        Ok(())
    }

    fn execute(
        &self,
        code_hash: [u8; 32],
        calldata: &[u8],
        host_state: HostState,
        gas_limit: u64,
    ) -> Result<ContractOutput, PvmError> {
        let module = self.module_cache.write().get(&code_hash).cloned()
            .ok_or_else(|| PvmError::WasmBackendError("Module not found in cache".into()))?;

        let mut store = Store::new(&self.engine, host_state);
        store.set_fuel(gas_limit).map_err(|e| PvmError::WasmBackendError(e.to_string()))?;
        
        store.limiter(|s| s);

        let linker = self.create_linker()?;
        let instance = linker.instantiate(&mut store, &module)
            .map_err(|e| PvmError::WasmBackendError(format!("Instantiation failed: {}", e)))?;

        let call_func = instance.get_typed_func::<(i32, i32), ()>(&mut store, "call")
            .map_err(|e| PvmError::WasmBackendError(format!("Export 'call' not found: {}", e)))?;

        // Passing calldata if needed (though current spec doesn't say how)
        let _ = calldata; 

        call_func.call(&mut store, (0, 0))
            .map_err(|e| PvmError::WasmBackendError(format!("Execution failed: {}", e)))?;

        let gas_used = gas_limit.saturating_sub(store.get_fuel().unwrap_or(0));
        let final_host_state = store.into_data();

        Ok(ContractOutput {
            return_data: Vec::new(), // Phase 2 return data is empty for now or handled via events/delta
            gas_used,
            state_delta: final_host_state.delta,
        })
    }
}

/// Read `len` bytes from WASM linear memory at `ptr`.
fn read_wasm_bytes(
    mem: &Memory,
    caller: &Caller<'_, HostState<'_>>,
    ptr: i32,
    len: i32,
) -> Option<Vec<u8>> {
    if ptr < 0 || len < 0 { return None; }
    let start = ptr as usize;
    let end = start.checked_add(len as usize)?;
    mem.data(caller).get(start..end).map(|s| s.to_vec())
}

/// Write `bytes` into WASM linear memory at `ptr`.
fn write_wasm_bytes(
    mem: &Memory,
    caller: &mut Caller<'_, HostState<'_>>,
    ptr: i32,
    bytes: &[u8],
) -> bool {
    if ptr < 0 { return false; }
    let start = ptr as usize;
    let end = match start.checked_add(bytes.len()) {
        Some(e) => e,
        None => return false,
    };
    match mem.data_mut(caller).get_mut(start..end) {
        Some(slot) => { slot.copy_from_slice(bytes); true }
        None => false,
    }
}
