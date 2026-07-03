## Overview
`primus-vm` is the deterministic execution engine for the Primus blockchain, responsible for processing reactions and maintaining state consistency. It serves as the bridge between storage and consensus, enforcing the physical and cryptographic rules of the network. The crate maintains a strict dependency invariant where the dependency graph flows strictly downward: `primus-types` → `primus-storage` → `primus-vm` → `primus-core`.

## Architecture
The `primus-vm` crate is structured into four main layers:
1. **Execution context & traits (context.rs)**: Defines the `ExecutionContext` and abstraction traits like `StateView` and `CryptoVerifier`, allowing the VM to interact with storage and verify signatures without direct dependencies on external crates.
2. **Native PVM (pvm.rs)**: Implements the execution logic for native payloads such as `Transfer`, `Generic`, and `MiningReward`. It handles nonce-based sequence checks and applies the physics simulation to every reaction.
3. **WASM subsystem (wasm/)**: Manages the execution of smart contracts under the 16 MiB Mandate. This layer includes gas metering logic, resource limits, host function bindings, and the Wasmtime backend.
4. **Payload dispatcher (dispatch.rs)**: The central entry point that receives a batch of reactions and routes them to either the native PVM or the WASM runtime based on the payload type.

```text
PayloadDispatcher → PVM::execute_single (native payloads)
                  → WasmtimeRuntime::execute (Payload::ContractCall)
```

## Crate Features
| Feature | Default | Description |
| :--- | :--- | :--- |
| `wasmtime-backend` | Yes | Enables the Wasmtime-based execution engine for smart contracts. |
| `wasmer-backend` | No | Enables the Wasmer-based execution engine for smart contracts. |

## Gas Metering
Gas is the deterministic resource-accounting mechanism for WASM contracts. Every host function call and WASM instruction consumes gas. The `GasMeter` tracks consumption, with the limit derived from the reaction's energy field: `energy * GAS_PER_ENERGY`, clamped between `BASE_CONTRACT_GAS` (10,000) and `MAX_GAS_PER_REACTION` (1,000,000). A **CRITICAL** rule is that `gas.charge()` MUST be called **BEFORE** the operation it meters to prevent resource exhaustion attacks.

| Constant | Value (u64) |
| :--- | :--- |
| `GET_ATOM_MASS` | 100 |
| `GET_ATOM_NONCE` | 100 |
| `GET_CRYSTAL_INDEX` | 10 |
| `GET_CALLER_PK` | 50 |
| `TRANSFER_MASS` | 500 |
| `EMIT_EVENT` | 200 |
| `VERIFY_SIGNATURE` | 5000 |
| `REMAINING_GAS` | 0 |

## Host Functions (primus_v1 namespace)
| Function | Signature | Gas Cost | Return convention |
| :--- | :--- | :--- | :--- |
| `get_atom_mass` | `(pk_ptr: i32, pk_len: i32) -> i64` | 100 | Mass of the atom, or -1 if not found. |
| `get_atom_nonce` | `(pk_ptr: i32, pk_len: i32) -> i64` | 100 | Nonce of the atom, or -1 if not found. |
| `get_crystal_index` | `() -> i64` | 10 | Current crystal index. |
| `get_caller_pk` | `(out_ptr: i32) -> i32` | 50 | Length of PK written to buffer, or -1 on error. |
| `transfer_mass` | `(f_ptr: i32, f_len: i32, t_ptr: i32, t_len: i32, amt: i64) -> i32` | 500 | 0=Success, 1=Insuff. mass, 2=Not owner, 3=Err, 4=OOG. |
| `emit_event` | `(t_ptr: i32, t_l: i32, d_ptr: i32, d_l: i32) -> i32` | 200 | 0=Success, 1=OOG, 2=Limit reached, 3=Invalid input. |
| `verify_signature` | `(pk: i32, pk_l: i32, msg: i32, msg_l: i32, sig: i32, sig_l: i32) -> i32` | 5000 | 1=Valid, 0=Invalid, -1=Error. |
| `remaining_gas` | `() -> i64` | 0 | Returns the remaining gas in the meter. |

## Resource Limits (16 MiB Mandate)
* `MAX_WASM_MEMORY_PAGES`: 256 — 256 pages × 64 KiB = 16 MiB mandate.
* `MAX_WASM_STACK_SIZE`: 524288 — 512 KiB maximum call stack size.
* `MAX_MODULE_SIZE_BYTES`: 4194304 — 4 MiB maximum size of WASM module binary.
* `MODULE_CACHE_SIZE`: 256 — LRU cache capacity for compiled modules.
* `MAX_CONTRACT_EVENTS`: 64 — Maximum number of events a single contract invocation may emit.
* `MAX_EVENT_SIZE_BYTES`: 1024 — Maximum size of a single contract event (topic + data combined).
* `GAS_HEAT_DIVISOR`: 1000.0 — Divisor for converting gas usage to crystal heat contribution.

## Physics Engine
The physics engine simulates the environmental and computational costs of state transitions:
* `get_galactic_drift`: Determines which sector of the mempool is resonant for the current crystal index.
* `calculate_orbital_resonance`: Grants a curvature discount if the atom's first public-key byte matches the current galactic drift.
* `calculate_gravity_assist_from_iter`: Reduces spacetime curvature based on proximity to high-mass "star" atoms (TODO: Requires `iter_atoms` in `StateView`).
* `get_spacetime_curvature`: Computes the base heat contribution derived from the reaction hash and chamber temperature.
* `calculate_macro_shift`: Applies a complexity multiplier when local heat exceeds the critical threshold.
* `calculate_entropy_tax`: Computes the final computational cost of a reaction, scaled by local temperature.

Constants:
- `THERMAL_CAPACITY`: 1000.0
- `GRAVITY_SHIELD_GATE`: 150.0
- `MACRO_SHIFT_CRITICAL`: 250.0
- `MAX_GRAVITY_PULL`: 25.0

## Error Reference
| Variant | When it is returned |
| :--- | :--- |
| `SignatureRejected` | An ML-DSA-87 signature fails verification against the signing digest. |
| `NonceMismatch` | The reaction nonce does not match the sender's on-chain nonce. |
| `InsufficientMass` | An atom lacks the mass required for a transfer or contract storage. |
| `ThermalLimitExceeded` | Total crystal heat exceeds the `THERMAL_CAPACITY` limit. |
| `QuantumCollapse` | An entangled atom reacts without its partner being active in the same batch. |
| `InvalidRewardRecipient` | A `MiningReward` payload is sent to an address other than the Architect. |
| `UnknownPayload` | The VM encounters a payload variant it does not recognize. |
| `NegativeEnergy` | A reaction contains a negative energy value. |
| `AtomNotFound` | The source atom (sender) is missing from the state. |
| `ArithmeticOverflow` | An internal calculation results in an arithmetic overflow. |
| `OutOfGas` | A WASM contract exceeds its allocated gas limit. |
| `WasmTrap` | A WASM execution encounters a runtime trap. |
| `ContractNotFound` | The WASM bytecode for the requested hash is missing from storage. |
| `CompilationFailed` | The WASM module fails to compile in the backend. |
| `InvalidModule` | The WASM module structure is invalid. |
| `HostViolation` | A host function call violates safety or protocol rules. |
| `GasOverflow` | Gas consumption tracking exceeds `u64::MAX`. |
| `WasmBackendError` | A general error from the underlying WASM engine (Wasmtime/Wasmer). |
| `Other` | A generic catch-all for errors wrapped in `anyhow`. |

## Security Invariants
1. Zero signature bypasses on user transactions.
2. Every Transfer / Generic MUST carry a valid ML-DSA-87 signature.
3. MiningReward is the sole signature-exempt path.
4. All arithmetic uses checked/saturating ops.
5. Thermal capacity limit (1000.0) still applies.

## Adding a New Host Function
1. Define a new gas cost constant in `src/wasm/gas.rs` within the `costs` module.
2. Implement the function logic within `WasmtimeRuntime::create_linker` in `src/wasm/wasmtime_backend.rs`.
3. Call `caller.data_mut().gas.charge(costs::YOUR_NEW_COST)?` as the first step in the implementation.
4. Register the function under the `primus_v1` namespace using `linker.func_wrap`.
5. Use the `read_wasm_bytes` and `write_wasm_bytes` helpers for interacting with WASM linear memory.

## Running Tests
```sh
cargo test
cargo test --features wasmtime-backend
```

## License
See LICENSE in the workspace root.
