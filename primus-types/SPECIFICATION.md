# Technical Specification for Primus Ecosystem Types

This document serves as the Single Source of Truth for the `primus-types` crate, defining the core data structures, network primitives, cryptographic standards, serialization constraints, and architectural invariants for the Obsidian Nexus ecosystem.

## Core Entities

### Atom
The `Atom` is the canonical on-chain identity and state of a Primus participant. It acts as both an account and an actor in the state machine.
*   **Purpose**: Represents a user's balance (mass), identity (public key), and state logic (element, quantum state).
*   **Relationship to Reaction/Crystal**: When an Atom participates in a `SignedReaction`, the transaction is validated against its current state snapshot. Upon `Crystal` synthesis (block confirmation), the Atom's `mass`, `nonce`, and `last_reaction_hash` are updated.

### SignedReaction
The `SignedReaction` is the unified transaction type for Obsidian Nexus.
*   **Purpose**: Requests an economic action (like a transfer) from the Primus Virtual Machine (PVM).
*   **Relationship to Crystal**: Reactions are broadcast to the network, held in the mempool, and eventually synthesized into a `Crystal` (a block) by the mining loop. A confirmed reaction updates the state of both the sender and receiver `Atom`.

### Crystal (Conceptual)
While defined in `primus-core`, a `Crystal` represents a sealed block of state changes. It synthesizes a sequence of `SignedReaction`s, locking in their state transitions and establishing consensus through accumulated thermodynamic weight (cumulative energy).

## Data Structures

### `Atom`
| Field | Type | Description |
| :--- | :--- | :--- |
| `public_key` | `Vec<u8>` | ML-DSA-87 verifying key bytes (Length: 2592). |
| `element` | `Element` | Elemental classification governing binding potential and base mass. |
| `neutron_count` | `u32` | Isotope neutron count, modifying decay and binding rates. |
| `mass` | `u64` | The atom's economic value/balance. |
| `charge` | `f32` | Electronegativity charge, evolving via decay. |
| `last_reaction_hash` | `[u8; 32]` | Anti-replay anchor from the last confirmed reaction. |
| `last_active_index` | `u64` | Crystal index of last participation (used for entropy decay). |
| `nonce` | `u64` | Sequential nonce incremented after each reaction. |
| `quantum_state` | `QuantumState` | Logic state of the atom (Phase 3 / Intellect milestone). |

### `SignedReaction`
| Field | Type | Description |
| :--- | :--- | :--- |
| `sender` | `Atom` | State snapshot of the sender at signing time. |
| `receiver` | `Atom` | Current or zero-mass snapshot of the receiver. |
| `reaction_hash` | `[u8; 32]` | Canonical on-chain identifier. |
| `energy` | `f32` | Network fee burned by the protocol. |
| `timestamp` | `u64` | Construction time (Unix seconds). |
| `signature` | `Vec<u8>` | ML-DSA-87 signature over `signing_digest` (Length: 4627). |
| `payload` | `Payload` | The economic action being requested (e.g., Transfer). `#[serde(default)]` — older nodes that serialized before Payload was introduced deserialize as `Payload::Generic`. |

### `Payload` (Enum)
*   `Generic`: No-op reaction for testing or genesis injection.
*   `Transfer`: Move mass between atoms. Fields: `amount: u64`.
*   `MiningReward`: Block reward self-authenticating via `reaction_hash`. Fields: `amount: u64`.
*   `Unknown`: Catch-all for forward compatibility.

### `MerkleProof` (v2 — compact siblings)

Wire format version: `MPT_PROOF_VERSION = 2`

| Field | Type | Description |
|---|---|---|
| `trie_key` | `[u8; 32]` | SHA3-256(public_key) |
| `value` | `Option<Vec<u8>>` | bincode(Atom) or None |
| `root` | `[u8; 32]` | Trie root this proof is against |
| `siblings` | `Vec<[u8; 32]>` | Combined sibling hashes at each Branch |
| `path` | `Vec<PathStep>` | Node types + nibbles traversed root→leaf |

**Size:** O(depth × 32) ≈ 2 KB vs 25 KB in v1.
**v1 `nodes: Vec<Vec<u8>>`** is removed. Wire format is not backward compatible.

### Proof age limits
MPT roots are retained for `UNDO_WINDOW = 8` blocks. Proof requests for older
blocks return `StorageError::ProofTooOld`. Clients (SDK, CLI) must handle this
error and inform the user to request a more recent proof.


## Network Primitives

### `PrimusMessage` (primus-net-opt)
The P2P wire protocol enum is defined in `primus-net-opt::network`.
`primus-types` provides the payload types it carries (`SignedReaction`,
`GalacticStatus`, `SyncMessage`) but does not own the message envelope.
Refer to the `primus-net-opt` specification for the full message schema.

### `PrimusNR` (Node Record)
The `PrimusNR` serves as the decentralized identity and routing record for a peer.
*   **Role**: Provides routing information (`addr_ip`, `addr_port`) combined with the peer's static `public_key`.
*   **Self-Signed**: Contains an ML-DSA-87 `signature` over its own fields to prevent forgery.
*   **Peer Identification**: The NodeID is derived via `SHA3-256(public_key)`, creating a stable Kademlia DHT identifier regardless of IP changes.

### `NoiseHandshakePayload`
*   Contains the peer's `PrimusNR` and an `ephemeral_sig` binding their static ML-DSA-87 identity to the Noise ephemeral session key to prevent identity misbinding.

### `IpcRequest` / `IpcResponse`
Local inter-process communication types used by `primus-cli` to
interact with a running node over a Unix socket or named pipe.

`IpcRequest` variants:
- `Status` — request node status
- `GetChallenge` — request a 32-byte nonce for admin authentication
- `AdminShutdown { signature }` — graceful shutdown (signed)
- `AdminConnectPeer { addr, signature }` — connect to a specific peer
- `GetProof { address }` — request a Merkle proof for an atom

`IpcResponse` variants:
- `Ok` — generic success
- `Error(String)` — failure reason
- `Challenge(Vec<u8>)` — 32-byte nonce
- `StatusReport { height, peers, cache_size, frame_drops }` — node health telemetry
- `ProofResponse(MerkleProof)` — Merkle proof of atom state

Both types derive `serde`, `rkyv`, `Clone`, and `Debug`.
They are not part of the P2P wire protocol.

## Cryptographic Definitions

### ML-DSA-87 Post-Quantum Signatures
The Primus ecosystem exclusively uses ML-DSA-87 for all cryptographic signatures, securing both transaction validation and peer identity.
*   **Public Key**: 2592 bytes (`PK_BYTES`). Stored directly in the `Atom` and `PrimusNR`.
*   **Signature**: 4627 bytes (`SIG_BYTES`). Produced over a canonical `signing_digest`.
*   **Serialization**: Raw byte arrays without lengths prefixed internally (though vectors use bincode length prefixes on the wire).

### `signing_digest` vs `reaction_hash`
*   `signing_digest()` constructs the exact SHA3-256 hash that the ML-DSA-87 private key must sign.
*   `compute_reaction_hash()` derives the on-chain transaction identifier.
Currently, these derive from the same inputs, but they are semantically separate to allow future divergence.

## Serialization & Constraints

### Derived Traits
Core types (`Atom`, `SignedReaction`, `PrimusNR`, `Payload`) strictly derive:
*   `serde::Serialize`, `serde::Deserialize` (bincode wire format).
*   `rkyv::Archive`, `rkyv::Serialize`, `rkyv::Deserialize` (zero-copy memory).
*   `Clone`, `Debug`, `PartialEq`.

### `no_std` Compatibility
`primus-types` is built with `#![cfg_attr(not(feature = "std"), no_std)]`.
*   Uses `alloc::vec::Vec` and `alloc::string::String` instead of standard collections.
*   Network types like `SocketAddr` in `peer.rs` are strictly gated behind `#[cfg(feature = "std")]` to allow core logic to compile in `no_std` environments (e.g., WASM light clients).

### PhysicsCanon — Cross-Architecture Determinism
Any `f32` value that participates in a hash (state root, signing
digest, fee comparison) MUST be encoded via `PhysicsCanon::encode()`
before hashing. The encoder multiplies by `FIXED_POINT_SCALE = 10⁹`
and converts to `u64`, collapsing the 1-ULP divergence between x86
and ARM FPUs.

Feeding raw `f32::to_bits()` into any hasher is a consensus bug.
Code review must treat it as a correctness violation equivalent to
an off-by-one in a balance check.

Affected fields: `energy` in `SignedReaction`, `charge` in `Atom`,
`current_entropy` and `cumulative_energy` in `GalacticStatus`.

### `ArchivedSignedReaction` — Zero-Copy Hot Path
`reaction.rs` exposes `ArchivedSignedReaction` with two methods
mirroring the owned type:
- `signing_digest()` — zero-copy signing message computation
- `validate_structure()` — structural validation without allocation

Use `SignedReaction::from_bytes_zero_copy(bytes)` on the mempool
ingress path. Use `bincode::deserialize::<SignedReaction>()` for
storage I/O and IPC. Never substitute one for the other.

### Memory Profile & Stack Requirements
*   **Large Footprints**: `Atom` contains a 2592-byte public key. `SignedReaction` contains two Atoms (5184 bytes) plus a 4627-byte signature. A single `SignedReaction` structure is nearly **10 KiB** in memory.
*   **Stack Mitigation**: Public keys and signatures are stored as `Vec<u8>` (heap-allocated) rather than fixed arrays, so `SignedReaction` itself does not cause stack pressure in primus-types. ML-DSA signing and verification operations — performed in primus-core — have separate stack requirements documented there.
*   **Zero-Copy Routing**: rkyv allows the PVM to access `ArchivedSignedReaction` fields zero-copy on the mempool hot path. The GravityShield uses `validate_structure()` which is rkyv-backed internally, but only after an initial `bincode::deserialize` step.

## System Constants

Key constants defined natively in `constants.rs` (others are localized in `primus-core`):
*   `PK_BYTES`: 2592
*   `SIG_BYTES`: 4627
*   `REACTION_HASH_BYTES`: 32
*   `PROTOCOL_MIN_FEE`: 10 (Minimum energy burned by the protocol).
*   `MINING_REWARD_AMOUNT`: 10 (Mass credited to the Architect per Crystal).

## Architectural Invariants

1.  **NO CRYPTOGRAPHIC LOGIC**: With the transitional exception of `PrimusNR::verify`, this crate must not execute ML-DSA routines. It provides the structures; `primus-core` provides the engine.
2.  **BINCODE WIRE FORMAT FROZEN**: Bincode serializes structs by field declaration order and enums by index. Fields must NEVER be reordered, renamed, or removed. New fields must be appended with `#[serde(default)]`.
3.  **SEND + SYNC + CLONE**: All public types must satisfy these traits to support Rayon parallel processing and async Tokio sharing.
4.  **SEPARATION OF FORMATS**: `bincode` is EXCLUSIVELY for P2P network transmission and disk storage. `rkyv` is EXCLUSIVELY for hot-path zero-copy memory access within the node. They are not interchangeable.

<!-- Last sync: 2026-05-02 | fixes: S1,S2,S3,S4,S5,S6,S7 -->
