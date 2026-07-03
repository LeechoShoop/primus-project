# Technical Specification: primus-core

This document defines the internal mechanics of the Obsidian Nexus state machine and consensus engine. It serves as the definitive reference for protocol-level invariants and architectural constraints within the `primus-core` crate.

## 1. The Kinetic Engine & Consensus

The consensus mechanism, known as the **Kinetic Engine**, replaces traditional probabilistic mining with a physics-inspired deterministic synthesis model.

### Galactic Drift Logic
The network state is partitioned into 256 sectors (0-255). The **Galactic Drift** is a deterministic parameter derived from the current crystal index:
`drift = crystal_index % 256`
This parameter dictates which sector of the mempool is prioritized for the current block synthesis, ensuring balanced processing across the global atom space.

### Resonance Mechanism
Reactions are selected for inclusion via the **Resonance** mechanism:
1.  **Selection**: The `drain_resonant` function extracts high-energy reactions from the sector identified by the current Galactic Drift.
2.  **Injection**: Selected reactions are injected into the `ReactionChamber`.
3.  **Synthesis**: The `synthesize_with_gravity` function uses a `GravityEngine` seeded with the previous block's hash and index. It performs a deterministic "roll" for each reaction; those failing the stability threshold (based on chamber entropy and surface tension) are "evaporated" and returned to the mempool.

### Crystal Synthesis Lifecycle
1.  **Preparation**: Derive deterministic timestamp (Parent TS + target block time).
2.  **Filtering**: Perform a PVM pre-filter to drop semantically invalid reactions.
3.  **Reward Injection**: Prepend a synthetic `MiningReward` reaction (10 mass to Architect) to the confirmed reaction list.
4.  **PoW Solve**: Iteratively increment the crystal nonce until the `density` satisfies the difficulty target (influenced by chamber temperature and entropy).
5.  **Solidification**: Apply the changeset to the `StateTree` and commit the changeset to the `MerklePatriciaTrie` (MPT) to calculate the canonical state root. (`StateTree::calculate_root_hash` is deprecated).
6.  **Persistence**: Commit all changes to the Sled-backed `PrimusStorage`.

## 2. State Machine & Validation

### Atom Lifecycle
An **Atom** is the fundamental unit of state.
*   **Identity**: Bound to an ML-DSA-87 public key.
*   **Mass**: Represents the balance/energy of the atom.
*   **Nonce**: Incremented only upon confirmed `Transfer` or `Generic` reactions to prevent replay.
*   **Evolution**: Atoms evolve their `Element` and `Decay` state based on activity and crystal height.

### Validation Pipeline
Every incoming reaction must pass the following sequence in the PVM:
1.  **Cryptographic Check**: Mandatory ML-DSA-87 verification of the signature against the `signing_digest()`.
2.  **Sequence Integrity**: Anti-replay check: `on_chain.nonce == rx.sender.nonce`.
3.  **Mass Conservation**: For transfers, `sender.mass >= amount + fee`.
4.  **Thermal Filter**: Cumulative heat of all reactions in a crystal must not exceed the `thermal_capacity` (1000.0).

> **Implementation note**: The PVM enforces an internal threshold of 1000.0.
> `primus-core` normalizes temperatures via `physics_shim::to_vm_thermal()`
> before passing to PVM (thermal capacity path) and `physics_shim::to_vm_gate()`
> for Gravity Shield gating. Denormalization uses `physics_shim::from_vm_thermal()`.
> The alias `to_vm_temperature()` is deprecated. See `primus-core/src/physics_shim.rs`.

## 3. System Stability & Security (Windows Focus)

### The 16 MiB Mandate
Due to the memory-intensive nature of ML-DSA-87 (specifically matrix expansions and verification), all cryptographic tasks on Windows MUST follow this pattern:
*   **Isolation**: Offload to `tokio::task::spawn_blocking`.
*   **Stack Allocation**: Explicitly spawn an `std::thread` with a **16 MiB stack size**.
*   **Safety**: Failure to follow this pattern results in a `STATUS_STACK_OVERFLOW` (0xc00000fd) crash.

**Linux**: The same 16 MiB stack pattern MUST be applied on Linux.
While Linux default stacks (8 MiB) do not crash as deterministically
as Windows, ML-DSA-87 matrix expansion will cause silent stack corruption
under concurrent load without explicit stack sizing.
Implementation: `CoreCryptoVerifier` in `primus-core/src/crypto_shim.rs`
enforces this on both platforms via `std::thread::Builder::stack_size`.

### Gravity Shield Feedback
The engine provides a thermal feedback loop to the `primus-net-opt` layer:
*   **Chamber Temperature**: Real-time metric tracked in `ReactionChamber`.
*   **Thermal Gate**: If `temperature >= 150.0 K`, the node signals the **Gravity Shield** to drop all incoming non-architect reactions at the network entry point to prevent node meltdown and resource exhaustion.

### Security: Memory Hardening
To protect against memory forensics and cold-boot attacks, the system enforces strict memory sanitization:
*   **Zeroization**: All sensitive cryptographic material (BIP-39 mnemonics, ML-DSA-87 seeds, and signing keys) MUST implement the `ZeroizeOnDrop` trait.
*   **Wiping Policy**: Plaintext secrets are forbidden from persisting in the heap beyond the scope of a single cryptographic operation. All temporary buffers and copies created during key expansion or signing MUST be zeroed out immediately after use.
*   **Cli/SDK standard**: The `primus-sdk` and `primus-cli` utilize the `zeroize` crate to ensure that every `Wallet` and `Keypair` instance is wiped upon destruction.

## 4. Network Ingress & Bridge Architecture

The `primus-core` crate serves as the orchestration hub. The Network Layer has been fully extracted to `primus-net-opt`. The core engine only communicates with the network via the decoupled `CoreHandle` trait and IPC `AdminConnectPeer` messages.

### The Core Seam (CoreHandleImpl)
To prevent circular dependencies between consensus and networking, `primus-core` implements the `CoreHandle` and `MempoolIngress` traits defined in `primus-net-opt`.
*   **Inversion of Control**: The network layer only knows about the trait interfaces. `CoreHandleImpl` provides the concrete implementation that interacts with the `PrimusEngine`, `SectoralMempool`, and `PrimusStorage`.
*   **Back-references**: `CoreHandleImpl` maintains a reference to the active `PrimusNetwork` instance to enable automated gossip re-broadcasts of newly validated reactions.

### Ingress Pipeline (Gravity Shield + rkyv)
Every incoming message from the network (TCP or QUIC) follows a strict high-assurance validation pipeline:
1.  **Framing (Bounded Transport)**: The `LengthDelimitedCodec` enforces a strict **16 MiB frame limit**. Any violation results in immediate connection termination and is logged as a security event.
2.  **Structural Filter (Gravity Shield)**: Raw bytes are passed to the `GravityShield`.
    *   **Layer 1-3**: Cheap structural and size checks performed in `primus-net-opt`.
    *   **Layer 4**: Thermal gating. If `chamber.temperature >= 150.0 K`, the reaction is rejected.
    *   **Layer 5**: Phantom Sender Check. If the sender is not the Architect and has no balance on-chain, the reaction is dropped.

> **Implementation note**: Layer 5 (Phantom Sender Check) is enforced by
> `CoreHandleImpl` in `primus-core/src/bridge.rs`. primus-net-opt (qualified)
> does not implement this layer. The check is a silent drop to prevent
> amplification attacks via error response probing.

3.  **Zero-Copy Validation**: Validated bytes are checked via `rkyv::check_archive` to prevent OOB access or malicious memory layouts without full deserialization.
4.  **Ingress**: Validated reactions are pushed to the `SectoralMempool`. If the reaction is new, the bridge triggers a re-broadcast to connected peers.

### Node Discovery (KademliaBridge)
The `KademliaBridge` synchronizes the `PrimusDHT` with incoming network RPCs:
*   **RPC Handling**: Incoming `FindNodeRequest` messages are processed by searching the local `PrimusDHT` for the 20 closest neighbors.
*   **Liveness**: The bridge ensures that routing table updates from the network layer are correctly reflected in the engine's view of the topology.

## 5. Administrative IPC & Control

The core exposes a secure IPC interface for local administrative control.
To prevent "Socket Shadowing" attacks and ensure privacy, the IPC interface is bound to user-restricted, secure paths instead of global temp directories.
*   **Unix**: Binds to `[XDG_RUNTIME_DIR]/primus.sock` (fallback `$HOME/.primus/run/primus.sock`) with `0600` permissions.
*   **Windows**: Binds to a Named Pipe embedded with the user's SID (e.g., `\\.\pipe\primus-nexus-[USER_SID]`) to utilize OS-level access control.

### Architect Authorization
Commands affecting node state are protected by a **Challenge-Response** protocol:
1.  The requester asks for a challenge.
2.  The node provides a 32-byte random nonce.
3.  The requester signs the nonce with the hardcoded **Architect Public Key**.
4.  The node verifies the signature using the **16 MiB Mandate** before executing the command.

### Command Set
*   **Status**: Retrieves a `StatusReport` containing:
    *   `height`: Current crystal index.
    *   `peers`: Active peer count in the Kademlia DHT.
    *   `cache_size`: Number of active persistent TCP connections.
    *   `frame_drops`: Total frames dropped. Shared Arc<AtomicU64> between
        GravityShield, PrimusNetwork, and IpcServer. Ordering::Relaxed.

*   **AdminConnectPeer**: Directs the QUIC server to establish a connection with a remote peer.
*   **AdminShutdown**: Gracefully terminates the mining loop and flushes the database.

## 6. Resource Management

### Sectoral Mempool
*   **Capacity**: Maximum 1,000,000 reactions per sector.
*   **Eviction**: Priority is given to high-energy (fee) reactions; older, low-energy reactions are evicted when capacity is reached.

### Persistence & Data Integrity
*   **Atomic Changesets**: State updates are applied as atomic changesets.
*   **Reorg Atomicity Limitation**: Full chain reorganizations (multi-block
    rollbacks) are NOT wrapped in a single sled atomic transaction. Each
    block's rollback is applied sequentially via `UndoLog`. In the event of
    a crash mid-reorg, the node will restart in a partially-rolled-back state.
    Recovery: the node detects the inconsistency on startup via state root
    mismatch and replays available `UndoLog` entries to reach a consistent
    state. Nodes that cannot recover automatically require manual intervention.
    **Future work**: Wrap the full reorg sequence in `sled::Db::transaction`
    when sled's multi-tree transaction API stabilizes.
*   **Flushing**: The `mine_block` function invokes a database `flush()` after every solidification to ensure that the state is recoverable in the event of an ungraceful shutdown.
*   **Storage**: Utilizes Sled for high-performance, embedded key-value storage of Atoms and Crystals.

## 7. Networking & Transport (Hardening)

Obsidian Nexus uses a hybrid networking model combining legacy TCP (for Galactic Sync) and high-performance QUIC/WebTransport (for gossip and RPC).

### Strict Bounded Transport
To protect against resource exhaustion and OOM attacks, the networking stack enforces the following constraints:
- **Frame Limits**: The Noise Protocol transport layer enforces a hard
  maximum of 65535 bytes per Noise message (Noise Protocol spec §3 constraint).
  At the application layer, logical messages MUST NOT exceed **16 MiB**.
  `primus-core/src/framing.rs` handles chunking/reassembly (conclusion A)
  or outbound payload guarding (conclusion B) — see Prompt 1 memo in AUDIT_REPORT.md.
  Outbound payloads exceeding the effective limit are rejected with
  `FrameError::MessageTooLarge` before serialization.
- **Concurrency Semaphores**: Each active peer connection is bounded by a semaphore allowing a maximum of 100 concurrent streams/tasks.
- **Zero-Copy Ingress**: Ingress utilizes `rkyv` zero-copy validation to minimize CPU and memory pressure.

### DHT Resilience
The Kademlia implementation in `primus-net-opt` includes **"Ping-the-Tail"** logic:
- When a k-bucket is full, the oldest node (tail) is pinged before being replaced.
- If the tail responds, the new candidate is dropped.
- If the tail fails to respond, it is evicted and replaced, ensuring routing table liveness and protection against Eclipse attacks.

<!-- Last sync: 2026-06-04 | fixes: frame-shim, rkyv-boundary, ml-dsa-threading,
     physics-shim (to_vm_thermal/to_vm_gate), gravity-shield-l5,
     sdk-frame-constant, sdk-noise-xx, cli-remote-address,
     PrimusNetwork::new 4-arg, pvm.rs dead-code, INTDIV-001 async RwLock,
     size-guard-G4, reorg-atomicity-note -->

