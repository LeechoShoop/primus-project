# primus-cli — Obsidian Nexus Command Line Interface

`primus-cli` is the administrative and user-facing command-line tool for the
Obsidian Nexus Layer-1 network. It communicates with a running `primus-core`
node via both TCP (for balance/transaction queries) and secure IPC (for
administrative commands).

## Installation

```powershell
cargo build --release -p primus-cli
```

The binary will be at `target/release/primus-cli` (or `primus-cli.exe` on Windows).

## Usage

```
primus-cli [OPTIONS] <COMMAND>
```

### Global Options

| Flag | Env Variable | Default | Description |
|------|-------------|---------|-------------|
| `--node-host <HOST>` | `PRIMUS_NODE_HOST` | `127.0.0.1` | primus-core node host address |
| `--node-port <PORT>` | `PRIMUS_NODE_PORT` | `9000` | primus-core node TCP port |

### Commands

| Command | Description |
|---------|-------------|
| `wallet create [--path]` | Generate a new 24-word ML-DSA-87 wallet |
| `wallet show [--path]` | Display address and info for an existing wallet |
| `balance <address>` | Check on-chain mass (balance) for an address |
| `balance <address> --prove` | Fetch and verify a Merkle balance proof |
| `send --to <addr> --amount <n> [--from]` | Broadcast a signed Transfer reaction |
| `admin status` | Retrieve node health via IPC (height, peers, frame drops) |
| `admin shutdown [--wallet]` | Gracefully shut down the node (requires architect key) |
| `admin connect-peer --addr <ip:port> [--wallet]` | Instruct node to connect to a peer |

## Connecting to a Remote Node

By default the CLI connects to a local node at `127.0.0.1:9000`.
Use flags or environment variables to connect remotely:

```bash
# Via flags
primus-cli --node-host 192.168.1.100 --node-port 9000 balance <address>

# Via flags — send a transaction
primus-cli --node-host 192.168.1.100 --node-port 9000 send \
  --to <recipient-hex-address> --amount 1000 --from my.wallet

# Via environment variables (useful in scripts and containers)
export PRIMUS_NODE_HOST=192.168.1.100
export PRIMUS_NODE_PORT=9000
primus-cli balance <address>
primus-cli send --to <addr> --amount 500

# Container / CI usage
PRIMUS_NODE_HOST=10.0.0.5 PRIMUS_NODE_PORT=9000 primus-cli status
```

## Security Notes

### Administrative Commands (IPC)
Admin commands (`admin status`, `admin shutdown`, `admin connect-peer`) use the
**secure IPC channel** — not the TCP port. The IPC path is OS-specific:
- **Windows**: Named Pipe `\\.\pipe\primus-nexus-<USER_SID>`
- **Unix**: `$XDG_RUNTIME_DIR/primus.sock` (fallback: `$HOME/.primus/run/primus.sock`)

The node verifies IPC socket ownership before accepting connections.

### Noise_XX Encryption
TCP connections (balance, send) should use `NodeClient::new_with_noise()` in
consumer code. The CLI's built-in commands use `NodeClient::new()` during the
BLK-003 transition period — full Noise wiring of CLI commands is a planned
next step.

### Wallet Security
- Wallets are encrypted with ML-DSA-87 (post-quantum safe).
- Signing keys are **never** persisted in plaintext — they are re-derived from
  seed on every signing operation and zeroed immediately after use.
- Run signing operations from a thread with 16 MiB stack (handled automatically
  by the CLI for `send` and admin commands).

## Examples

```bash
# Create a wallet
primus-cli wallet create --path my.wallet

# Check balance locally
primus-cli balance <hex-address>

# Check balance with Merkle proof verification
primus-cli balance <hex-address> --prove

# Send 1000 mass
primus-cli send --to <recipient-hex> --amount 1000 --from my.wallet

# Remote node status
primus-cli --node-host 10.0.0.1 admin status
```

---
*Built for the Obsidian Nexus Network — primus-cli v0.1.0*
<!-- Last sync: 2026-05-15 | fixes: BLK-001 remote address parameterization, BLK-003 noise-xx transition -->
