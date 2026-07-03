# Obsidian Nexus — Networking (`primus-net-opt`)

This document explains how nodes talk to each other, how they authenticate one another, and how a node finds peers on a network it knows nothing about at startup. It complements [`primus-net-opt/SPECIFICATION.md`](primus-net-opt/SPECIFICATION.md) with a narrative walkthrough rather than a field-by-field reference.

## Table of Contents
- [1. The Big Picture](#1-the-big-picture)
- [2. Identity: the PrimusNR](#2-identity-the-primusnr)
- [3. Transport: QUIC + TCP, No HTTP](#3-transport-quic--tcp-no-http)
- [4. The Noise_XX Handshake](#4-the-noise_xx-handshake)
- [5. How a Node Finds Peers](#5-how-a-node-finds-peers)
- [6. Becoming Reachable: NAT Traversal](#6-becoming-reachable-nat-traversal)
- [7. Propagating Data: Gossip](#7-propagating-data-gossip)
- [8. Guarding the Front Door: GravityShield](#8-guarding-the-front-door-gravityshield)
- [9. Talking to Consensus: the CoreHandle Bridge](#9-talking-to-consensus-the-corehandle-bridge)
- [10. RPC for Clients (SDK/CLI)](#10-rpc-for-clients-sdkcli)
- [11. End-to-End Walkthrough: Cold Start](#11-end-to-end-walkthrough-cold-start)
- [12. Failure Modes](#12-failure-modes)

## 1. The Big Picture

`primus-net-opt` is the only crate that speaks to the outside world. It owns three jobs:

1. **Move bytes** between nodes (transport: QUIC + TCP).
2. **Prove who sent them** (Noise_XX handshake bound to ML-DSA-87 identity).
3. **Find who to talk to** (Kademlia DHT + LAN discovery + gossip).

It does this without knowing anything about consensus. `primus-core` implements the `CoreHandle` trait defined in this crate; the network layer calls into that trait and never reaches into `primus-core` internals directly. This means the entire networking stack can be tested, fuzzed, or reused with a fake `CoreHandle` that has nothing to do with blockchains at all.

```
                     ┌─────────────────────────────┐
                     │         primus-core          │
                     │   (implements CoreHandle)     │
                     └───────────────▲───────────────┘
                                      │ trait calls only
                     ┌────────────────┴────────────────┐
                     │           primus-net-opt          │
                     │                                    │
   discovery.rs ──▶  dht.rs ──▶ gossip.rs ──▶ network.rs ──▶ server.rs
   (LAN beacons)     (Kademlia)  (broadcast)   (TCP)         (QUIC)
                     nat.rs (UPnP)      gravity_shield.rs (ingress filter)
                     noise.rs (Noise_XX + ML-DSA-87)
                     transport.rs (unified inbound stream abstraction)
                     gravity_net.rs (drift-based priority routing)
                     └────────────────────────────────────┘
```

## 2. Identity: the PrimusNR

Every peer is identified by a **`PrimusNR`** (Node Record) — a small, self-signed struct that says "this is who I am and where you can reach me":

- `public_key` — the peer's static ML-DSA-87 identity key.
- `addr_ip` / `addr_port` — where to dial it (updated after NAT resolution, see §6).
- `signature` — an ML-DSA-87 signature over the record's own fields, so nobody can forge a PrimusNR claiming to be someone else.

The peer's **NodeID** used for DHT routing is `SHA3-256(public_key)` — stable across IP changes, so a peer that moves networks keeps its place in everyone's routing tables once it re-announces.

## 3. Transport: QUIC + TCP, No HTTP

Obsidian Nexus deliberately has no HTTP layer anywhere. Two binary transports do all the work:

| Transport | Used for | Port |
|---|---|---|
| QUIC (`quinn`) | Kademlia RPC (bi-directional streams), gossip (persistent uni-directional streams) | `9000` (`P2P_PORT`, configurable) |
| TCP | Chain sync ("Galactic Sync"), and as a fallback inside `broadcast_message` if QUIC is unavailable | `9000` |

Every frame — regardless of transport — is length-delimited and capped at **16 MiB**. `LengthDelimitedCodec` enforces this on TCP; the QUIC path enforces the same ceiling before decryption. Anything larger is a protocol violation and the connection is dropped, logged as a security event. This single number (16 MiB) shows up again in the storage layer's frame budget and in the WASM VM's memory mandate — it's a deliberate, repeated resource ceiling, not a coincidence.

## 4. The Noise_XX Handshake

Before a single byte of application data crosses the wire, two peers run `Noise_XX_25519_ChaChaPoly_SHA256` with one addition on top of stock Noise: **Identity Binding**.

```
Initiator                                   Responder
    │   -> e (X25519 ephemeral pubkey)          │
    ├────────────────────────────────────────▶  │
    │                                            │
    │   <- e, ee, s, es + { PrimusNR, sig }      │
    │      sig = ML-DSA-87(responder_sk, e)      │
    ◀────────────────────────────────────────┤  │
    │                                            │
    │   -> s, se + { PrimusNR, sig }             │
    │      sig = ML-DSA-87(initiator_sk, e_r)    │
    ├────────────────────────────────────────▶  │
    │                                            │
    ▼  both sides verify the ML-DSA-87 sig       ▼
     against the peer's PrimusNR public key
     before accepting the session
```

Why the extra signature: stock Noise_XX proves you hold the static key that *ended up* in the session, but it says nothing about a durable, publicly-known identity ahead of time. Signing the *ephemeral* key with the long-lived ML-DSA-87 static key ties "the key you're Diffie-Hellman-ing with right now" to "the identity you've been broadcasting via the DHT," which is what prevents identity misbinding / MITM in a network where identities are supposed to be quantum-resistant.

Any failure here — bad signature, timeout, malformed payload — is a hard connection close. There is no degraded/anonymous mode.

## 5. How a Node Finds Peers

This is the "как всё находится" part — three independent mechanisms, layered so a node isn't dependent on any single one:

### 5.1 Kademlia DHT (`dht.rs`)
The primary long-term mechanism. Each node keeps a `RoutingTable` of k-buckets indexed by XOR distance from its own NodeID. Peers are learned from:
- Responses to `FindNodeRequest` RPCs (ask a known peer "who's close to this ID?").
- Any inbound message from a new peer, which triggers a routing-table insert.

**Ping-the-Tail eviction**: when a k-bucket is full and a new candidate shows up, the *oldest* entry (the tail) isn't just dropped — it's pinged first. If it answers, it stays and the new candidate is discarded; if it doesn't answer, it's evicted and replaced. This biases the table toward long-lived, reliable peers and is a direct defense against Eclipse attacks (an attacker can't just flood a node with fresh Sybil identities to push out honest peers — it also has to actually beat existing peers in a liveness contest).

### 5.2 LAN Discovery (`discovery.rs`)
UDP broadcast beacons for same-network bootstrap — the fast path when two nodes are on the same LAN and shouldn't need a DHT round-trip or NAT traversal to find each other at all.

### 5.3 Gossip-Carried Records
`PrimusNR`s ride along on gossip and sync traffic. Once a node has *any* connection into the network, it starts learning about further peers passively as records propagate, independent of explicit DHT queries.

None of these require a central bootstrap server beyond an initial seed peer address to dial once — after that, discovery is fully peer-driven.

## 6. Becoming Reachable: NAT Traversal

Finding peers is only half the problem — a node also has to make *itself* findable from the public internet. `NatService::open_world` (via `igd_next`, i.e. UPnP) runs on startup:

1. **Gateway discovery** — locate a UPnP-capable router on the LAN.
2. **Port mapping** — map the configured `P2P_PORT` for both TCP and UDP.
3. **External IP resolution** — ask the gateway what the node's public IP is.

The resolved external `ip:port` is written into the node's own `PrimusNR`, re-signed, and broadcast through the DHT — so remote peers that only ever see this node's *advertised* record can still dial in correctly. If UPnP discovery fails (no capable gateway, corporate network, etc.), the node logs a warning, falls back to whatever local IP it has, and keeps running — inbound connectivity is just degraded to whatever manual port forwarding the operator sets up, not fatal to the node.

## 7. Propagating Data: Gossip

`GossipService` is a TTL-decaying epidemic broadcast — the mechanism that gets a new transaction or a new block from wherever it originated to the whole network without every node talking to every other node directly.

| Message | TTL behavior |
|---|---|
| `NewReaction(data, ttl)` | decrements by 1 per hop, dropped at TTL 0 |
| `NewCrystal(data, ttl)` | no decay — blocks propagate unchanged, every hop rebroadcasts |
| `Sync(_)` | always treated as TTL = 1 — one hop only, then decays |

**Deduplication**: every message is fingerprinted as `SHA3-256(bincode::serialize(message))`. Each node keeps a `HashSet<[u8; 32]>` of seen IDs, capped at 10,000 entries; once full, the oldest 1,000 are evicted. This is what stops a gossiped message from looping forever — a node that's already seen a message's hash just drops the duplicate.

**Source exclusion**: a message is never echoed straight back to the peer that just sent it — that would be a trivially wasteful round-trip.

**Local processing doesn't block propagation**: when a node receives `NewReaction`, it forwards to peers *and* spawns local processing (`CoreHandle::shield_filter` → `CoreHandle::on_reaction`) concurrently rather than validating-then-forwarding serially. This keeps propagation latency independent of how long local validation takes.

## 8. Guarding the Front Door: GravityShield

Every inbound buffer — gossip, RPC, or sync, doesn't matter which — passes through the same filter before it's trusted with a full deserialize:

1. **Bincode framing check** — reject anything that isn't valid Bincode immediately.
2. **`validate_structure()`** — rkyv-backed structural/field-range check (this happens *after* the initial bincode deserialize, not as a replacement for it — the two formats do different jobs here).
3. **Sanity checks** — non-empty public key, non-negative `energy`.
4. **Thermal gate** — if the node's Chamber Temperature is ≥ 150.0 K, non-Architect reactions are dropped outright (this is the "Gravity Shield" proper — the network layer's automatic self-protection against being overwhelmed).
5. **Phantom sender check** — reactions from public keys with no on-chain balance are silently dropped. This one is enforced in `primus-core::bridge`, not in `primus-net-opt` itself, and the drop is deliberately silent (no error response) specifically to deny an attacker a cheap oracle for probing which addresses exist on-chain.

Only after all of that does a reaction reach the Sectoral Mempool.

## 9. Talking to Consensus: the CoreHandle Bridge

`primus-net-opt` defines `CoreHandle` as a trait; `primus-core::CoreHandleImpl` is the concrete implementation. The network layer only ever calls the trait — it has no idea `PrimusEngine`, `SectoralMempool`, or `PrimusStorage` even exist.

Key methods the network layer relies on: `on_reaction`, `on_crystal`, `get_crystal_bytes`, `local_state`, `is_syncing` / `set_sync_target` / `finish_sync`, `shield_filter`, `push_bytes`, `get_atom_state`, `on_get_proof`. `CoreHandleImpl` additionally keeps a back-reference to the live `PrimusNetwork` instance so it can trigger automatic gossip rebroadcasts once a reaction is validated — that's the one place the "inversion" bends slightly, and it's contained entirely inside `primus-core`, not exposed to `primus-net-opt`.

## 10. RPC for Clients (SDK/CLI)

Wallets and CLIs don't join the gossip mesh — they talk to one specific node over a scoped TCP connection using the same `PrimusMessage` envelope (bincode + length-prefixed framing):

| Request | Response | Timeout |
|---|---|---|
| `SubmitReaction { reaction_bytes }` | `ReactionAck { reaction_hash }` | 10s |
| `FetchState { address }` | `StateResponse { mass, nonce, last_hash, element }` | 5s |
| `GetProof { address }` | `ProofResponse(MerkleProof)` | 10s |
| *(any)* | `NodeError { reason }` on failure | — |

`SubmitReaction` bytes go through the exact same `GravityShield::filter_bytes()` → `CoreHandle::push_bytes()` path as a gossiped reaction — a client isn't a trusted shortcut around ingress filtering. GravityShield rejections on this path increment the same shared `frame_drops` counter as everything else.

## 11. End-to-End Walkthrough: Cold Start

Putting §5–§8 together, here's what happens when a brand-new node boots with only one seed peer address:

1. Node generates/loads its ML-DSA-87 identity, builds its own `PrimusNR` with a placeholder local address, signs it.
2. `NatService::open_world` tries UPnP; if it succeeds, the `PrimusNR`'s address is updated to the resolved external `ip:port` and re-signed.
3. Node dials the seed peer over QUIC. Noise_XX handshake runs (§4); both sides verify each other's `PrimusNR` signature.
4. Node sends a `FindNodeRequest` for its own NodeID to the seed peer, populating its routing table with the closest known peers.
5. Node repeats step 3–4 against newly-discovered peers, converging its k-buckets (Kademlia iterative lookup, with `Box::pin` futures to keep this off the stack on Windows).
6. In parallel, LAN discovery beacons are sent/listened for in case any peers are on the same local network.
7. Once connected to enough peers, gossip starts flowing both ways; the node begins learning further `PrimusNR`s passively from message payloads without needing explicit DHT queries for each one.
8. If the node is behind the current chain tip, it sets a sync target and pulls Crystals via TCP sync rather than waiting for gossip to replay history.

## 12. Failure Modes

| Situation | Behavior |
|---|---|
| Noise handshake fails (bad sig, timeout, malformed payload) | Immediate connection termination, no fallback to plaintext |
| UPnP gateway not found / mapping fails | Warning logged, node continues with local IP only — reduced inbound reachability, not fatal |
| Frame exceeds 16 MiB | Connection terminated, logged as a security event |
| GravityShield rejects a frame | Dropped, `warn`-level log with reason (`Chamber Overheat`, `Phantom Sender`, structural failure, etc.), `frame_drops` counter incremented |
| k-bucket full, tail peer unresponsive | Tail evicted, new candidate inserted |
| k-bucket full, tail peer responsive | New candidate discarded, tail kept |

---
*See also: [`primus-net-opt/README.md`](primus-net-opt/README.md) for the crate-level module table, [`primus-net-opt/SPECIFICATION.md`](primus-net-opt/SPECIFICATION.md) for the field-by-field wire format reference, and [`SPECIFICATION.md`](SPECIFICATION.md) §4–§6 for how this fits into the workspace-wide protocol.*
