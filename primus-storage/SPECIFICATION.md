# primus-storage — Crate Specification

**Version:** 0.2.0-draft  
**Status:** Pre-implementation  
**Author:** Primus Project  
**Date:** 2026-05-02

---

## 1. Purpose

`primus-storage` is the persistent ledger layer for Obsidian Nexus. It owns all disk I/O and provides the canonical State Root via a **Merkle-Patricia Trie** (MPT), replacing the current flat BLAKE3 hash over a BTreeMap. Every other crate reads and writes state exclusively through this crate's public API — no crate touches Sled directly.

---

## 2. Position in the Crate Graph

```
primus-types  ←──────────────────────────────────────┐
     ↑                                                │
primus-storage  ←── primus-core                       │
     ↑               ↑                                │
primus-net-opt ──────┘        primus-sdk ─────────────┘
     ↑                              ↑
primus-cli ─────────────────────────┘
```

**Dependency rules (enforced at compile time):**

| Crate | Depends on primus-storage? | Role |
|---|---|---|
| `primus-types` | ❌ No | Wire types only, zero storage knowledge |
| `primus-storage` | ✅ primus-types | Persistence + Merkle proofs |
| `primus-core` | ✅ primus-storage | Consensus, PVM, engine |
| `primus-net-opt` | ✅ primus-storage (read-only) | Sync, proof delivery |
| `primus-sdk` | ✅ primus-storage (read-only) | Balance queries, proof verification |
| `primus-cli` | ✅ primus-storage (read-only) | `inspect`, `balance`, `root` commands |

`primus-storage` must **never** depend on `primus-core`, `primus-net-opt`, `primus-sdk`, or `primus-cli`. The dependency arrow is strictly downward.

---

## 3. Current State (What Exists)

### 3.1 `storage.rs` — `PrimusStorage`

Sled-backed key-value store. Current key schema:

| Key pattern | Value | Notes |
|---|---|---|
| `atom_{pk}` | `bincode(Atom)` | One entry per on-chain atom |
| `crystal_{index}` | `bincode(Crystal)` | One entry per block |
| `undo_{index}` | `bincode(UndoLog)` | Rollback log, pruned at `UNDO_WINDOW=8` |
| `global_metrics` | `bincode(GlobalMetrics)` | Temperature + entropy |
| `global_crystal_index` | `u64 LE` | Tip height |
| `latest_index` | `u64 LE` | Fast latest-crystal lookup |

**Known issues:**
- `get_crystal_latest()` falls back to a full scan over `crystal_*` if `latest_index` is missing — O(N) on restart.
- `prune_undo_window()` iterates `0..cutoff` deleting one key at a time — O(cutoff) Sled ops on every block.
- `save_global_metrics()` calls `db.flush()` internally; `save_state_changes()` also calls `db.flush()` — two fsyncs per block minimum, can be batched into one.
- No Merkle proof generation. `calculate_root_hash()` lives in `state.rs` (primus-core) as a BLAKE3 scan over all atoms — not a Merkle tree, cannot produce inclusion proofs.

### 3.2 `state.rs` — `StateTree` + `Changeset`

In-memory hot storage. Lives in `primus-core` today, will be split:
- `Changeset` and `GlobalMetrics` move to `primus-storage`.
- `StateTree` (in-memory atom map) stays in `primus-core` as the runtime cache.
- `calculate_root_hash()` is replaced by `MerklePatriciaTrie::root()`.

### 3.3 `mempool.rs` — `Mempool` (legacy)

Uses `ReactionResult` (old type). **Will be deleted.** Replaced by `mempool_v2.rs`.

### 3.4 `mempool_v2.rs` — `SectoralMempool`

256-sector Sled-backed mempool using `SignedReaction` (primus-types). Stays in `primus-core` — the mempool is an in-flight buffer, not permanent state. `primus-storage` does not own it.

---

## 4. Target Architecture

### 4.1 Merkle-Patricia Trie (MPT)

The MPT replaces the flat `BTreeMap` + BLAKE3 state root. It provides:

1. **Inclusion proofs** — SDK and light clients can verify an atom's balance without downloading the full state.
2. **Exclusion proofs** — prove an atom does not exist (needed for receiver materialization checks).
3. **Subtree hashing** — each sector (first byte of public key) has its own subtree root, enabling parallel state root computation.
4. **Incremental updates** — only the path from a changed leaf to the root is rehashed per block, not all atoms.

**Key encoding:** The MPT key for an atom is `SHA3-256(public_key)` — 32 bytes, giving a balanced trie depth of 256 bits.

**Value encoding:** `bincode(Atom)` — same as today, wire-compatible with existing Sled data.

**Hash function:** BLAKE3 at each internal node (already used by `crystal.rs` for block density hash).

**Storage layout in Sled:**

| Key | Value | Description |
|---|---|---|
| `mpt_node_{hash32}` | `bincode(MptNode)` | Internal or leaf node |
| `mpt_root` | `[u8; 32]` | Current trie root hash |
| `mpt_root_{crystal_index}` | `[u8; 32]` | Historical root per block (for proof queries) |

**MptNode variants:**
```rust
pub enum MptNode {
    /// Leaf: full key suffix + value
    Leaf { key_suffix: Vec<u8>, value: Vec<u8> },
    /// Extension: shared prefix + child hash
    Extension { prefix: Vec<u8>, child: [u8; 32] },
    /// Branch: 16 children (nibble-indexed) + optional value
    Branch { children: [Option<[u8; 32]>; 16], value: Option<Vec<u8>> },
}
```

### 4.1.1 `MptNode` Serialization & Security
The `MptNode` struct and its serialization format is a core consensus component. Nodes are stored as complete `Vec<u8>` bytes representing the canonical MPT topology. Because the trie only contains public keys, cryptographic hashes, and public ledger balances, there are no cryptographic secrets (like private keys or mnemonics) within the MPT. 

As such, `MptNode` **deliberately omits** `Zeroize` and `ZeroizeOnDrop` memory sanitization traits. Using `Zeroize` on `MptNode` would needlessly corrupt consensus data during memory-intensive trie traversals, leading to node instability.

### 4.2 `PrimusStorage` — Revised Public API

```rust
pub struct PrimusStorage { /* sled::Db + cached root */ }

impl PrimusStorage {
    // ── Construction ──────────────────────────────────────────────────────────
    pub fn new(path: &str) -> Result<Self>;
    pub fn get_db(&self) -> &Db; // kept for primus-core::SectoralMempool

    // ── Atom I/O (MPT-backed) ─────────────────────────────────────────────────
    pub fn get_atom(&self, pk: &[u8]) -> Result<Option<Atom>>;
    pub fn get_all_atoms(&self) -> Result<BTreeMap<Vec<u8>, Atom>>;

    // ── Changeset commit (single fsync, single MPT update) ───────────────────
    /// Apply a Changeset to the MPT and Sled atomically.
    /// Updates the trie root. Returns the new root hash.
    pub fn commit_changeset(
        &self,
        changeset: &Changeset,
        crystal_index: u64,
    ) -> Result<[u8; 32]>;

    // ── Merkle proof API ──────────────────────────────────────────────────────
    /// Generate an inclusion or exclusion proof for a public key.
    pub fn prove(&self, pk: &[u8]) -> Result<MerkleProof>;

    /// Verify a proof against a known root hash (used by SDK + light clients).
    /// This implementation enforces strict path-traversal verification, ensuring
    /// the full MPT path matches the provided root before succeeding.
    pub fn verify_proof(
        root: &[u8; 32],
        pk: &[u8],
        proof: &MerkleProof,
    ) -> bool;

    // ── State root ────────────────────────────────────────────────────────────
    pub fn current_root(&self) -> [u8; 32];
    pub fn root_at(&self, crystal_index: u64) -> Result<Option<[u8; 32]>>;

    // ── Crystal I/O (unchanged) ───────────────────────────────────────────────
    pub fn get_crystal(&self, index: u64) -> Result<Option<Crystal>>;
    pub fn get_crystal_latest(&self) -> Result<Option<Crystal>>;
    pub fn save_crystal(&self, crystal: &Crystal) -> Result<()>;
    pub fn delete_crystal(&self, index: u64) -> Result<()>;

    // ── UndoLog I/O (unchanged) ───────────────────────────────────────────────
    pub fn save_undo_log(&self, log: &UndoLog) -> Result<()>;
    pub fn get_undo_log(&self, crystal_index: u64) -> Result<Option<UndoLog>>;
    pub fn delete_undo_log(&self, crystal_index: u64) -> Result<()>;
    pub fn prune_undo_window(&self, current_height: u64);

    // ── Metrics ───────────────────────────────────────────────────────────────
    pub fn save_global_metrics(&self, metrics: &GlobalMetrics) -> Result<()>;
    pub fn get_global_metrics(&self) -> Result<Option<GlobalMetrics>>;

    // ── Flush (one per block, not per method) ────────────────────────────────
    pub fn flush(&self, metrics: Option<&GlobalMetrics>) -> Result<()>;

    // ── Rollback ──────────────────────────────────────────────────────────────
    pub fn restore_atoms(&self, pre_images: &BTreeMap<Vec<u8>, Option<Atom>>) -> Result<()>;
}
```

### 4.3 `MerkleProof`

```rust
pub struct MerkleProof {
    /// The public key being proved (or disproved).
    pub key: Vec<u8>,
    /// The value at this key, or None for exclusion proofs.
    pub value: Option<Vec<u8>>,
    /// Sibling hashes on the path from root to leaf.
    pub siblings: Vec<[u8; 32]>,
    /// The root this proof was generated against.
    pub root: [u8; 32],
}
```

Wire-serializable via `serde`. Used by:
- `primus-sdk`: `Wallet::get_balance_with_proof()` — returns balance + proof for light client verification.
- `primus-net-opt`: `SyncMessage::ProofResponse` — allows peers to verify state without full download.
- `primus-cli`: `primus balance --prove` — prints hex proof for external verification.

### 4.4 `Changeset`

Moves from `primus-core::state` to `primus-storage`:

```rust
/// Write-set for a single Crystal. BTreeMap enforces deterministic iteration order.
pub struct Changeset {
    pub inner: BTreeMap<Vec<u8>, Atom>,
}
```

`primus-core` creates `Changeset` values and passes them to `PrimusStorage::commit_changeset()`. The storage crate owns the persistence.

---

## 5. Migration Plan

### Phase 1 — Extract (no behaviour change)
1. Move `mempool.rs` deletion — dead code, uses old `ReactionResult`.
2. Move `storage.rs`, `state::Changeset`, `state::GlobalMetrics`, `state::UndoLog` into `primus-storage`.
3. Update `primus-core/Cargo.toml`: `primus-storage = { path = "../primus-storage" }`.
4. Remove duplicate `FINALITY_DEPTH` from `processor.rs` (it's in `storage.rs` already).
5. All existing tests pass — no behaviour change yet.

### Phase 2 — MPT implementation
1. Add `mpt.rs` — `MptNode`, `MerklePatriciaTrie`, `MerkleProof`.
2. Add `mpt_store.rs` — Sled-backed node persistence.
3. Replace `PrimusStorage::commit_changeset()` internals to update the trie.
4. Replace `StateTree::calculate_root_hash()` in `primus-core` with `storage.current_root()`.
5. Add `prove()` and `verify_proof()`.

### Phase 3 — Integration
1. `primus-sdk`: add `get_balance_with_proof()` using `PrimusStorage::prove()`.
2. `primus-net-opt`: add `SyncMessage::ProofRequest / ProofResponse` variants.
3. `primus-cli`: add `--prove` flag to `balance` command.
4. Light client path: verify proof without `PrimusStorage` instance (pure function `verify_proof`).

### Phase 4 — Performance
1. Batch `db.flush()` — one fsync per block instead of per method call.
2. Cache the current trie root in memory to avoid Sled read on every `current_root()` call.
3. Parallel sector root computation — 256 subtrees can be hashed concurrently via rayon.

---

## 6. `Cargo.toml` — Target

```toml
[package]
name    = "primus-storage"
version = "0.2.0"
edition = "2024"

[dependencies]
primus-types = { path = "../primus-types" }
sled         = "0.34"
bincode      = "1"
serde        = { version = "1", features = ["derive"] }
anyhow       = "1"
blake3       = "1"
sha3         = { version = "0.10", default-features = false }
hex          = "0.4"

[dev-dependencies]
tempfile = "3"
```

No dependency on `primus-core`, `primus-net-opt`, `primus-sdk`, or `primus-cli`.

---

## 7. Invariants (Must Never Break)

1. **Single fsync per block.** `commit_changeset()` + `save_crystal()` + `save_global_metrics()` must be batched into a single `db.flush()` call. Multiple fsyncs per block kill throughput on spinning disks.

2. **MPT root == State root.** `storage.current_root()` and `StateTree::calculate_root_hash()` must produce the same value during Phase 2 transition. A test that runs both and asserts equality is required before Phase 2 is merged.

3. **Changeset iteration is deterministic.** `Changeset::inner` is `BTreeMap` — never `HashMap`. This is a consensus invariant: every node must apply atoms in the same order.

4. **Proof verification is pure.** `verify_proof()` is a free function with no Sled dependency. It must be usable in WASM (primus-sdk browser build) with zero I/O.

5. **Backward compatibility.** The flat `atom_{pk}` Sled keys must remain readable after the MPT migration. The MPT is an additional index on top, not a replacement of the raw atom storage. This allows rollback without MPT reconstruction.

6. **`UNDO_WINDOW = 8` is enforced.** Proofs older than 8 blocks may reference a trie root that is no longer in Sled. Proof queries for `crystal_index < current_height - UNDO_WINDOW` must return `Err(ProofTooOld)`.

---

## 8. Open Questions

| # | Question | Decision needed by |
|---|---|---|
| 1 | Use `jellyfish-merkle` crate or roll our own MPT? | Phase 2 start |
| 2 | Nibble width: 4-bit (standard Ethereum MPT) or 8-bit (simpler)? | Phase 2 start |
| 3 | Should `MerkleProof` be in `primus-types` (wire format) or `primus-storage`? | Phase 3 start |
| 4 | Historical root retention: keep all or only last `UNDO_WINDOW`? | Phase 2 start |
| 5 | Async Sled API vs sync? Current code is sync; tokio tasks call it from async context via `spawn_blocking`. Acceptable for now. | Phase 4 |

---

## 9. Known Limitations

| ID | Description | Status |
|---|---|---|
| L1 | Orphan MPT nodes | ✅ FIXED — GC via gc_since() called in prune_undo_window() |
| L2 | Large proofs ~25 KB | ✅ FIXED — compact siblings, ~2 KB per proof |
| L3 | Mutex blocks prove() | ✅ FIXED — RwLock<root> + lock-free prove() |
| L4 | root_at() Ok(None) no context | ✅ FIXED — StorageError::ProofTooOld with tip/window context |