# Technical Specification: primus-sdk

This document defines the public API surface and technical implementation details of the `primus-sdk`. The SDK abstracts the complexities of the Obsidian Nexus protocol, providing developers with stable primitives for wallet management, transaction construction, and network interaction.

## 1. Client Lifecycle

### Initialization: NodeClient
The SDK interacts with the Obsidian Nexus network via a `NodeClient` (or equivalent transport wrapper). The client is designed to be stateless, opening scoped connections for specific queries or submissions.

```rust
use primus_cli::client::NodeClient;

// Initialize a client targeting a local or remote node
let client = NodeClient::new("127.0.0.1", 9000, false);
```

### Connection Handling
While the core nodes utilize a permanent QUIC-based P2P mesh, the SDK/CLI client currently employs a **Length-Prefixed TCP/Bincode** transport for administrative and user queries.
*   **Framing**: `[4-byte big-endian length] ++ [bincode-serialized message]`.
*   **Handshake**: For peer-to-peer interactions (e.g., in `primus-net-opt`), a mandatory **Noise_XX** handshake with ML-DSA-87 identity binding is required. In the current SDK-to-Node query model, this is abstracted into direct framed requests.
*   **Timeout**: The client enforces a strict **5-second timeout** on all network I/O to ensure responsiveness.

## 2. Reaction Workflow

### Reaction Construction
Reactions are built using the `TransactionBuilder`, which ensures protocol compatibility and performs pre-flight sanity checks.

```rust
use primus_sdk::{Wallet, TransactionBuilder, PROTOCOL_MIN_FEE};

let wallet = Wallet::load(Path::new("my.wallet"))?;
let tx = TransactionBuilder::new(&wallet)
    .recipient(recipient_pk_bytes)
    .amount(1_000)
    .sender_mass(current_mass)
    .sender_last_hash(current_last_hash)
    .sender_nonce(current_nonce)
    .fee(PROTOCOL_MIN_FEE)
    .build()?;
```

### Signing: ML-DSA-87
The SDK handles **ML-DSA-87** signing through the `Wallet::sign` method.
*   **Internal Logic**: The signing key is re-derived from the 32-byte `key_seed` on every call, ensuring that the full 4896-byte secret key never persists in memory longer than necessary.
*   **Stack Management**: Due to the ~4 MiB stack requirement for ML-DSA-87 operations, callers on stack-constrained platforms (like Windows) must execute signing within a high-stack environment. The recommended pattern is using a dedicated thread or `tokio::task::spawn_blocking` with a **16 MiB stack size**.

### Broadcasting
Once a `Transaction` is built and signed, it must be serialized and wrapped for the network.

```rust
let tx_bytes = tx.to_bytes()?;
// Wrap in PrimusMessage::NewReaction(tx_bytes, ttl) and send via client
let ack = client.broadcast_tx(tx_bytes).await?;
```

## 3. State Sync & Queries

### Atom Tracking
The SDK provides methods to query the on-chain state of an `Atom` (identity, balance, nonce).
*   **Method**: `NodeClient::get_atom_state(address: &str)`
*   **Data Returned**: `mass` (balance), `nonce`, `last_hash` (for anti-replay), and `element`.

### Crystal Monitoring
Monitoring for new Crystals (blocks) is performed via the `SyncMessage` protocol defined in `primus-types`.
*   **Polling**: Clients can poll for the latest `GalacticStatus` via the `SyncMessage::Handshake` variant.
*   **Subscription**: In high-performance implementations (like `primus-net-opt`), nodes receive `PrimusMessage::NewCrystal` broadcasts. SDK consumers typically poll the `get_crystal_bytes(index)` endpoint to verify solidification.

## 4. Merkle Proof Verification (Light Client)

The SDK provides high-assurance, dependency-free Merkle proof verification for light clients and WASM environments.

### `verify_balance_proof`
This function validates that a specific atom's state (mass, nonce, element) is correctly included in a trusted state root.
*   **WASM-Safe**: Implementation is pure logic with zero I/O, zero Sled/storage dependencies, and no `std::net` usage.
*   **Logic**: Reconstructs the Merkle path from siblings and hashes using `blake3`. Supports both **Inclusion Proofs** (atom exists) and **Exclusion Proofs** (atom does not exist).
*   **API**:
  ```rust
  let is_valid = primus_sdk::verify_balance_proof(&proof, &trusted_root)?;
  ```

### Key Security Property
The SDK ensures that the state root trusted by the caller (e.g., from a block header with verified PoW) is the only source of truth. The node cannot lie about the state without breaking the cryptographic hash chain.

## 5. Security Measures

### Shield Awareness (Back-pressure)
The SDK is designed to handle rejections from the node's **GravityShield**.
*   **NodeError**: If the node's `Chamber` temperature exceeds **150.0 K**, it will return a `NodeError { reason: "Shield: Chamber Overheat..." }`.
*   **Phantom Sender**: Rejections also occur if the sender's public key is not yet matured or registered on-chain.

### Identity Protection
*   **Memory Safety**: The `Wallet` struct hides the `mnemonic_phrase` and `key_seed` fields. Access is strictly gated via getters.
*   **Seed Storage**: The SDK stores only the 32-byte child seed and derivation index. The raw signing key is ephemeral and re-derived only during the `sign()` call.
*   **Persistence**: `Wallet::save()` writes an encrypted/encoded file containing only the mnemonic and index, requiring a full re-derivation on `load()`.

## 5. Error System

The SDK utilizes a structured error system (via `anyhow`) with the following logical categories:

| Error Type | Description |
| :--- | :--- |
| `HandshakeError` | Failure to establish a Noise_XX session or protocol version mismatch. |
| `SignatureError` | ML-DSA-87 verification failure or malformed signature bytes. |
| `TransportError` | Connection refused, timeout, or frame size violation (max 32 MiB). |
| `ReactionRejected` | Specific rejection from the node (e.g., `InsufficientMass`, `Sequence Mismatch`, or `Chamber Overheat`). |

## 6. Interaction Map

*   **`primus-sdk` ↔ `primus-types`**: The SDK uses the core types (`Atom`, `Reaction`, `Payload`) to ensure wire compatibility.
*   **`primus-sdk` ↔ `primus-net-opt`**: The SDK relies on the `Noise_XX` and `Bincode` framing specifications defined in the transport layer for all node interactions.
*   **`primus-sdk` ↔ `primus-core`**: The SDK constructs transactions that are semantically identical to those validated by the Core's PVM and GravityShield.
