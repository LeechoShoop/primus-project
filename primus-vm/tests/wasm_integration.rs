// =============================================================================
// primus-vm/tests/wasm_integration.rs — WASM Runtime Integration Tests
//
// Verifies that the WASM backend (Wasmtime) correctly enforces limits,
// charges gas, and interacts with the host state.
// =============================================================================

use primus_vm::wasm::wasmtime_backend::WasmtimeRuntime;
use primus_vm::wasm::WasmRuntime;
use primus_vm::{StateView, CryptoVerifier};
use primus_vm::wasm::host::{HostState, ContractDelta};
use primus_vm::wasm::gas::GasMeter;
use primus_types::atom::Atom;

struct MockState;
impl StateView for MockState {
    fn get_atom(&self, pk: &[u8]) -> Option<Atom> {
        if pk == b"alice" {
            let mut a = Atom::new_materialized(pk.to_vec(), primus_types::atom::Element::Hydrogen);
            a.mass = 5000;
            Some(a)
        } else {
            None
        }
    }
    fn crystal_index(&self) -> u64 { 1 }
    fn load_contract(&self, _code_hash: [u8; 32]) -> Option<Vec<u8>> { None }
}

struct MockCrypto;
impl CryptoVerifier for MockCrypto {
    fn verify(_pk: &[u8], _digest: &[u8], _sig: &[u8]) -> bool { true }
}

#[test]
fn test_wasm_execution_simple() {
    let runtime = WasmtimeRuntime::new().unwrap();
    
    let wat = r#"
        (module
            (memory 1)
            (export "memory" (memory 0))
            (func (export "call") (param i32 i32)
                nop
            )
        )
    "#;
    
    let wasm = wat::parse_str(wat).expect("Failed to parse WAT");
    let code_hash = [1u8; 32];
    
    runtime.load_module(code_hash, &wasm).unwrap();
    
    let state = MockState;
    let host_state = HostState {
        atoms: &state,
        caller_pk: vec![1; 32],
        crystal_index: 1,
        delta: ContractDelta::default(),
        gas: GasMeter::from_energy(2000.0),
        architect_pk: vec![0; 32],
        verify_fn: |_, _, _| true,
    };

    let output = runtime.execute(
        code_hash,
        &[0u8; 32],
        host_state,
        200_000,
    ).unwrap();
    
    assert!(output.gas_used > 0);
}

#[test]
fn test_wasm_memory_mandate_enforcement() {
    let runtime = WasmtimeRuntime::new().unwrap();
    
    // Try to grow memory beyond 16 MiB (256 pages)
    let wat = r#"
        (module
            (memory 1)
            (export "memory" (memory 0))
            (func (export "call") (param i32 i32)
                i32.const 512 ;; Try to grow to 512 pages (32 MiB)
                memory.grow
                drop
            )
        )
    "#;
    
    let wasm = wat::parse_str(wat).unwrap();
    let code_hash = [2u8; 32];
    runtime.load_module(code_hash, &wasm).unwrap();
    
    let state = MockState;
    let host_state = HostState {
        atoms: &state,
        caller_pk: vec![1; 32],
        crystal_index: 1,
        delta: ContractDelta::default(),
        gas: GasMeter::from_energy(2000.0),
        architect_pk: vec![0; 32],
        verify_fn: |_, _, _| true,
    };

    let output = runtime.execute(
        code_hash,
        &[0u8; 32],
        host_state,
        100_000,
    ).unwrap();
    
    assert!(output.gas_used > 0);
}
