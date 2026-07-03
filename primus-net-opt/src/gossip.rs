// =============================================================================
// primus-net-opt/src/gossip.rs
//
// MIGRATION: Moved from primus-core/src/gossip.rs.
//
// CHANGE: GossipService<H: CoreHandle> is now generic over the core handle.
// Local processing (on_reaction, on_crystal) is delegated through CoreHandle
// instead of calling PrimusProcessor directly.
// =============================================================================

use sha3::{Digest, Sha3_256};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::network::{CoreHandle, PrimusMessage, PrimusNetwork};

/// Maximum number of seen message IDs to keep in memory.
/// When the set exceeds this, the oldest 10% are evicted.
const MAX_SEEN: usize = 10_000;
const EVICT_COUNT: usize = 1_000;

pub struct GossipService<H: CoreHandle> {
    network: PrimusNetwork<H>,
    seen_messages: Arc<Mutex<HashSet<[u8; 32]>>>,
}

// Manual Clone — H is behind Arc<H>, so H: Clone is not required.
impl<H: CoreHandle> Clone for GossipService<H> {
    fn clone(&self) -> Self {
        Self {
            network: self.network.clone(),
            seen_messages: Arc::clone(&self.seen_messages),
        }
    }
}

impl<H: CoreHandle> GossipService<H> {
    pub fn new(network: PrimusNetwork<H>) -> Self {
        Self {
            network,
            seen_messages: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Propagate a message to all known peers, applying TTL decay.
    ///
    /// Performs local processing (reaction → mempool, crystal → processor)
    /// via `CoreHandle` before propagation, so the local node always
    /// processes its own gossip before forwarding.
    pub async fn spread(&self, message: PrimusMessage, source_addr: Option<String>) {
        let (current_ttl, should_decay) = match &message {
            PrimusMessage::NewReaction(_, ttl) => (*ttl, true),
            PrimusMessage::NewCrystal(_, ttl) => (*ttl, false),
            PrimusMessage::Sync(_) => (1, true),
            _ => return,
        };

        if current_ttl == 0 {
            return;
        }

        // ── Deduplication ─────────────────────────────────────────────────────
        let msg_id = self.message_id(&message);
        {
            let mut seen = self.seen_messages.lock().await;
            if seen.contains(&msg_id) {
                return;
            }
            seen.insert(msg_id);

            // Bounded eviction — remove oldest entries when over capacity.
            if seen.len() > MAX_SEEN {
                let to_remove: Vec<_> = seen.iter().take(EVICT_COUNT).cloned().collect();
                for k in to_remove {
                    seen.remove(&k);
                }
            }
        }

        // ── Local processing via CoreHandle ───────────────────────────────────
        match &message {
            PrimusMessage::NewReaction(data, _) => {
                let core = self.network.core.clone();
                let data = data.clone();
                tokio::spawn(async move {
                    // INVARIANT: Use zero-copy rkyv-validated ingress as per SPEC.
                    if let Err(e) = core.push_bytes(&data).await {
                        log::debug!("Gossip: local reaction ingress failed: {}", e);
                    }
                });
            }
            PrimusMessage::NewCrystal(data, _) => {
                let core = self.network.core.clone();
                let data = data.clone();
                tokio::spawn(async move {
                    if let Err(e) = core.on_crystal(data).await {
                        log::debug!("Gossip: local crystal processing failed: {}", e);
                    }
                });
            }
            _ => {}
        }

        // ── Propagation ───────────────────────────────────────────────────────
        let peers = self.network.dht.get_peer_list().await;

        for peer_addr in peers {
            if source_addr.as_ref() == Some(&peer_addr) {
                continue; // Don't echo back to sender
            }

            let next_ttl = if should_decay {
                match current_ttl.checked_sub(1) {
                    Some(0) | None => continue,
                    Some(t) => t,
                }
            } else {
                current_ttl
            };

            let msg_to_send = match &message {
                PrimusMessage::NewReaction(d, _) => PrimusMessage::NewReaction(d.clone(), next_ttl),
                PrimusMessage::NewCrystal(d, _) => PrimusMessage::NewCrystal(d.clone(), next_ttl),
                PrimusMessage::Sync(s) => PrimusMessage::Sync(s.clone()),
                _ => continue,
            };

            let net = self.network.clone();
            let addr = peer_addr.clone();
            tokio::spawn(async move {
                if let Err(e) = net.send_to_peer(&addr, msg_to_send).await {
                    log::debug!("Gossip: send to {} failed: {}", addr, e);
                }
            });
        }
    }

    /// Compute a stable SHA3-256 ID for deduplication.
    fn message_id(&self, msg: &PrimusMessage) -> [u8; 32] {
        let mut hasher = Sha3_256::new();
        match bincode::serialize(msg) {
            Ok(bytes) => hasher.update(&bytes),
            Err(_) => hasher.update(format!("{:?}", msg).as_bytes()),
        }
        hasher.finalize().into()
    }
}
