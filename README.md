# Obsidian Nexus (Primus)

A post-quantum Layer-1 blockchain written in Rust. Every identity, transaction, and peer connection is authenticated with **ML-DSA-87** (NIST post-quantum signatures) — there is no legacy elliptic-curve fallback. The network rejects HTTP entirely in favor of a custom QUIC/TCP + Noise_XX transport, and consensus runs on a deterministic, physics-inspired execution model called the **Kinetic Engine**.

> **Status: research project, not under active development.** Core consensus, networking, storage, CLI, SDK, and a WASM smart-contract VM are implemented and integrated, but this repository is published as-is and is not being maintained going forward. See [Project Status](#project-status) below.

## Table of Contents
- [Why](#why)
- [Architecture](#architecture)
- [Workspace Crates](#workspace-crates)
- [Key Technical Pillars](#key-technical-pillars)
- [Getting Started](#getting-started)
- [Security](#security)
- [Project Status](#project-status)
- [License](#license)

## Why

Obsidian Nexus exists to answer one question concretely: what does a production-shaped L1 look like if you assume quantum adversaries and design out the HTTP/JSON attack surface from day one? The project trades ecosystem convenience (no EVM, no JSON-RPC) for a fully binary, post-quantum-native protocol stack — Noise_XX handshakes bound to ML-DSA-87 identities, bincode/rkyv wire formats, and a Merkle-Patricia Trie state root with compact proof generation.

## Architecture

```
primus-types    (wire structs, ML-DSA-87 constants, no crypto logic)
     |
primus-storage  (Sled + Merkle-Patricia Trie, exclusive disk I/O owner)
     |
primus-vm       (deterministic execution: native PVM + WASM/Wasmtime)
     |
primus-core     (Kinetic Engine consensus, GravityShield, IPC admin server)
     |         \
primus-net-opt  \ (QUIC/TCP transport, Noise_XX, Kademlia DHT, gossip)
     |            \
primus-sdk ------- primus-cli
(client library)   (wallet / admin / chain CLI)
```

Dependencies flow strictly downward — `primus-storage` never imports from `primus-core`, `primus-net-opt`, `primus-sdk`, or `primus-cli`. `primus-net-opt` has zero compile-time dependency on `primus-core` internals; the two are decoupled through the `CoreHandle` trait (dependency inversion), so the networking stack only ever talks to a trait object.

## Workspace Crates

| Crate | Role | Docs |
|---|---|---|
| `primus-types` | Wire structs (`Atom`, `SignedReaction`, `PrimusNR`), ML-DSA-87 constants, `no_std`-compatible core types | [README](primus-types/README.md) · [SPEC](primus-types/SPECIFICATION.md) |
| `primus-storage` | Sled-backed persistence, Merkle-Patricia Trie state root, compact Merkle proofs | [README](primus-storage/README.md) · [SPEC](primus-storage/SPECIFICATION.md) |
| `primus-vm` | Deterministic execution engine — native PVM + Wasmtime-backed smart contracts, gas metering | [README](primus-vm/README.md) |
| `primus-core` | Kinetic Engine consensus, Sectoral Mempool, GravityShield ingress filter, secure IPC admin server | [README](primus-core/README.md) · [SPEC](primus-core/SPECIFICATION.md) |
| `primus-net-opt` | QUIC/TCP transport, Noise_XX + ML-DSA-87 handshake, Kademlia DHT, gossip, UPnP NAT traversal | [README](primus-net-opt/README.md) · [SPEC](primus-net-opt/SPECIFICATION.md) |
| `primus-sdk` | Client library: wallet management, transaction building, Noise-encrypted `NodeClient`, light-client proof verification | [README](primus-sdk/README.md) · [SPEC](primus-sdk/SPECIFICATION.md) |
| `primus-cli` | `primus-cli` binary — wallet, balance/send, and Architect-signed admin commands over secure IPC | [README](primus-cli/README.md) · [SPEC](primus-cli/SPECIFICATION.md) |

## Key Technical Pillars

- **Post-quantum by default.** ML-DSA-87 (2592-byte public keys, 4627-byte signatures) is the only signature scheme in the protocol. There is no secp256k1/Ed25519 fallback anywhere on the wire.
- **No-HTTP transport.** Peer-to-peer traffic runs over QUIC (Kademlia RPC, gossip) and TCP (chain sync), secured end-to-end with a Noise_XX handshake that binds ephemeral session keys to ML-DSA-87 static identities before any application data is exchanged.
- **GravityShield ingress filter.** Every inbound frame — gossip, RPC, or sync — passes a multi-layer pre-deserialization filter (structural checks via `rkyv`, thermal gating, phantom-sender rejection) before it ever reaches consensus logic.
- **Merkle-Patricia Trie state.** State root is a 4-bit-nibble MPT over `SHA3-256(public_key)`, BLAKE3-hashed at every internal node, with compact (~2 KB) inclusion/exclusion proofs for light clients.
- **Kinetic Engine consensus.** A deterministic, physics-inspired synthesis model (Galactic Drift sector selection, thermal/entropy-gated block synthesis) replaces probabilistic PoW mining while still using a nonce-search "crystal synthesis" step.
- **WASM smart contracts under a strict resource mandate.** Wasmtime-backed contracts run under a 16 MiB memory / 512 KiB stack / 4 MiB module-size ceiling, with gas metering charged before every host call.
- **Windows-safe cryptography.** ML-DSA-87 key expansion and signing require ~4 MiB of stack; all such operations are explicitly offloaded to threads with a 16 MiB stack allocation to avoid `STATUS_STACK_OVERFLOW` crashes, enforced identically on Linux to prevent silent stack corruption under concurrent load.

## Getting Started

```powershell
# Build the whole workspace
cargo build --release

# Run a node with default settings
cargo run --release -p primus-core

# Interact with it via the CLI (in a second terminal)
cargo build --release -p primus-cli
target/release/primus-cli wallet create --path my.wallet
target/release/primus-cli balance <hex-address>
```

By default the node listens on TCP/QUIC port `9000`; the CLI talks to `127.0.0.1:9000` unless overridden with `--node-host` / `--node-port` or the `PRIMUS_NODE_HOST` / `PRIMUS_NODE_PORT` environment variables. See [`primus-cli/README.md`](primus-cli/README.md) for the full command reference.

## Security

- All P2P connections require a Noise_XX handshake bound to ML-DSA-87 identities before any data is exchanged; plain TCP is rejected by live nodes.
- Administrative control (shutdown, peer connect) requires a signed Challenge-Response exchange against a hardcoded Architect public key, delivered over a user-restricted local IPC channel (Unix socket with `0600` permissions, or a Windows Named Pipe scoped to the user's SID) — never over the network.
- Sensitive material (BIP-39 mnemonics, ML-DSA-87 seeds, signing keys) implements `ZeroizeOnDrop` and is never persisted in plaintext; signing keys are re-derived from seed on every operation and wiped immediately after use.
- Inbound frames are bounded to 16 MiB and pass GravityShield structural/thermal/phantom-sender checks before deserialization.
- The project underwent a pre-publication security audit; findings (hardcoded seeds in example configs, missing `.gitignore` entries, localhost defaults) were addressed prior to publishing this repository. See individual crate specs for audit-driven fixes (tagged `DIV-*`, `BLK-*` in changelog comments).

This is a research/portfolio project and has **not** undergone third-party security review. Do not use it to secure real value.

## Project Status

This is a solo research/portfolio project built to explore post-quantum L1 design end-to-end — consensus, networking, storage, a WASM VM, and client tooling. **Active development has stopped.** The repository is published in its current state for anyone to read, fork, or build on; no further features, fixes, or releases are planned by the original author, and issues/PRs may go unanswered.

Per-crate state at the point of publication:

| Crate | Status |
|---|---|
| `primus-types` | Stable wire format, frozen bincode ordering |
| `primus-storage` | Phase 2 complete — MPT + compact proofs + GC |
| `primus-vm` | Native PVM + Wasmtime backend implemented |
| `primus-core` | Shim layer complete, consensus + IPC admin server functional |
| `primus-net-opt` | Hardened & optimized post-audit |
| `primus-sdk` | Noise-encrypted client, automatic retry/back-off |
| `primus-cli` | Wallet, balance/send, admin commands implemented |

Known open items are tracked in each crate's own spec (e.g. reorg atomicity is not fully transactional — see [§11 of SPECIFICATION.md](SPECIFICATION.md#11-known-limitations--recovery)). Forks are welcome; there's no CLA or contribution process since the project isn't maintained.

## License

See `LICENSE` in the workspace root.
