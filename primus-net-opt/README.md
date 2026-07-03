# primus-net-opt — Post-Quantum Secure P2P Transport

`primus-net-opt` is the high-performance networking engine for the Obsidian Nexus ecosystem. It defines a transport layer built from the ground up to eliminate the vulnerabilities of the legacy web while providing absolute cryptographic resilience against post-quantum adversaries.

## Core Pillars

### 🚫 No-HTTP Policy
Obsidian Nexus rejects the overhead and attack surface of HTTP/HTTPS. `primus-net-opt` utilizes a custom transport stack based on **QUIC** and **WebTransport**, providing multiplexed, low-latency communication without the baggage of legacy web-stack vulnerabilities. TCP is the primary transport for gossip and chain sync (`PrimusNetwork`); QUIC is used for Kademlia RPC bi-streams and persistent gossip uni-streams, with TCP as fallback in `broadcast_message`.

### 🛡️ Quantum-Resistant Identity
Identity is not an afterthought. Every connection undergoes a mandatory **Noise_XX** handshake integrated with **ML-DSA-87** post-quantum signatures. Identity verification occurs before a single byte of application data is exchanged, ensuring that the network remains secure even in the face of quantum computing advancements.

### 🌐 Internet-Ready & NAT-T
The network is designed for global, decentralized connectivity. With native **NatService** integration using UPnP, nodes automatically map ports and resolve external IP addresses. This ensures that independent nodes on different PCs can connect remotely via the internet without manual firewall configuration. Default QUIC port: 9000 (`P2P_PORT` in server.rs). Configurable at startup.

### ⚡ High-Throughput Performance
Leveraging `tokio` for asynchronous I/O and `quinn` for QUIC implementation, `primus-net-opt` is optimized for high-speed gossip and synchronization. The architecture supports thousands of concurrent streams, providing the scalability required for a global L1 ecosystem.

## Crate Structure
| Module | Purpose |
|---|---|
| `dht` | Kademlia routing table (`RoutingTable` + `PrimusDHT`) |
| `gossip` | TTL-decaying epidemic broadcast (`GossipService`) |
| `discovery` | UDP LAN peer discovery via broadcast beacons |
| `network` | TCP transport, `PrimusNetwork<H>`, `CoreHandle` trait |
| `server` | QUIC + WebTransport server, `PeerSession`, nonce tracking |
| `noise` | Noise_XX_25519_ChaChaPoly_SHA256 + ML-DSA-87 binding |
| `nat` | UPnP port mapping via `igd_next` |
| `gravity_shield` | Pre-deserialization ingress filter |
| `transport` | Unified inbound stream abstraction |
| `gravity_net` | Galactic drift helpers for priority routing |

## Performance & Stability
The crate is engineered for production-grade stability across platforms. Specifically for **Windows**, ML-DSA key material (`ml_dsa_sk`) is heap-allocated via `Arc<Box<[u8]>>` to avoid placing 7+ KB on the stack (FIX 1). The iterative Kademlia lookup uses `Box::pin` futures to move state to the heap instead of deep async recursion (FIX 2).

## Safety & Security (Post-Audit 2026-05)
Incoming traffic is gated by the **Gravity Shield**, a multi-layer pre-deserialization filter that drops malformed frames and malicious payloads before they can impact the execution engine. This first line of defense is critical for maintaining network integrity under DDoS conditions.

- 16 MiB TCP frame limit enforced by `LengthDelimitedCodec` (network.rs)
- 16 MiB QUIC gossip payload limit bounded by strict framing limits before decryption (server.rs) to prevent memory exhaustion by unbounded QUIC streams.
- 100 concurrent streams per peer via `Semaphore` (server.rs, PeerSession)
- Ping-the-tail DHT eviction for Eclipse attack mitigation (dht.rs)
- Gossip deduplication: SHA3-256 IDs, bounded at 10,000 entries (gossip.rs)

## RPC over TCP

SDK and CLI clients submit transactions and query state directly over TCP
using the same `PrimusMessage` protocol as node gossip:

| Request | Response |
|---------|----------|
| `SubmitReaction` | `ReactionAck` or `NodeError` |
| `FetchState` | `StateResponse` or `NodeError` |
| `GetProof` | `ProofResponse` or `NodeError` |

All RPC requests pass through `GravityShield` pre-validation.
Handlers apply 5–10 second timeouts to protect the peer session loop.

---
**Status: Hardened & Optimized (Post-Audit 2026-05)**

The networking stack is now fully aligned with the 16 MiB framing mandate, featuring TCP connection pooling, non-blocking DHT maintenance, and zero-copy `rkyv` ingress. The core-networking boundary relies on the `MempoolIngress` interface, which is now an `#[async_trait]` to seamlessly support deep asynchronous I/O and non-blocking stream handling.

<!-- Last sync: 2026-05-22 | fixes: 16MiB-framing, semaphore-guarding, tcp-pooling, rkyv-ingress, noise-nonce-fix, kademlia-autoreg, windows-discovery-fix -->
