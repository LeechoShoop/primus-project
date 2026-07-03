# Technical Specification: primus-net-opt

This document details the internal mechanisms and protocol specifications of the `primus-net-opt` transport layer.

## 1. Handshake Flow: Noise_XX with ML-DSA-87 Binding

The Obsidian Nexus network uses a customized Noise_XX handshake pattern (`Noise_XX_25519_ChaChaPoly_SHA256`) to establish secure sessions. Unlike standard Noise, we add a mandatory **Identity Binding** step to prevent identity misbinding and MITM attacks.

### Step-by-Step Exchange:
1.  **Initiator ephemeral (`-> e`)**: The initiator sends its X25519 ephemeral public key.
2.  **Responder identity (`<- e, ee, s, es + Payload`)**:
    *   The responder sends its ephemeral key and static key.
    *   **Payload**: Contains the responder's `PrimusNR` (Node Record) and an `ephemeral_sig`.
    *   **Binding**: `ephemeral_sig` is an ML-DSA-87 signature by the responder's static identity key over the initiator's ephemeral key (`e`).
3.  **Initiator identity (`-> s, se + Payload`)**:
    *   The initiator sends its static key.
    *   **Payload**: Contains the initiator's `PrimusNR` and an `ephemeral_sig`.
    *   **Binding**: `ephemeral_sig` is an ML-DSA-87 signature by the initiator's static identity key over the responder's ephemeral key (`e_r`).

**Verification Requirement**: Both parties MUST verify the ML-DSA-87 signature against the provided `PrimusNR` public key before accepting the session.

## 2. Message Lifecycle & Gravity Shield

Every buffer received from the wire must pass through the **Gravity Shield** before it is processed by the execution engine.

### Filtering Pipeline:
1.  **Framing**: 
    - TCP frames: up to 16 MiB (`LengthDelimitedCodec`, network.rs)
    - QUIC gossip uni-streams: up to 16 MiB (server.rs)
2.  **GravityShield (Pre-deserialization)**:
    - Layer 1: `bincode::deserialize` — rejects non-Bincode frames immediately
    - Layer 2: `rx.validate_structure()` — structural field-range check (rkyv-backed internally)
    - Layer 3: sanity checks — non-empty public key, non-negative energy (`rx.energy >= 0.0`)

Note on known issue: `PeerSession::send_gossip` previously sent plaintext over QUIC uni-streams. Outbound Noise encryption is now applied, resolving BUG 1 in the audit log.

3.  **Core Filtering (`CoreHandle::shield_filter`)**:
    *   **Thermal Gate**: If the local Chamber Temperature is **$\ge 150.0$ K**, all non-architect reactions are dropped to prevent node meltdown.
    *   **Sender Validation**: Reactions from "phantom senders" (keys not found on-chain) are rejected.
4.  **Mempool Ingestion**: Only after passing all checks is the `SignedReaction` ingested into the sectoral mempool.

## 3. NAT & Connectivity

### NatService (UPnP)
The node uses `igd_next` for asynchronous UPnP discovery. On startup, `NatService::open_world` performs:
1.  **Gateway Discovery**: Searches for a UPnP-capable gateway.
2.  **Port Mapping**: Maps the configured P2P port (default `P2P_PORT = 9000`, declared in server.rs) for both **TCP** and **UDP**.
3.  **External IP Resolution**: Retrieves the public IP address from the gateway.

### Network Record (NR) Persistence
The resolved external IP and port are injected into the node's `PrimusNR`. This signed record is broadcast via the DHT, allowing remote peers to route packets to the node across the internet.

## 4. Error Handling

*   **Handshake Failures**: Any failure during the Noise exchange (signature mismatch, timeout, malformed payload) results in immediate connection termination.
*   **NAT Timeouts**: If UPnP discovery fails, the node logs a warning and proceeds with the local IP, potentially limiting inbound connectivity unless manual port forwarding is used.
*   **Shield Rejections**: Dropped packets are logged at the `warn` level with the specific rejection reason (e.g., `Chamber Overheat` or `Phantom Sender`).

## 5. Safety Guards: Windows Stability

To prevent `STATUS_STACK_OVERFLOW` crashes on Windows platforms:
*   **Stack Allocation**: ML-DSA key material (`ml_dsa_sk`) is heap-allocated via `Arc<Box<[u8]>>` to avoid placing 7+ KB on the stack (FIX 1).
*   **Heap Futures**: The iterative Kademlia lookup uses `Box::pin` futures to move state to the heap instead of deep async recursion (FIX 2).

## 6. Gossip Protocol

Epidemic Broadcast (GossipService)
TTL behavior per message type:
| Message | TTL behavior |
|---|---|
| NewReaction(data, ttl) | Decrements by 1 per hop; dropped when TTL = 0 |
| NewCrystal(data, ttl) | No decay; propagated unchanged |
| Sync(_) | Treated as TTL = 1, always decays |

Deduplication:
- Message ID = SHA3-256 of `bincode::serialize(message)`
- `seen_messages`: `HashSet<[u8; 32]>` per node
- Capacity cap: `MAX_SEEN = 10_000`
- Eviction: oldest `EVICT_COUNT = 1_000` entries removed when cap is exceeded

Local processing (before propagation):
- `NewReaction` → `CoreHandle::shield_filter` → `CoreHandle::on_reaction` (spawned)
- `NewCrystal`  → `CoreHandle::on_crystal` (spawned)
- Local processing does not block propagation

Source exclusion: messages are never echoed back to the originating peer.

## 7. CoreHandle — Dependency Inversion

`primus-net-opt` defines the `CoreHandle` trait. `primus-core` implements it.
`PrimusNetwork<H: CoreHandle>` is generic over the implementation, giving
`primus-net-opt` zero compile-time dependency on `primus-core` internals.

| Method | Purpose |
|---|---|
| `on_reaction(rx)` | Ingest a validated SignedReaction into the mempool |
| `on_crystal(bytes)` | Validate PoW, solidify block or trigger chain reorg |
| `get_crystal_bytes(index)` | Fetch serialized crystal for sync responses |
| `local_state()` | Return `(chain_height, entropy, cumulative_energy)` |
| `is_syncing()` | True if node is catching up to a sync target |
| `set_sync_target(height)` | Set the target height for a sync session |
| `finish_sync()` | Mark the current sync session as complete |
| `shield_filter(raw)` | Full semantic validation: signature, nonce, mass balance |
| `mempool_push(rx)` | Push a pre-validated reaction to the sectoral mempool |
| `get_atom_state(addr)` | Query atom mass, nonce, last_hash, element by pubkey |
| `push_bytes(bytes)` | Ingest raw reaction bytes — GravityShield + PVM pipeline |
| `on_get_proof(addr)` | Retrieve Merkle balance proof for addr |

## 8. RPC over TCP

SDK and CLI clients connect to the TCP listener and use the same
`PrimusMessage` envelope with `bincode` + `LengthDelimitedCodec` framing.

| Request | Response | Timeout | Purpose |
|---------|----------|---------|---------|
| `SubmitReaction { reaction_bytes }` | `ReactionAck { reaction_hash }` | 10 s | Submit signed reaction |
| `FetchState { address }` | `StateResponse { mass, nonce, last_hash, element }` | 5 s | Query atom state |
| `GetProof { address }` | `ProofResponse(MerkleProof)` | 10 s | Merkle balance proof |
| any | `NodeError { reason }` | — | Error for any failure |

**Security**: `SubmitReaction` bytes pass through `GravityShield::filter_bytes()`
before `CoreHandle::push_bytes()`. Same invariant as gossip `NewReaction` path.

**frame_drops**: GravityShield rejections on the RPC path increment the shared
`PrimusNetwork.frame_drops` counter.

## 9. Telemetry & Monitoring

The network layer tracks operational health through atomic counters:
- **`frame_drops`**: Single `Arc<AtomicU64>` shared between `PrimusServer`,
  `PrimusNetwork`, and `IpcServer`. Created once in `primus-core` and passed
  to all three via constructor injection (`PrimusNetwork::new` fourth argument).
  Incremented by: GravityShield failures, QUIC framing violations,
  RPC `SubmitReaction` GravityShield rejections.
  `Ordering::Relaxed` — exact counts not required.

<!-- Last sync: 2026-05-22 | changes: RPC-over-TCP section, CoreHandle new methods,
     frame_drops shared Arc, GravityShield on SubmitReaction, timeouts on RPC handlers -->
