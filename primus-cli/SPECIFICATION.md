# Technical Specification: primus-cli

This document defines the architecture, command hierarchy, and security protocols of the `primus-cli`. The CLI serves as the primary administrative and user interface for the Obsidian Nexus node, abstracting low-level IPC and transport complexities into a stable command-line environment.

## 1. Transport Architecture

The `primus-cli` communicates with the local or remote node via two distinct bridges depending on the command's sensitivity:

### Administrative IPC (Local)
For sensitive operations (e.g., node shutdown, manual peering), the CLI connects to the node's local IPC listener.
*   **Unix**: Unix Domain Socket at `/tmp/primus.sock`.
*   **Windows**: Named Pipe at `\\.\pipe\primus-pipe`.
*   **Framing**: `[4-byte little-endian length] ++ [bincode-serialized IpcRequest]`.
*   **Timeout**: All IPC operations enforce a **5-second I/O timeout**.

### Network Gateway (Remote)
For chain queries and transaction broadcasts, the CLI utilizes the standard node transport.
*   **Port**: Default administrative port is **9001** (or 9000 for standard P2P interactions).
*   **Framing**: `[4-byte big-endian length] ++ [bincode-serialized CliMessage]`.
*   **Protocol**: Pure TCP/Bincode (No HTTP/JSON).

## 2. Command Hierarchy

The CLI is built using `clap` and organized into logical subcommands:

### Wallet Management
| Command | Action |
| :--- | :--- |
| `wallet create` | Generates a new 24-word BIP-39 mnemonic and derives a 32-byte seed into an ML-DSA-87 keypair. |
| `wallet show` | Displays the hex-encoded public address and file status for an existing wallet. |
| `wallet list` | (Intended) Lists all wallets found in the `.secrets` directory. |

### Network Control
| Command | Action |
| :--- | :--- |
| `admin connect-peer` | Instructs the node to establish a QUIC connection with a specific remote address. Requires Architect signing. |
| `admin status` | Fetches a `StatusReport` from the node via IPC, including chain height and peer count. |
| `admin shutdown` | Authenticated command to gracefully terminate the node process. |

### Chain Interaction
| Command | Action |
| :--- | :--- |
| `send` | A high-level workflow that fetches the sender's current on-chain state, builds a `Transaction`, signs it, and broadcasts it to the network. |

## 3. The Architect’s Protocol (Security)

Sensitive administrative commands are protected by a **Challenge-Response** protocol to prevent unauthorized control or replay attacks.

### Command Signing Process:
1.  **Challenge**: The CLI sends `IpcRequest::GetChallenge` to the node.
2.  **Nonce**: The node generates and returns a cryptographically secure 32-byte random nonce.
3.  **Signature**: The CLI loads the Architect's wallet and signs the 32-byte nonce using ML-DSA-87.
4.  **Authenticated Request**: The CLI sends the signed command (e.g., `AdminShutdown { signature }`).
5.  **Verification**: The node validates the signature against the hardcoded `architect_pk` before execution.

## 4. Windows Stability Guards

Due to the intensive memory requirements of post-quantum cryptography, `primus-cli` enforces strict stack management:
*   **16 MiB Stack Size**: All operations involving ML-DSA-87 (loading wallets, generating keys, and signing) are offloaded to `std::thread` with an explicit **16 MiB stack allocation**.
*   **Non-Blocking**: On Windows, these threads are wrapped in `tokio::task::spawn_blocking` to ensure the asynchronous runtime remains responsive during heavy key expansion.

## 5. Diagnostics & Monitoring

The CLI provides real-time visibility into the node's internal state via the `Status` request.

### Gravity Shield Reporting (Planned)
Future revisions of the `admin status` command will include real-time metrics from the **Gravity Shield**:
*   **Chamber Temperature**: Current thermal state of the reactor (Gate threshold: 150.0 K).
*   **Packet Drops**: Statistics on malformed or malicious frames rejected by the pre-deserialization filter.

---

## Commands Reference

### Example: Remote Peer Connection
```bash
# Instruct the node to connect to a new peer using the architect identity
primus admin connect-peer --addr 1.2.3.4:9000 --wallet .secrets/master.wallet
```

### Example: Sending Mass
```bash
# Build, sign, and broadcast a transfer transaction
primus send --to <RECIPIENT_HEX> --amount 5000 --from my.wallet
```

### Example: Node Diagnostics
```bash
# Fetch current node health and metrics
primus admin status
```
