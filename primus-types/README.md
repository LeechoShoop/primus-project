# primus-types — The Cryptographic Backbone of Obsidian Nexus

## Mission
Obsidian Nexus represents a paradigm shift from traditional HTTP-based Web 2.0 architectures to a post-quantum, high-performance Layer 1 ecosystem. The `primus-types` crate serves as the foundational data definitions and serialization contract for this entire network. By decoupling the core primitives from the execution engine (`primus-core`) and network stack (`primus-net-opt`), this library guarantees absolute architectural rigidity and deterministic cross-platform agreement across the Obsidian Nexus ecosystem. 

Our mission is to establish a network topology that is immune to legacy web vulnerabilities and resistant to emerging cryptographic threats, providing a stateless, scalable, and verifiable fabric for global digital interaction.

## Key Technical Innovations

### Post-Quantum Resilience
The Obsidian Nexus network natively integrates the **ML-DSA-87** parameter set as its exclusive digital signature algorithm. Standard elliptic curves (e.g., secp256k1) have been deprecated entirely. Every identity, transaction, and peer record defined in `primus-types` is anchored by a 2592-byte ML-DSA public key and authenticated via a 4627-byte signature. This forward-thinking architecture ensures that the network remains completely secure against theoretical and practical quantum adversaries.

### Kinetic State Model
State management abandons traditional account-based or UTXO models in favor of a Kinetic State Model:
*   **Atoms**: The canonical on-chain identity. An Atom encapsulates balance (mass), classification (element), energy (charge), and Phase-3 logic (quantum state). 
*   **Crystals**: Network-wide state snapshots and blocks. Crystals synthesize the energy and reactions of the network, finalizing the state transition and advancing the network's thermodynamic weight.

### No-HTTP Protocol Schema
The P2P wire protocol (`PrimusMessage`, QUIC transport, Noise_XX handshake) is defined in `primus-net-opt`. `primus-types` provides only the payload structures those messages carry.

## Data Structure Overview

### Reactions
The `SignedReaction` is the fundamental unit of change in the network. A Reaction represents a mathematically verifiable intent to execute an economic action (e.g., a mass transfer). It encapsulates the exact state snapshots of the interacting Atoms at the time of construction, an exact network fee (`energy`), and a deterministic payload. 

### Network Records (NR)
The `PrimusNR` (Node Record) acts as the decentralized routing passport for a peer. It binds a node's static ML-DSA-87 identity to its dynamic IP/Port routing information. To support global reach, it natively integrates with UPnP/NAT-T, capturing the external public address to facilitate unhindered peer discovery and reliable remote connectivity.

### Gravity Shield Invariants
To preserve node stability during high-throughput stress or active DDoS campaigns, structures are built for zero-copy memory access via `rkyv`. rkyv enables zero-copy field access on the hot path inside primus-core (mempool scanning, PVM state tree). The GravityShield layer first deserializes via bincode, then calls `validate_structure()` which performs rkyv-backed field-range checks without running ML-DSA cryptography. Reactions are dynamically filtered against critical metadata—such as strict Chamber temperature limits and phantom sender bounds—ensuring that malformed or malicious packets are rejected before they reach the execution engine.

### PhysicsCanon
`PhysicsCanon` is the canonical encoder for `f32` physics values (temperature, entropy, charge) that must participate in deterministic hashing across heterogeneous hardware. Before any `f32` value enters a SHA3-256 hash, it is multiplied by `FIXED_POINT_SCALE` (10⁹) and converted to `u64`. This collapses the 1-ULP difference that x86 80-bit extended-precision FPUs can produce compared to ARM strict 32-bit results, guaranteeing identical state roots across all nodes.

### IPC Protocol
`IpcRequest` and `IpcResponse` define the local inter-process communication protocol between `primus-cli` and a running node. Both types derive `serde`, `rkyv`, `Clone`, and `Debug` and satisfy the `Send + Sync` invariant. They are not part of the P2P wire protocol and are never sent over the network.

## Developer Experience (DX)

### Safety Guards
The crate is engineered with strict adherence to memory safety and protocol immutability. 
*   The `bincode` wire format is permanently frozen; any modification to field ordering is prevented by design.
*   Data types are rigorously bound to `Send + Sync + Clone` to guarantee thread safety across multi-threaded asynchronous workers.
*   Error handling is localized, deterministic, and easily consumable.

## Stack Safety
ML-DSA-87 public keys (2592 bytes) and signatures (4627 bytes) are stored as heap-allocated `Vec<u8>` rather than fixed stack arrays. A single `SignedReaction` containing two `Atom`s is approximately 10 KiB; heap allocation via `Vec` ensures this never lives on the stack in primus-types itself. Callers that perform ML-DSA-87 signing or verification should consult primus-core for stack requirements of those operations.

## Build & Usage

To build the `primus-types` crate, ensure you are running the latest stable Rust toolchain. The crate relies on modern features, including `no_std` compatibility for core logic and zero-copy macro generation.

```bash
# Clone the Obsidian Nexus workspace
git clone <repository_url>
cd primus-project

# Build the primus-types library
cargo build -p primus-types --release

# Run structural invariant tests
cargo test -p primus-types
```

*Note: Any modifications to this crate should be heavily audited to maintain cross-architecture determinism, specifically observing the serialization boundaries between `bincode` (P2P routing/storage) and `rkyv` (in-process hot paths).*

<!-- Last sync: 2026-05-02 | fixes: R1,R2,R3,R4,R5 -->
