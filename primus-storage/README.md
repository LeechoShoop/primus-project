# primus-storage — Persistent Ledger Layer for Obsidian Nexus

`primus-storage` is the canonical persistence crate for the Obsidian Nexus node. It owns **all disk I/O**, provides the **Merkle-Patricia Trie (MPT)** state root, and is the exclusive gateway through which every other crate reads and writes ledger state. No crate in the workspace may touch Sled directly.

---

## Architecture Position

```
primus-types
     ↑
primus-storage   ← YOU ARE HERE
     ↑
primus-core   primus-net-opt   primus-sdk   primus-cli
```

**Hard rule:** `primus-storage` depends only on `primus-types`. It must never import from `primus-core`, `primus-net-opt`, `primus-sdk`, or `primus-cli`.

---

## Modules

| Module | Purpose |
|---|---|
| `storage.rs` | `PrimusStorage` — main Sled-backed API: atoms, crystals, undo logs, metrics, flush |
| `mpt.rs` | `MerklePatriciaTrie`, `MptNode`, `verify_proof()` — pure trie logic |
| `mpt_store.rs` | `SledMptStore` — binds the trie to a Sled tree (`mpt_nodes`) |
| `proof_builder.rs` | `ProofBuilder::verify()` — stateless proof verification, WASM-safe |
| `changeset.rs` | `Changeset` — `BTreeMap`-backed write-set for a single crystal |
| `types.rs` | `GlobalMetrics`, `UndoLog` — shared state types |
| `mempool_v2.rs` | `SectoralMempool` — 256-sector Sled mempool with zero-copy ingress |
| `storage.rs` | `StorageError` — typed error enum for storage failures |

---

## Key Design Decisions

### Merkle-Patricia Trie (MPT)

State root is computed by a custom 4-bit nibble MPT:

- **Key encoding:** `SHA3-256(public_key)` → 32 bytes → 64 nibbles
- **Hash function:** BLAKE3 at every internal node
- **Value encoding:** `bincode(Atom)`
- **Proof format:** Compact v2 — `siblings: Vec<[u8;32]>` + `path: Vec<PathStep>` ≈ 2 KB per proof vs 25 KB in the naive approach
- **GC:** `gc_since(old_root)` removes orphan nodes after each block

### Single fsync Per Block

`commit_changeset()` batches all writes — atoms, crystal, metrics, MPT root — into a single `db.flush()` call. Multiple fsyncs per block destroy throughput on spinning disks.

### Changeset Determinism

`Changeset::inner` is a `BTreeMap`. This is a **consensus invariant**: every node must iterate atom updates in the same order. Changing to `HashMap` is a protocol break.

### Sectoral Mempool

`SectoralMempool` partitions 256 sectors, each backed by two Sled trees: one for data, one for weight (fee+timestamp). The zero-copy ingress path uses rkyv structural validation before full deserialization, minimising CPU pressure on the hot path.

### UndoLog & Rollback

Up to `UNDO_WINDOW = 8` blocks of rollback state are retained. `prune_undo_window()` calls `gc_since()` to delete orphan MPT nodes from pruned roots, keeping storage bounded.

---

## Public API Summary

```rust
// Construction
PrimusStorage::new(path: &str) -> Result<Self>
PrimusStorage::get_db() -> &Db

// Atoms (MPT-backed)
get_atom(pk: &[u8]) -> Result<Option<Atom>>
get_all_atoms() -> Result<BTreeMap<Vec<u8>, Atom>>

// Changeset (single fsync)
commit_changeset(cs: &Changeset, crystal_index: u64) -> Result<[u8; 32]>

// Merkle proofs
prove(pk: &[u8]) -> Result<MerkleProof>
verify_proof(root: &[u8;32], pk: &[u8], proof: &MerkleProof) -> bool  // pure, no I/O

// State root
current_root() -> [u8; 32]
root_at(crystal_index: u64) -> Result<Option<[u8; 32]>>

// Crystals
get_crystal(index: u64) -> Result<Option<Crystal>>
get_crystal_latest() -> Result<Option<Crystal>>
save_crystal(crystal: &Crystal) -> Result<()>
delete_crystal(index: u64) -> Result<()>

// UndoLog
save_undo_log(log: &UndoLog) -> Result<()>
get_undo_log(crystal_index: u64) -> Result<Option<UndoLog>>
delete_undo_log(crystal_index: u64) -> Result<()>
prune_undo_window(current_height: u64)

// Metrics
save_global_metrics(metrics: &GlobalMetrics) -> Result<()>
get_global_metrics() -> Result<Option<GlobalMetrics>>

// Flush + Rollback
flush(metrics: Option<&GlobalMetrics>) -> Result<()>
restore_atoms(pre_images: &BTreeMap<Vec<u8>, Option<Atom>>) -> Result<()>
```

---

## Sled Key Schema

| Key | Value | Notes |
|---|---|---|
| `atom_{hex(sha3(pk))}` | `bincode(Atom)` | One per on-chain atom |
| `crystal_{u64_le}` | `bincode(Crystal)` | One per block |
| `undo_{u64_le}` | `bincode(UndoLog)` | Rollback log, pruned at UNDO_WINDOW=8 |
| `global_metrics` | `bincode(GlobalMetrics)` | Temperature + entropy |
| `global_crystal_index` | `u64 LE` | Tip height |
| `latest_index` | `u64 LE` | Fast latest-crystal lookup |
| `mpt_root` | `[u8; 32]` | Current trie root |
| `mpt_root_{u64_le}` | `[u8; 32]` | Historical root per block |
| `mpt_nodes` tree | `bincode(MptNode)` | All trie nodes, keyed by BLAKE3 hash |

---

## Constants

| Constant | Value | Meaning |
|---|---|---|
| `FINALITY_DEPTH` | `6` | Blocks before a crystal is considered final |
| `UNDO_WINDOW` | `8` | Rollback log retention depth |

---

## Error Handling

`StorageError` is a typed error enum:

| Variant | Meaning |
|---|---|
| `ProofTooOld { index, tip, window }` | Proof requested for a pruned historical root |
| `CrystalNotFound(u64)` | Crystal index not in storage |
| `Sled(sled::Error)` | Underlying Sled I/O error |
| `Other(anyhow::Error)` | Catch-all for serialization or logic errors |

---

## Build

```bash
cargo build -p primus-storage --release
cargo test  -p primus-storage
```

**Dependencies:** `primus-types`, `sled`, `bincode`, `serde`, `anyhow`, `blake3`, `sha3`, `thiserror`, `rkyv`, `zeroize`

<!-- Last sync: 2026-05-04 | status: Phase 2 complete — MPT + compact proofs + GC -->