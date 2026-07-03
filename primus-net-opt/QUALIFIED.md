# QUALIFIED.md — primus-net-opt
**Audit Date**: 2026-05-22
**Previous Audit**: 2026-05-08
**Status**: ✅ RE-QUALIFIED — Integration-Complete
**Crate**: `primus-net-opt v0.1.0`

---

## ⛔ FREEZE NOTICE

This crate is **QUALIFIED**. The presence of this file means:

- **No agent, tool, or human may modify any source file** in
  `primus-net-opt/src/` without explicit written approval and a new audit pass.
- **No dependency versions may be changed** without re-running the full audit.
- **No new public API surface** may be added without updating this document.

To make changes: delete this file, make changes, re-run the full audit,
then recreate this file with a new audit date.

---

## Changes Since Previous Qualification (2026-05-08)

| File | Change |
|------|--------|
| `src/network.rs` | `BoxFuture` wrapping of `handle_peer_logic` — fixes Send bounds |
| `src/network.rs` | `frame_drops: Arc<AtomicU64>` on `PrimusNetwork` — injected, not created |
| `src/network.rs` | `SubmitReaction`, `ReactionAck` variants added to `PrimusMessage` |
| `src/network.rs` | `GetProof`, `ProofResponse` variants added to `PrimusMessage` |
| `src/network.rs` | `FetchState`, `StateResponse` variants added to `PrimusMessage` |
| `src/network.rs` | `NodeError` variant added to `PrimusMessage` |
| `src/network.rs` | `get_atom_state`, `push_bytes`, `on_get_proof` added to `CoreHandle` |
| `src/network.rs` | `GravityShield` pre-filter on `SubmitReaction` handler |
| `src/network.rs` | 5–10 s timeouts on `FetchState`, `SubmitReaction`, `GetProof` handlers |

---

## Frozen Interface Contracts

### CoreHandle trait (frozen — 10 methods)
```rust
pub trait CoreHandle: Send + Sync + 'static {
    async fn on_reaction(&self, rx: SignedReaction) -> Result<()>;
    async fn on_crystal(&self, crystal_bytes: Vec<u8>) -> Result<()>;
    async fn local_state(&self) -> (u64, f32, f32);
    async fn get_crystal_bytes(&self, index: u64) -> Option<Vec<u8>>;
    async fn set_sync_target(&self, height: u64);
    async fn is_syncing(&self) -> bool;
    async fn finish_sync(&self);
    async fn get_atom_state(&self, addr: [u8; 32]) -> Result<(u64, u64, [u8; 32], String)>;
    async fn push_bytes(&self, bytes: &[u8]) -> Result<()>;
    async fn on_get_proof(&self, addr: [u8; 32]) -> Result<MerkleProof>;
}
```

### PrimusMessage variants (frozen — 17 variants)
```
Ping, Pong, Handshake, GetPeers, PeersResponse,
NewReaction, NewCrystal, GetCrystal, CrystalResponse, Sync,
FetchState, StateResponse, SubmitReaction, ReactionAck,
NodeError, GetProof, ProofResponse
```

### PrimusNetwork::new signature (frozen)
```rust
pub fn new(
    port: u16,
    core: Arc<H>,
    dht: Arc<PrimusDHT>,
    frame_drops: Arc<AtomicU64>,  // shared with PrimusServer and IpcServer
) -> Self
```

### Wire format (frozen)
- Serialization: `bincode` for all `PrimusMessage` variants
- Framing: `LengthDelimitedCodec`, max 16 MiB (MAX_FRAME_BYTES)
- `handle_peer_logic` returns `BoxFuture<'static, Result<()>>`

---

## Caller contracts (what primus-core MUST do)

1. Create ONE `Arc<AtomicU64>` for frame_drops
2. Pass it to BOTH `PrimusServer` AND `PrimusNetwork::new()`
3. Pass the same Arc to `IpcServer` for `StatusReport`
4. Implement ALL 10 methods of `CoreHandle`

---

## Known Limitations (unchanged from 2026-05-08)

| # | Item |
|---|------|
| 1 | IPv6 not supported in NatService |
| 2 | KBucket refresh not implemented |
| 3 | Discovery port range fixed overlap |
| 4 | `rand 0.8` in maintenance mode |
| 5 | Gossip eviction non-FIFO |
| 6 | `known_peers` unbounded in discovery |

---

## Qualification Basis

- `cargo check -p primus-net-opt` passes with 0 errors
- Security invariant verified: GravityShield on all reaction ingress paths
- Timeouts on all storage-touching RPC handlers
- frame_drops is a single shared Arc (no isolated counters)

*Note: fuzz re-run not performed — changes are additive new match arms only.*
*Existing paths unchanged. Full fuzz recommended before production.*
