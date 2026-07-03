# Technical Specification: primus-vm

**Version:** 0.1.0-draft  
**Status:** Pre-implementation  
**Date:** 2026-05-04  
**Scope:** Extraction of PVM from `primus-core` + WASM execution engine

---

## 1. Purpose & Position in the Architecture

`primus-vm` is a standalone crate that:

1. **Absorbs** the existing `PVM` from `primus-core/pvm.rs` and exposes it as a public, versioned extension point.
2. **Adds a WASM engine** (`WasmRuntime`) built on `wasmtime` (primary) and `wasmer` (fallback / hot-swap), enabling smart contracts as `.wasm` modules to execute as a new `Payload::Contract` variant.
3. **Guarantees determinism** — a hard consensus requirement: identical input → identical output on every platform.

### Position in the Dependency Graph

```
primus-types
     ↑
primus-storage
     ↑
primus-vm          ← NEW CRATE
     ↑
primus-core        (replaces the embedded pvm.rs)
     ↑
primus-net-opt
     ↑
primus-cli / primus-sdk
```

**Dependency rules:**

| Crate | Depends on primus-vm? | Role |
|---|---|---|
| `primus-types` | ❌ | Wire types only, zero VM knowledge |
| `primus-storage` | ❌ | Persistence only |
| `primus-vm` | ✅ `primus-types`, `primus-storage` (read-only) | Transaction & contract execution |
| `primus-core` | ✅ `primus-vm` | Consensus, Kinetic Engine |
| `primus-net-opt` | ❌ | Transport only |
| `primus-sdk` | ✅ `primus-vm` (optional) | Local contract simulation |

`primus-vm` **must never** depend on `primus-core`, `primus-net-opt`, or `primus-cli`.

---

## 2. Crate Layout

```
primus-vm/
├── Cargo.toml
├── src/
│   ├── lib.rs              — public API, re-exports
│   ├── pvm.rs              — PVM (migrated from primus-core, logic unchanged)
│   ├── physics.rs          — physics helpers (galactic_drift, resonance, etc.)
│   ├── context.rs          — ExecutionContext, HostState
│   ├── wasm/
│   │   ├── mod.rs          — WasmRuntime trait + dispatcher
│   │   ├── wasmtime.rs     — wasmtime implementation (primary)
│   │   ├── wasmer.rs       — wasmer implementation (fallback)
│   │   ├── host_api.rs     — host functions imported by contracts
│   │   ├── gas.rs          — gas model (deterministic GasMeter)
│   │   ├── limits.rs       — constants: MAX_MEMORY, MAX_GAS, MAX_DEPTH
│   │   └── sandbox.rs      — isolation, float + SIMD instruction bans
│   ├── dispatch.rs         — PayloadDispatcher (Native + Wasm)
│   └── error.rs            — PvmError enum
└── tests/
    ├── pvm_integration.rs
    ├── wasm_determinism.rs
    └── gas_metering.rs
```

---

## 3. PVM (Native Engine)

### 3.1 Migration from primus-core

`pvm.rs` is migrated **without any logic changes**. Only the import paths change:

```rust
// Was in primus-core:
use crate::atom::{AtomCoreExt, QuantumLogic};
use crate::crypto::Crypto;
use crate::kinetic::{Payload, SignedReaction};
use crate::state::{Changeset, StateTree};

// Becomes in primus-vm:
use primus_types::atom::{AtomCoreExt, QuantumLogic};
use primus_types::kinetic::{Payload, SignedReaction};
use primus_storage::{Changeset, StateTree};
// Crypto stays in primus-core — injected via CryptoVerifier trait (see 3.2)
```

### 3.2 Dependency Inversion for Crypto

To prevent `primus-vm` from depending on `primus-core::crypto`, a trait is introduced:

```rust
// context.rs
pub trait CryptoVerifier: Send + Sync {
    fn verify(pk: &[u8], digest: &[u8], sig: &[u8]) -> bool;
}
```

`primus-core` implements `CryptoVerifier` via ML-DSA-87 and injects it into `PVM::execute_payload`. Tests can substitute a mock implementation.

### 3.3 ExecutionContext

```rust
pub struct ExecutionContext<'a, C: CryptoVerifier> {
    /// Read-only state snapshot (StateTree view)
    pub state: &'a dyn StateView,
    /// Architect's public key
    pub architect_pk: &'a [u8],
    /// Current chamber temperature (from ReactionChamber)
    pub current_temp: f32,
    /// Current crystal index
    pub crystal_index: u64,
    /// Cryptography provider (zero-size phantom)
    pub _crypto: std::marker::PhantomData<C>,
}
```

`StateView` is a trait with `get_atom(pk) -> Option<Atom>` and `crystal_index()`. This allows testing the PVM without a live Sled instance.

### 3.4 PVM — Public API

```rust
impl PVM {
    /// Main entry point — executes a batch of SignedReactions.
    /// Returns (Changeset, consumed_entropy).
    pub fn execute_payload<C: CryptoVerifier>(
        ctx: &ExecutionContext<C>,
        payload: &[SignedReaction],
    ) -> Result<(Changeset, f32), PvmError>;

    // --- Physics helpers (live in physics.rs, re-exported here) ---
    pub fn get_galactic_drift(crystal_index: u64) -> u8;
    pub fn calculate_orbital_resonance(atom_id: &[u8], drift: u8) -> f32;
    pub fn calculate_gravity_assist(state: &dyn StateView, atom_id: &[u8]) -> f32;
    pub fn get_spacetime_curvature(rx_hash: &[u8; 32], base_temp: f32) -> f32;
    pub fn calculate_macro_shift(temp: f32) -> f32;
    pub fn calculate_entropy_tax(complexity: u64, local_temp: f32) -> u64;
}
```

### 3.5 Thermal Constants (unchanged)

| Constant | Value | Source |
|---|---|---|
| `THERMAL_CAPACITY` | `1000.0` | PVM thermal limit per crystal |
| `GRAVITY_SHIELD_GATE` | `150.0` | Chamber overheat threshold |
| `MACRO_SHIFT_CRITICAL` | `250.0` | Macro-shift trigger |
| `MAX_GRAVITY_PULL` | `25.0` | Cap on gravity_assist |

---

## 4. WASM Engine

### 4.1 Overview

WASM contracts are a new `Payload::Contract { code_hash, calldata }` variant. This extends `primus-types::Payload`:

```rust
// In primus-types/src/kinetic.rs — new variant appended:
pub enum Payload {
    Generic,
    Transfer { amount: u64 },
    MiningReward { amount: u64 },
    Unknown,
    /// NEW: WASM smart-contract invocation
    Contract {
        /// SHA3-256 hash of the .wasm bytecode (code stored separately in PrimusStorage)
        code_hash: [u8; 32],
        /// ABI-encoded call arguments (bincode)
        calldata: Vec<u8>,
    },
}
```

Contract bytecode (`Vec<u8>`) is stored in `primus-storage` under key `contract_{code_hash}` and is **not** duplicated inside each transaction.

### 4.2 WasmRuntime Trait

```rust
// wasm/mod.rs
pub trait WasmRuntime: Send + Sync {
    /// Compile and cache a module by code_hash.
    fn load_module(&self, code_hash: [u8; 32], wasm_bytes: &[u8]) -> Result<(), WasmError>;

    /// Execute a contract call.
    /// Returns: (output_bytes, gas_used).
    fn execute(
        &self,
        code_hash: [u8; 32],
        calldata: &[u8],
        host_state: HostState,
        gas_limit: u64,
    ) -> Result<ContractOutput, WasmError>;

    /// Returns the engine name (for logs and diagnostics).
    fn engine_name(&self) -> &'static str;
}

pub struct ContractOutput {
    pub return_data: Vec<u8>,
    pub gas_used: u64,
    /// State changes requested by the contract via Host API.
    pub state_delta: ContractDelta,
}

pub struct ContractDelta {
    /// Atoms to update (merged into Changeset by the dispatcher).
    pub atom_updates: Vec<(Vec<u8>, AtomPatch)>,
    /// Events emitted (for SDK subscribers).
    pub events: Vec<ContractEvent>,
}
```

### 4.3 Implementation: Wasmtime (Primary)

**Rationale:** `wasmtime` is the primary engine because:
- Cranelift compilation is deterministic when `Config::epoch_interruption(false)`.
- Native `fuel` support provides a deterministic gas model.
- Actively maintained by the Bytecode Alliance.

```rust
// wasm/wasmtime.rs
use wasmtime::{Config, Engine, Linker, Module, Store};

pub struct WasmtimeRuntime {
    engine: Engine,
    /// LRU cache of compiled modules (code_hash → Module).
    module_cache: Arc<Mutex<LruCache<[u8; 32], Module>>>,
}

impl WasmtimeRuntime {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config
            .wasm_simd(false)           // banned — non-deterministic float across x86/ARM
            .wasm_threads(false)        // banned — shared memory is non-deterministic
            .wasm_reference_types(false)// banned — GC dependency
            .wasm_bulk_memory(true)     // allowed — deterministic
            .consume_fuel(true)         // gas model
            .epoch_interruption(false);
        let engine = Engine::new(&config)?;
        Ok(Self {
            engine,
            module_cache: Arc::new(Mutex::new(LruCache::new(256))),
        })
    }
}
```

**Feature flags disabled for determinism:**

| Feature | Status | Reason |
|---|---|---|
| `wasm_simd` | ❌ Banned | Float divergence between x86 and ARM |
| `wasm_threads` | ❌ Banned | Non-deterministic shared memory |
| `wasm_reference_types` | ❌ Banned | GC dependency |
| `float` instructions | ✅ Allowed (via PhysicsCanon) | Raw f32/f64 in hashes is a consensus bug |
| `bulk_memory` | ✅ Allowed | Deterministic |
| `tail_calls` | ✅ Allowed | Deterministic |

### 4.4 Implementation: Wasmer (Fallback)

`wasmer` is used as a fallback when Cranelift compilation fails on exotic platforms, and for hot-swap engine replacement without node restart.

```rust
// wasm/wasmer.rs — activated via feature flag
#[cfg(feature = "wasmer-backend")]
pub struct WasmerRuntime {
    store: wasmer::Store,
    module_cache: Arc<Mutex<LruCache<[u8; 32], wasmer::Module>>>,
}
```

**Feature flags in Cargo.toml:**

```toml
[features]
default          = ["wasmtime-backend"]
wasmtime-backend = ["dep:wasmtime"]
wasmer-backend   = ["dep:wasmer"]
# Enabling both simultaneously produces a compile error via cfg-check.
```

**Dispatcher:**

```rust
// wasm/mod.rs
pub enum RuntimeKind {
    Wasmtime(WasmtimeRuntime),
    #[cfg(feature = "wasmer-backend")]
    Wasmer(WasmerRuntime),
}

impl WasmRuntime for RuntimeKind {
    fn execute(...) -> Result<ContractOutput, WasmError> {
        match self {
            RuntimeKind::Wasmtime(rt) => rt.execute(...),
            #[cfg(feature = "wasmer-backend")]
            RuntimeKind::Wasmer(rt) => rt.execute(...),
        }
    }
}
```

### 4.5 Gas Model

Gas is anchored to the **entropy_tax** from the native PVM, giving a unified cost model:

```rust
// wasm/gas.rs

/// 1 gas unit = 1 PVM complexity unit.
/// Base cost per contract call: 10_000 gas.
pub const BASE_CONTRACT_GAS: u64 = 10_000;

/// Gas limit = transaction energy × GAS_PER_ENERGY.
pub const GAS_PER_ENERGY: u64 = 100;

/// Maximum gas per reaction (DoS protection).
pub const MAX_GAS_PER_REACTION: u64 = 1_000_000;

pub struct GasMeter {
    pub limit: u64,
    pub consumed: u64,
}

impl GasMeter {
    pub fn from_energy(energy: f32) -> Self {
        let limit = ((energy as u64) * GAS_PER_ENERGY)
            .min(MAX_GAS_PER_REACTION)
            .max(BASE_CONTRACT_GAS);
        Self { limit, consumed: 0 }
    }

    pub fn charge(&mut self, amount: u64) -> Result<(), WasmError> {
        self.consumed = self.consumed.checked_add(amount)
            .ok_or(WasmError::GasOverflow)?;
        if self.consumed > self.limit {
            Err(WasmError::OutOfGas { limit: self.limit, consumed: self.consumed })
        } else {
            Ok(())
        }
    }
}
```

**Integration with the thermal model:** gas consumed by a contract is added to `total_crystal_heat` via:

```
contract_heat = gas_used as f32 / GAS_HEAT_DIVISOR   (GAS_HEAT_DIVISOR = 1000.0)
```

This ensures that heavy contracts affect `thermal_capacity` on equal footing with native reactions.

### 4.6 Host API

Contracts communicate with blockchain state through imported host functions:

```rust
// wasm/host_api.rs

/// Functions exported by the host into the WASM module.
/// Namespace: "primus_v1"

// — Read state —
fn get_atom_mass(pk_ptr: i32, pk_len: i32) -> i64;
    // Returns atom mass, or -1 if not found.

fn get_atom_nonce(pk_ptr: i32, pk_len: i32) -> i64;
    // Returns atom nonce, or -1 if not found.

fn get_crystal_index() -> i64;
    // Current crystal index.

fn get_caller_pk(out_ptr: i32) -> i32;
    // Writes the caller's public key to the buffer, returns byte length.

// — Write state (via ContractDelta — never directly into StateTree) —
fn transfer_mass(
    from_ptr: i32, from_len: i32,
    to_ptr:   i32, to_len:   i32,
    amount:   i64,
) -> i32;  // 0 = ok, 1 = insufficient mass, 2 = unauthorized

fn emit_event(
    topic_ptr: i32, topic_len: i32,
    data_ptr:  i32, data_len:  i32,
) -> i32;

// — Cryptography (verification only, no signing) —
fn verify_signature(
    pk_ptr:  i32, pk_len:  i32,
    msg_ptr: i32, msg_len: i32,
    sig_ptr: i32, sig_len: i32,
) -> i32;  // 1 = valid, 0 = invalid

// — Gas introspection —
fn remaining_gas() -> i64;
```

**Contract restrictions:**
- A contract **cannot** directly modify `Atom.nonce` — that is the PVM's exclusive responsibility.
- A contract **cannot** call other contracts (no re-entrancy in v0.1).
- A contract **cannot** make network requests (no I/O).
- A contract **cannot** read wall-clock time — only `crystal_index`.

### 4.7 Sandbox and Memory Limits

```rust
// wasm/limits.rs
pub const MAX_WASM_MEMORY_PAGES: u32  = 256;        // 256 × 64 KiB = 16 MiB (= 16 MiB Mandate)
pub const MAX_WASM_STACK_SIZE: usize  = 512 * 1024; // 512 KiB
pub const MAX_MODULE_SIZE_BYTES: usize = 4 * 1024 * 1024; // 4 MiB
pub const MODULE_CACHE_SIZE: usize    = 256;         // LRU capacity
pub const MAX_CONTRACT_EVENTS: usize  = 64;          // per reaction
pub const MAX_EVENT_SIZE_BYTES: usize = 1024;        // per event
```

The `16 MiB` memory limit is intentionally aligned with the **16 MiB Mandate** from `primus-core` — a single unified resource policy across the entire ecosystem.

### 4.8 Determinism Guarantees

| Requirement | Mechanism |
|---|---|
| Identical execution order | Reactions processed sequentially (never in parallel) |
| No floating-point in hashes | All f32 from contracts encoded via `PhysicsCanon::encode()` before hashing |
| No SIMD | `wasm_simd(false)` in Wasmtime Config |
| No system time | Host API does not expose `clock_gettime` |
| No randomness | Host API does not expose `random_get` |
| Gas limit is deterministic | `fuel` in wasmtime — instruction counter, not time-based |
| ABI versioning | `primus_v1` namespace — future versions do not break existing contracts |

---

## 5. PayloadDispatcher

The unified entry point for `primus-core`:

```rust
// dispatch.rs
pub struct PayloadDispatcher<C: CryptoVerifier> {
    wasm_runtime: Option<Arc<dyn WasmRuntime>>,
    _crypto: PhantomData<C>,
}

impl<C: CryptoVerifier> PayloadDispatcher<C> {
    /// Executes a batch of reactions.
    /// Native reactions → PVM.
    /// Contract reactions → WasmRuntime → merged into Changeset.
    pub fn execute(
        &self,
        ctx: &ExecutionContext<C>,
        reactions: &[SignedReaction],
    ) -> Result<(Changeset, f32), PvmError> {
        // Step 1: PVM pre-filter (semantic validation)
        // Step 2: for each reaction:
        //   if Payload::Contract → self.dispatch_wasm(...)
        //   else                 → PVM::execute_payload(...)
        // Step 3: merge all Changesets
        // Step 4: final thermal check
    }
}
```

This replaces the direct `PVM::execute_payload` call in `primus-core::kinetic`.

---

## 6. Integration with primus-core

### 6.1 Changes to primus-core

- Delete `src/pvm.rs` — migrated into `primus-vm`.
- Add dependency: `primus-vm = { path = "../primus-vm" }`.
- Replace the call site:

```rust
// Before:
let (changeset, entropy) = PVM::execute_payload(&state, &reactions, temp, &arch_pk)?;

// After:
let (changeset, entropy) = dispatcher.execute(&ctx, &reactions)?;
```

### 6.2 Contract Integration into Crystal Synthesis Lifecycle

The existing lifecycle (from the `primus-core` SPECIFICATION) is extended:

1. **Preparation**: Derive deterministic timestamp. *(unchanged)*
2. **Filtering**: PVM pre-filter — now via `PayloadDispatcher::pre_filter()` — handles both native and Contract reactions.
3. **Reward Injection**: Prepend `MiningReward`. *(unchanged)*
4. **Dispatch**: `PayloadDispatcher::execute()` — native reactions via PVM, contracts via WasmRuntime.
5. **PoW Solve**: *(unchanged)*
6. **Solidification**: Merge `ContractDelta` into Changeset before applying to StateTree.
7. **Persistence**: Commit. *(unchanged)*

---

## 7. Contract Storage in primus-storage

`primus-storage` is extended with two new key patterns (Phase 3 migration):

| Key | Value | Notes |
|---|---|---|
| `contract_{code_hash_hex}` | `Vec<u8>` (wasm bytes) | Raw WASM bytecode |
| `contract_meta_{code_hash_hex}` | `bincode(ContractMeta)` | ABI version, average gas_used |

```rust
// PrimusStorage extended with:
pub fn store_contract(&self, code_hash: [u8; 32], wasm_bytes: &[u8]) -> Result<()>;
pub fn load_contract(&self, code_hash: [u8; 32]) -> Result<Option<Vec<u8>>>;
```

Contract deployment is a native `Payload::Generic` transaction whose `calldata` contains the WASM bytes. After confirmation the bytecode is saved to storage; subsequent invocations reference only the `code_hash`.

---

## 8. Error Handling

```rust
// error.rs
#[derive(Debug, thiserror::Error)]
pub enum PvmError {
    // --- Native PVM ---
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

    // --- WASM ---
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
}
```

---

## 9. Invariants (Must Never Be Broken)

1. **PVM is deterministic.** Given the same `ExecutionContext` and `payload`, the result is always identical — on every platform, in every Rust version.

2. **WASM has no I/O.** Contracts have no access to the filesystem, network, wall-clock time, or random numbers.

3. **Gas is charged before execution.** `GasMeter::charge()` is called before every significant operation. Insufficient gas → immediate `OutOfGas` with no partial state application.

4. **ContractDelta is isolated.** Contracts never write directly into `Changeset` — they produce a `ContractDelta` that `PayloadDispatcher` validates and applies. This is the sole write path.

5. **Thermal accounting is end-to-end.** `contract_heat` is added to `total_crystal_heat`. WASM contracts do not bypass `THERMAL_CAPACITY`.

6. **Nonce is incremented only by the native layer.** `Payload::Contract` does not self-increment `sender.nonce` — the `PayloadDispatcher` PVM layer does so after successful execution.

7. **16 MiB Mandate enforced.** `MAX_WASM_MEMORY_PAGES = 256` → 16 MiB. No contract may request more.

8. **Bincode Wire Format is frozen.** `Payload::Contract` is appended as a new variant with `#[serde(default)]` — old nodes that do not know about it receive `Payload::Unknown` and correctly reject the transaction.

---

## 10. `Cargo.toml` — Target Configuration

```toml
[package]
name    = "primus-vm"
version = "0.1.0"
edition = "2024"

[features]
default          = ["wasmtime-backend"]
wasmtime-backend = ["dep:wasmtime"]
wasmer-backend   = ["dep:wasmer"]

[dependencies]
primus-types   = { path = "../primus-types" }
primus-storage = { path = "../primus-storage" }

# Crypto trait (ML-DSA-87 impl stays in primus-core)
anyhow         = "1"
thiserror      = "1"
hex            = "0.4"
sha3           = { version = "0.10", default-features = false }
bincode        = "1"
serde          = { version = "1", features = ["derive"] }
lru            = "0.12"
parking_lot    = "0.12"

# WASM backends (optional)
wasmtime       = { version = "25", optional = true, features = ["cranelift", "fuel"] }
wasmer         = { version = "4", optional = true }

[dev-dependencies]
tempfile = "3"
wat      = "1"   # write test contracts in WAT (WebAssembly Text Format)
```

---

## 11. Migration Plan

### Phase 1 — Extraction (no behaviour change)
1. Create the `primus-vm` crate with `src/pvm.rs` (copy from primus-core).
2. Introduce the `CryptoVerifier` trait; adapt `PVM::execute_payload` to accept it.
3. Introduce `ExecutionContext` and the `StateView` trait.
4. Update `primus-core/Cargo.toml`: add `primus-vm` dependency.
5. Delete `primus-core/src/pvm.rs`; replace all call sites.
6. Migrate all existing PVM tests to `primus-vm/tests/pvm_integration.rs`.
7. CI: all tests green — behaviour unchanged.

### Phase 2 — WASM Native Support (wasmtime)
1. Add `wasm/wasmtime.rs`, `wasm/host_api.rs`, `wasm/gas.rs`, `wasm/limits.rs`.
2. Extend `primus-types::Payload` with the `Contract` variant.
3. Implement `PayloadDispatcher`.
4. Add `primus-storage::store_contract` / `load_contract`.
5. Write determinism tests: single `.wasm`, two nodes, identical Changeset.
6. Write gas metering tests: contract exceeding limit → `OutOfGas`.

### Phase 3 — Wasmer Fallback
1. Implement `wasm/wasmer.rs` under `feature = "wasmer-backend"`.
2. CI matrix: both features on Linux / Windows / macOS.
3. Hot-swap test: switch engines without restarting the node.

### Phase 4 — Contract Deploy Workflow
1. SDK: `TransactionBuilder::deploy_contract(wasm_bytes)`.
2. CLI: `primus contract deploy --file my_contract.wasm`.
3. CLI: `primus contract call --hash <CODE_HASH> --calldata <HEX>`.

---

## 12. Open Questions

| # | Question | Decision needed by |
|---|---|---|
| 1 | Re-entrancy in v0.2? Can a contract call another contract? | Phase 3 |
| 2 | ABI standard for calldata: bincode or custom encoding? | Phase 2 start |
| 3 | Cache the compiled Cranelift artifact in Sled to speed up restarts? | Phase 4 |
| 4 | Per-block event limit (sum across all contracts)? | Phase 2 |
| 5 | Contract upgrade: immutable code_hash or proxy pattern? | Phase 3 |
| 6 | WASM Interface Types (wit-bindgen) for a typed ABI? | Phase 4 |

---

*Last sync: 2026-05-04 | primus-vm v0.1.0-draft*