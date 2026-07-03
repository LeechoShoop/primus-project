# Primus Core: Obsidian Nexus Engine

`primus-core` is the central orchestration crate for the Obsidian Nexus node. It manages the consensus state machine, the deterministic kinetic engine, and coordinates high-performance networking with administrative control.

## Component Architecture

The node is built on a modular "Orchestration" layer that decouples the consensus logic from the transport mechanics:

1.  **Kinetic Engine**: Performs deterministic synthesis of reactions into crystals using physics-inspired entropy and energy models.
2.  **Sectoral Mempool**: A high-capacity, partitioned transaction pool that prioritizes high-energy reactions for inclusion.
3.  **The Bridge**: A decoupled interface (`CoreHandleImpl`) that connects the engine to the `primus-net-opt` networking stack.
4.  **Secure IPC Server**: Provides local, authenticated administrative access via platform-specific secure sockets (Named Pipes on Windows, Unix Domain Sockets on Linux).

## Operational Telemetry

Operators can monitor node health in real-time using the administrative IPC interface (via `primus-cli`). The status report includes:

- **Height**: The current index of the solidified crystal chain.
- **Peers**: Number of active nodes in the Kademlia DHT.
- **TCP Cache**: Count of active persistent legacy transport connections.
- **Frame Drops**: A security metric tracking frames rejected due to protocol violations or size limits.

## Project Maturity

**Current Status**: `Active Development — Shim Layer Complete`

The following shims are in place to bridge divergences between primus-core
and qualified (frozen) modules. All shims are in primus-core and do not
require changes to qualified modules.

| Shim | File | Fixes |
|------|------|-------|
| Frame chunking / outbound guard | `src/framing.rs` | BLK-002, DIV frame size |
| rkyv boundary validation | `src/bridge.rs` | DIV rkyv bypass |
| ML-DSA-87 threading | `src/crypto_shim.rs` | DIV signature threading |
| Physics normalization | `src/physics_shim.rs` | DIV physics constants |
| GravityShield Layer 5 | `src/bridge.rs` | DIV phantom sender check |

## Getting Started

To launch the node with default settings:

```powershell
cargo run --release -p primus-core
```

For administrative control, use the `primus-cli` tool.

---
<!-- Last sync: 2026-05-15 | fixes: shim-layer-complete, frame-chunking, rkyv-boundary, ml-dsa-threading, physics-shim, gravity-shield-l5 -->
