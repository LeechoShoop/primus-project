# Obsidian Nexus — Workspace Technical Specification

This document is the top-level index and cross-crate reference for the Obsidian Nexus protocol. Each crate owns its own detailed `SPECIFICATION.md`; this file summarizes the invariants that span crate boundaries and links to the authoritative per-crate documents.

## 1. Crate Graph & Dependency Rules

```
primus-types  <───────────────────────────────────────────┐
     ^                                                     │
primus-storage <── primus-vm <── primus-core                │
     ^                              ^                       │
primus-net-opt ── (CoreHandle) ─────┘        primus-sdk ─────┘
     ^                                            ^
primus-cli ────────────────────────────────────────┘
```

- `primus-types` depends on nothing else in the workspace. No cryptographic logic lives here (transitional exception: `PrimusNR::verify`).
- `primus-storage` depends only on `primus-types`. It never imports `primus-core`, `primus-net-opt`, `primus-sdk`, or `primus-cli`, and is the **exclusive** owner of Sled I/O.
- `primus-net-opt` has zero compile-time dependency on `primus-core`. It defines the `CoreHandle` trait; `primus-core` implements it (`CoreHandleImpl`). This inversion keeps consensus and networking decoupled.
- `primus-sdk` and `primus-cli` are read-only consumers of `primus-storage`'s proof/state API and of `primus-net-opt`'s wire protocol.

Per-crate detail: [primus-types](primus-types/SPECIFICATION.md) · [primus-storage](primus-storage/SPECIFICATION.md) · [primus-core](primus-core/SPECIFICATION.md) · [primus-net-opt](primus-net-opt/SPECIFICATION.md) · [primus-sdk](primus-sdk/SPECIFICATION.md) · [primus-cli](primus-cli/SPECIFICATION.md)

## 2. Cryptographic Standard

- **Signature scheme**: ML-DSA-87 exclusively. `PK_BYTES = 2592`, `SIG_BYTES = 4627`. No elliptic-curve fallback exists anywhere in the protocol.
- **Stack safety**: ML-DSA-87 signing/verification requires ~4 MiB of stack. All such operations run inside a thread with an explicit **16 MiB stack allocation** (the "16 MiB Mandate"), enforced on both Windows (to avoid `STATUS_STACK_OVERFLOW`) and Linux (to avoid silent stack corruption under concurrent load). On Windows, key material itself (`ml_dsa_sk`) is heap-allocated via `Arc<Box<[u8]>>` rather than placed on the stack.
- **`signing_digest()` vs `reaction_hash`**: semantically distinct SHA3-256 derivations that currently share inputs but are kept separate to allow future divergence.

## 3. Wire Formats

Two serialization formats are used, and they are **never interchangeable**:

| Format | Scope | Notes |
|---|---|---|
| `bincode` | P2P network transmission, disk storage, IPC | Field order is permanently frozen; new fields must be appended with `#[serde(default)]`. Enums serialize by index. |
| `rkyv` | In-process zero-copy hot paths only (mempool scanning, PVM state access) | `ArchivedSignedReaction::validate_structure()` runs after an initial `bincode::deserialize`, not instead of it. |

All core types (`Atom`, `SignedReaction`, `PrimusNR`, `Payload`) derive `serde` + `rkyv` + `Clone + Debug + PartialEq` and satisfy `Send + Sync`.

### PhysicsCanon (cross-architecture determinism)
Any `f32` that participates in a hash (state root, signing digest, fee comparison — i.e. `energy`, `charge`, `current_entropy`, `cumulative_energy`) MUST be encoded via `PhysicsCanon::encode()`: multiply by `FIXED_POINT_SCALE = 10^9`, convert to `u64`. This collapses x86/ARM floating-point divergence. Feeding raw `f32::to_bits()` into a hasher is treated as a consensus-breaking bug.

## 4. Transport & Framing

- **P2P**: QUIC (Kademlia RPC bi-streams, gossip uni-streams) + TCP fallback (chain sync), default port `9000`.
- **Frame limit**: 16 MiB hard ceiling at the application layer on both TCP (`LengthDelimitedCodec`) and QUIC. The Noise Protocol layer itself caps individual messages at 65,535 bytes; larger application payloads are chunked/reassembled below that ceiling.
- **Handshake**: `Noise_XX_25519_ChaChaPoly_SHA256` with a mandatory Identity Binding step — each side's ML-DSA-87 static key signs the peer's ephemeral key before either accepts the session, preventing identity misbinding/MITM.
- **Concurrency**: each peer connection is bounded by a semaphore of 100 concurrent streams/tasks.
- **Client transport**: `primus-sdk`/`primus-cli` use length-prefixed TCP/Bincode (`NodeClient`). `new_with_noise()` / `new_with_ephemeral_noise()` are required for live nodes — the plain `new()` constructor is deprecated and rejected by live nodes.
- **Admin/local IPC**: separate channel, not exposed to the network — see §6.

## 5. GravityShield Ingress Pipeline

Every inbound byte buffer (gossip, RPC, or sync) passes through, in order:

1. **Framing** — reject anything over the 16 MiB limit outright.
2. **Structural filter** — `bincode::deserialize`, then `rx.validate_structure()` (rkyv-backed field-range checks), then basic sanity checks (non-empty public key, non-negative energy).
3. **Thermal gate** — if Chamber Temperature ≥ `150.0 K`, all non-Architect reactions are dropped.
4. **Phantom sender check** — reactions from public keys with no on-chain balance are silently dropped (prevents amplification via error-response probing). Enforced in `primus-core::bridge`, not in `primus-net-opt`.
5. **Zero-copy validation** — `rkyv::check_archive` guards against OOB access / malicious memory layout without a full deserialize.
6. Only after all layers pass is a reaction pushed into the Sectoral Mempool.

## 6. Administrative IPC (Architect Protocol)

Local-only, never sent over the network:

- **Unix**: `$XDG_RUNTIME_DIR/primus.sock` (fallback `$HOME/.primus/run/primus.sock`), `0600` permissions.
- **Windows**: Named Pipe `\\.\pipe\primus-nexus-<USER_SID>`.
- **Challenge-Response**: client requests `GetChallenge` → node returns a 32-byte random nonce → client signs it with the Architect's ML-DSA-87 key → signed command is sent → node verifies against the hardcoded `architect_pk` before executing.
- **Commands**: `Status`, `AdminShutdown { signature }`, `AdminConnectPeer { addr, signature }`, `GetProof { address }`.
- All IPC operations enforce a 5-second I/O timeout; ownership of the socket/pipe is verified before the node accepts a connection.

## 7. State Model & Consensus (Kinetic Engine)

- **Atom**: canonical on-chain identity — `public_key`, `element`, `mass`, `charge`, `nonce`, `last_reaction_hash`, `quantum_state`.
- **SignedReaction**: the unified transaction type, carrying sender/receiver `Atom` snapshots, `energy` (fee), `signature`, and a `Payload` (`Generic` / `Transfer` / `MiningReward` / `Unknown`).
- **Galactic Drift**: `drift = crystal_index % 256` selects which of the 256 mempool sectors is prioritized for the current block.
- **Crystal synthesis lifecycle**: derive deterministic timestamp → PVM pre-filter → prepend `MiningReward` → PoW-style nonce search against a density/difficulty target → commit changeset to the MPT → flush to storage.
- **Validation pipeline (PVM)**: ML-DSA-87 signature check → nonce/anti-replay check → mass conservation (`sender.mass >= amount + fee`) → thermal filter (cumulative crystal heat ≤ `THERMAL_CAPACITY = 1000.0`).
- **Constants**: `PROTOCOL_MIN_FEE = 10`, `MINING_REWARD_AMOUNT = 10`, `GRAVITY_SHIELD_GATE = 150.0 K`, `MACRO_SHIFT_CRITICAL = 250.0`, `FINALITY_DEPTH = 6`, `UNDO_WINDOW = 8`.

## 8. State Root — Merkle-Patricia Trie

- **Key encoding**: `SHA3-256(public_key)` → 32 bytes → 64 nibbles (4-bit nibble width).
- **Hash function**: BLAKE3 at every internal node.
- **Node types**: `Leaf`, `Extension`, `Branch` (16-way).
- **Proof format (v2, compact)**: `{ trie_key, value, root, siblings: Vec<[u8;32]>, path: Vec<PathStep> }`, ≈2 KB per proof (v1's `nodes: Vec<Vec<u8>>` format, ≈25 KB, is removed and is not wire-compatible).
- **Historical roots**: retained for `UNDO_WINDOW = 8` blocks; older proof requests return `StorageError::ProofTooOld`.
- **Determinism invariant**: `Changeset::inner` is a `BTreeMap`, never a `HashMap` — every node must apply atoms in the same order. Changing this is a protocol break.
- **Single fsync per block**: `commit_changeset()` + crystal save + metrics save are batched into one `db.flush()`.
- **`MptNode` deliberately omits `Zeroize`** — it holds only public keys, hashes, and public balances, and consensus data must not be corrupted by memory-sanitization traversal.

## 9. Smart Contracts (WASM / primus-vm)

- Backend: Wasmtime (default), optional Wasmer backend.
- Resource ceiling (the "16 MiB Mandate" applied to contracts): 256 × 64 KiB pages = 16 MiB memory, 512 KiB max call stack, 4 MiB max module size, 256-entry LRU module cache.
- Gas: `energy * GAS_PER_ENERGY`, clamped to `[BASE_CONTRACT_GAS = 10,000, MAX_GAS_PER_REACTION = 1,000,000]`. `gas.charge()` **must** be called before the metered operation, never after.
- Host functions live under the `primus_v1` namespace (`get_atom_mass`, `get_atom_nonce`, `transfer_mass`, `verify_signature`, `emit_event`, etc.), each with a fixed gas cost.
- `MiningReward` is the sole signature-exempt payload; every `Transfer`/`Generic` reaction must carry a valid ML-DSA-87 signature.

## 10. Networking Extras

- **DHT**: Kademlia with "Ping-the-Tail" eviction — when a k-bucket is full, the oldest node is pinged before replacement; only evicted if it fails to respond. Mitigates Eclipse attacks.
- **Gossip**: TTL-decaying epidemic broadcast; message ID = SHA3-256 of the serialized message; deduplication set capped at 10,000 entries (oldest 1,000 evicted on overflow). `NewCrystal` messages do not decay; `NewReaction` decrements TTL by 1 per hop; `Sync` is always treated as TTL = 1.
- **NAT traversal**: UPnP via `igd_next` (`NatService::open_world`) maps the P2P port for TCP and UDP and resolves the external IP for the node's `PrimusNR`.
- **Telemetry**: `frame_drops` is a single `Arc<AtomicU64>` shared between the QUIC server, `PrimusNetwork`, and the IPC server (`Ordering::Relaxed` — exact counts are not consensus-relevant).

## 11. Known Limitations & Recovery

- **Reorg atomicity**: multi-block rollbacks are not wrapped in a single Sled transaction; each block's rollback applies sequentially via `UndoLog`. A crash mid-reorg leaves the node in a partially-rolled-back state, detected on restart via state-root mismatch and repaired by replaying available `UndoLog` entries. Nodes that cannot recover automatically require manual intervention. Planned fix: wrap the full reorg sequence in `sled::Db::transaction` once Sled's multi-tree transaction API stabilizes.
- **`primus-storage` Known Limitations table** (see [primus-storage/SPECIFICATION.md](primus-storage/SPECIFICATION.md) §9) tracks orphan-node GC, proof size, and lock-contention fixes — all currently marked resolved.

## 12. Security Audit Trail

Findings from the pre-publication audit (hardcoded server seed, seed leakage in entropy output, missing `.gitignore` entries for `data/`/`*.db`, hardcoded genesis parameters and bind addresses, hardcoded localhost in TLS identity) were addressed before this repository was made public. In-code changelog tags (`BLK-001`..`BLK-003`, `DIV-001`..`DIV-003`, `INTDIV-001`, `size-guard-G4`, etc., visible in per-crate `<!-- Last sync -->` comments) trace individual fixes back to specific audit findings.

This specification describes the protocol as implemented; it is a portfolio/research project and has not undergone independent third-party audit.
