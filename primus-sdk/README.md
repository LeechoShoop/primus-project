# Primus SDK: Post-Quantum Ready

The `primus-sdk` is a high-performance, developer-centric gateway to the Obsidian Nexus network. It abstracts the complexities of the Primus protocol, providing stable primitives for wallet management, transaction construction, and network interaction, all while maintaining rigorous post-quantum security and operational safety.

## Key Features

- **ML-DSA-87 Cryptography**: Native support for NIST-standardized post-quantum signatures (Module-Lattice-Based Digital Signature Algorithm).
- **16 MiB Windows Stack Guard**: Automatic isolation of high-memory cryptographic tasks to prevent `STATUS_STACK_OVERFLOW` crashes on Windows.
- **Smart Resilience**: Automatic `SequenceMismatch` retries and native awareness of the node's **Gravity Shield** (Thermal Throttling).
- **Pure Binary Transport**: High-efficiency framing using length-prefixed Bincode, bypassing the overhead of HTTP/JSON.
- **Stateless NodeClient**: A lightweight, robust interface for interacting with individual nodes or the global mesh.
- **Memory Safety (Zeroize)**: Automatic wiping of sensitive secrets from memory after use to protect against cold-boot attacks.

## Installation

Add the following to your `Cargo.toml`:

```toml
[dependencies]
primus-sdk = { path = "../primus-sdk" }
```

## Quick Start

The following example demonstrates how to load a wallet, initialize a client, and broadcast a signed **Reaction** (transaction) to the network.

```rust
use primus_sdk::{Wallet, TransactionBuilder, PROTOCOL_MIN_FEE};
// NodeClient lives in primus-cli, not primus-sdk.
// DIV-003 fix: corrected import path (was primus_cli::client::NodeClient).
// See primus-cli/src/client.rs for the full implementation.
use primus_cli::client::NodeClient;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // 1. Load your wallet (Protected by 16 MiB Stack Guard internally)
    let wallet = Wallet::load(Path::new("master.wallet"))?;

    // 2. Initialize the NodeClient with Noise_XX encryption (BLK-003 fix).
    //    new_with_noise() requires a 32-byte X25519 static key.
    //    Use new_with_ephemeral_noise() for a one-shot ephemeral key.
    let mut client = NodeClient::new_with_ephemeral_noise("127.0.0.1", 9000)?;

    // 3. Fetch the current state of your Atom (identity, balance, nonce)
    let state = client.get_atom_state(&wallet.address).await?;

    // 4. Build a Reaction using the fluent builder
    let builder = TransactionBuilder::new(&wallet)
        .recipient(Wallet::decode_address("recipient_hex_address")?)
        .amount(1_000)
        .fee(PROTOCOL_MIN_FEE)
        .sender_mass(state.mass)
        .sender_last_hash(state.last_hash)
        .sender_nonce(state.nonce);

    // 5. Broadcast with Automatic Retry logic
    // Signing is automatically performed in a high-stack environment.
    // If a Sequence Mismatch occurs, the SDK refreshes state and retries once.
    let response = client.broadcast_tx(&wallet, builder).await?;
    
    println!("Node Response: {}", response);
    Ok(())
}
```

## Connecting to a Remote Node

Use `new_with_noise()` for all connections to live nodes. Plain TCP is rejected
by live primus-core nodes (AUDIT_REPORT.md BLK-003).

```rust
use primus_cli::client::NodeClient;

// Supply a persistent 32-byte X25519 static key (load from secure storage):
let client = NodeClient::new_with_noise("192.168.1.100", 9000, &static_key_bytes)?;

// Or use an ephemeral key for one-shot / diagnostic connections:
let client = NodeClient::new_with_ephemeral_noise("192.168.1.100", 9000)?;
```

The `new()` constructor is **deprecated** and will be removed in a future release.
Use `new_with_noise()` or `new_with_ephemeral_noise()` instead.

## Transport Notes

- **Max frame size: 16 MiB** (matches primus-core SPECIFICATION.md §7).
  Previous SDK versions incorrectly used 32 MiB, causing silent connection
  drops on large payloads against live nodes. Fixed in AUDIT_REPORT.md DIV-001.
- The Noise transport layer imposes a hard 65535-byte per-message limit.
  `primus-core/src/framing.rs` handles chunking of larger application payloads
  transparently before they reach the Noise layer.



## Resilience & Safety

### 16 MiB Stack Guard
ML-DSA-87 operations (signing and key expansion) require ~4 MiB of stack space. To prevent crashes on Windows—where the default stack is often 1 MiB—the SDK wraps all critical cryptographic calls in `ensure_high_stack`. This utility spawns a dedicated 16 MiB thread to ensure stability without requiring the developer to manage thread parameters manually.

### Memory Safety (Zeroize)
The SDK utilizes the `zeroize` crate to implement defense-in-depth against memory forensics. All sensitive data—including BIP-39 mnemonics, ML-DSA seeds, and signing keys—implement the `ZeroizeOnDrop` trait. This ensures that secrets are cryptographically wiped from the heap immediately after a signing operation completes, protecting the user against memory dumps and cold-boot attacks.

### Protected Administrative Channels
To protect against "Socket Shadowing" attacks, the administrative IPC commands (e.g., node shutdown, peer connection) are executed over user-restricted channels. On Unix, sockets are bound to `XDG_RUNTIME_DIR` with strict 0600 permissions, and ownership is actively verified prior to connecting. On Windows, Named Pipes are embedded with the user's SID for OS-level isolation.

### Gravity Shield & Thermal Back-off
The Obsidian Nexus nodes monitor their **Chamber Temperature**. If the temperature exceeds **150.0 K**, the **Gravity Shield** activates to protect the node from resource exhaustion. The SDK detects these signals and returns a specific `ThermalThrottling` error, allowing applications to implement proper back-off strategies.

### Automatic Sequence Retry
In high-concurrency environments, an atom's `nonce` or `last_hash` might change between the state-fetch and the broadcast. The `broadcast_tx` method implements an automatic retry policy: if the node rejects a reaction with a `Sequence Mismatch`, the client automatically refreshes the atom state and resubmits exactly once.

## Error Handling: `PrimusSdkError`

The SDK provides a structured error system to handle various network and protocol conditions:

| Variant | Technical Description |
| :--- | :--- |
| `SequenceMismatch` | The transaction nonce or last_hash does not match the on-chain state. |
| `ThermalThrottling` | Node's Gravity Shield is active due to Chamber Overheat (Cooldown required). |
| `NodeUnreachable` | Connection failed or timed out (Dynamic peer resolution may be triggered). |
| `NodeError` | A generic error returned by the node's execution engine. |
| `Transport` | Errors related to Bincode serialization or framing violations. |

## Client Lifecycle

The `NodeClient` is designed to be stateless. Each request opens a scoped TCP connection using the length-prefixed framing specified in the Obsidian Nexus protocol. For long-term resilience, the client supports storing a `PrimusNR` (Node Record) which can be used to dynamically re-resolve the node's IP address across the DHT if a connection is lost.

---
*Built for the Obsidian Nexus Network*
