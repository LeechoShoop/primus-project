// primus-net-opt/src/network.rs — P2P Network Protocol
//
// Refactored to fix data loss in TCP framing and Send bounds.

use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use primus_types::{PrimusNR, SignedReaction, GalacticStatus, SyncMessage, MerkleProof};
use crate::dht::PrimusDHT;
use crate::gossip::GossipService;

const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024; // 16 MiB

#[async_trait::async_trait]
pub trait CoreHandle: Send + Sync + 'static {
    async fn on_reaction(&self, rx: SignedReaction) -> Result<()>;
    async fn on_crystal(&self, crystal_bytes: Vec<u8>) -> Result<()>;
    async fn local_state(&self) -> (u64, f32, f32);
    async fn get_crystal_bytes(&self, index: u64) -> Option<Vec<u8>>;
    async fn set_sync_target(&self, height: u64);
    async fn is_syncing(&self) -> bool;
    async fn finish_sync(&self);
    async fn get_atom_state(&self, addr: Vec<u8>) -> Result<(u64, u64, [u8; 32], String)>;
    async fn push_bytes(&self, bytes: &[u8]) -> Result<()>;
    async fn on_get_proof(&self, addr: Vec<u8>) -> Result<MerkleProof>;
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum PrimusMessage {
    Ping,
    Pong,
    Handshake {
        version: String,
        node_id: String,
        chain_height: u64,
        total_energy: f32,
        listener_port: u16,
    },
    GetPeers,
    PeersResponse(Vec<String>),
    NewReaction(Vec<u8>, u8), // data, ttl
    NewCrystal(Vec<u8>, u8),  // data, ttl
    GetCrystal(u64),
    CrystalResponse(Vec<u8>),
    Sync(SyncMessage),
    FetchState { address: Vec<u8> },
    StateResponse {
        mass: u64,
        nonce: u64,
        last_hash: [u8; 32],
        element: String,
    },
    SubmitReaction { reaction_bytes: Vec<u8> },
    ReactionAck { reaction_hash: [u8; 32] },
    NodeError { reason: String },
    GetProof { address: Vec<u8> },
    ProofResponse(MerkleProof),
}

pub struct PrimusNetwork<H: CoreHandle> {
    pub port: u16,
    pub core: Arc<H>,
    pub dht: Arc<PrimusDHT>,
    pub gossip: Option<Arc<GossipService<H>>>,
    pub quic_sessions: Arc<dashmap::DashMap<std::net::SocketAddr, Arc<crate::server::PeerSession>>>,
    /// Shared frame drop counter.
    /// Same Arc as PrimusServer.frame_drops and IpcServer counter.
    /// Injected via new() — do NOT create a new Arc here.
    pub frame_drops: Arc<AtomicU64>,
    #[allow(clippy::type_complexity)]
    pub tcp_cache: Arc<dashmap::DashMap<String, Arc<Mutex<Framed<TcpStream, LengthDelimitedCodec>>>>>,
}

impl<H: CoreHandle> Clone for PrimusNetwork<H> {
    fn clone(&self) -> Self {
        Self {
            port: self.port,
            core: self.core.clone(),
            dht: self.dht.clone(),
            gossip: self.gossip.clone(),
            quic_sessions: self.quic_sessions.clone(),
            frame_drops: self.frame_drops.clone(),
            tcp_cache: self.tcp_cache.clone(),
        }
    }
}

impl<H: CoreHandle> PrimusNetwork<H> {
    pub fn new(port: u16, core: Arc<H>, dht: Arc<PrimusDHT>, frame_drops: Arc<AtomicU64>) -> Self {
        Self {
            port,
            core,
            dht,
            gossip: None,
            quic_sessions: Arc::new(dashmap::DashMap::new()),
            frame_drops,
            tcp_cache: Arc::new(dashmap::DashMap::new()),
        }
    }

    pub fn set_gossip(&mut self, gossip: Arc<GossipService<H>>) {
        self.gossip = Some(gossip);
    }

    pub async fn start_listener(&self) -> Result<()> {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;
        log::info!("Network: TCP listener active on {}", addr);
        loop {
            let (socket, _) = listener.accept().await?;
            let net = self.clone();
            tokio::spawn(async move {
                let framed = Framed::new(socket, LengthDelimitedCodec::builder()
                    .max_frame_length(MAX_FRAME_BYTES as usize)
                    .new_codec());
                if let Err(e) = handle_peer_logic(net, framed).await {
                    log::warn!("Peer session error: {}", e);
                }
            });
        }
    }

    pub async fn connect_to_peer(&self, target_addr: &str) -> Result<()> {
        if target_addr.ends_with(&format!(":{}", self.port)) { return Ok(()); }
        if self.dht.get_peer_list().await.contains(&target_addr.to_string()) { return Ok(()); }
        log::info!("Connecting to peer: {}", target_addr);
        let stream = TcpStream::connect(target_addr).await?;
        self.dht.register_peer_addr(target_addr.to_string()).await;

        let mut framed = Framed::new(stream, LengthDelimitedCodec::builder()
            .max_frame_length(MAX_FRAME_BYTES as usize)
            .new_codec());

        let (height, energy, cum_energy) = self.core.local_state().await;
        let handshake = PrimusMessage::Handshake {
            version: "0.1.0".into(),
            node_id: format!("Node-{}", self.port),
            chain_height: height,
            total_energy: energy,
            listener_port: self.port,
        };
        framed.send(bincode::serialize(&handshake)?.into()).await?;

        let sync_msg = PrimusMessage::Sync(SyncMessage::Handshake(GalacticStatus::from_engine(
            height, energy, cum_energy,
        )));
        framed.send(bincode::serialize(&sync_msg)?.into()).await?;
        framed.send(bincode::serialize(&PrimusMessage::GetPeers)?.into()).await?;

        let net = self.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_peer_logic(net, framed).await {
                log::warn!("Outbound peer session error: {}", e);
            }
        });
        Ok(())
    }

    pub async fn send_to_peer(&self, target_addr: &str, message: PrimusMessage) -> Result<()> {
        let stream_mutex = if let Some(cached) = self.tcp_cache.get(target_addr) {
            cached.value().clone()
        } else {
            let stream = TcpStream::connect(target_addr).await?;
            let framed = Framed::new(stream, LengthDelimitedCodec::builder()
                .max_frame_length(MAX_FRAME_BYTES as usize)
                .new_codec());
            let mutex = Arc::new(Mutex::new(framed));
            self.tcp_cache.insert(target_addr.to_string(), mutex.clone());
            mutex
        };

        let mut framed = stream_mutex.lock().await;
        let bytes = bincode::serialize(&message)?;
        if let Err(e) = framed.send(bytes.into()).await {
            // Evict the broken socket so the next send_to_peer reconnects cleanly.
            // Without this, subsequent writes (e.g. later Crystal chunks) go into
            // a half-closed socket, leaving a dangling length prefix in the peer's
            // LengthDelimitedCodec buffer → "Frame read error: bytes remaining on stream".
            drop(framed);
            self.tcp_cache.remove(target_addr);
            return Err(anyhow!("TCP send failed: {}", e));
        }
        Ok(())
    }

    pub async fn broadcast_message(&self, msg: PrimusMessage) -> Result<()> {
        // Use get_peer_list() — this merges the Kademlia NR table AND the flat
        // addr_peers bootstrap list (populated by register_peer_addr() on handshake).
        // get_all_records() only returns NR-verified Kademlia peers, which are
        // absent during normal bootstrap; peers from handshake only live in addr_peers.
        // Additionally, PrimusNR.addr() returns the QUIC port (my_port+1), not the
        // TCP listener port — get_peer_list() returns the correct TCP strings directly.
        for addr in self.dht.get_peer_list().await {
            let (msg_c, net_c) = (msg.clone(), self.clone());
            tokio::spawn(async move {
                let _ = net_c.send_to_peer(&addr, msg_c).await;
            });
        }
        Ok(())
    }

    pub async fn run_discovery_loop(&self) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            for peer in self.dht.get_peer_list().await {
                let _ = self.send_to_peer(&peer, PrimusMessage::GetPeers).await;
            }
        }
    }
}

// FIX: Use BoxFuture to explicitly satisfy Send bounds for tokio::spawn.
fn handle_peer_logic<H: CoreHandle>(net: PrimusNetwork<H>, mut framed: Framed<TcpStream, LengthDelimitedCodec>) -> BoxFuture<'static, Result<()>> {
    Box::pin(async move {
        let peer_ip = framed.get_ref().peer_addr()?.ip().to_string();
        let semaphore = Arc::new(Semaphore::new(50));

        while let Some(frame) = framed.next().await {
            let bytes = frame.map_err(|e| anyhow!("Frame read error: {}", e))?;
            let message: PrimusMessage = bincode::deserialize(&bytes)?;

            match message {
                PrimusMessage::Ping => {
                    framed.send(bincode::serialize(&PrimusMessage::Pong)?.into()).await?;
                }
                PrimusMessage::GetPeers => {
                    let peers = net.dht.get_peer_list().await;
                    framed.send(bincode::serialize(&PrimusMessage::PeersResponse(peers))?.into()).await?;
                }
                PrimusMessage::PeersResponse(peers) => {
                    for peer in peers {
                        let net_c = net.clone();
                        tokio::spawn(async move { let _ = net_c.connect_to_peer(&peer).await; });
                    }
                }
                PrimusMessage::Handshake { node_id, chain_height, total_energy, listener_port, .. } => {
                    let stable_addr = format!("{}:{}", peer_ip, listener_port);
                    log::info!("Handshake from {} ({}): height={} energy={:.2}", node_id, stable_addr, chain_height, total_energy);
                    net.dht.register_peer_addr(stable_addr.clone()).await;
                    let (h, e, cum) = net.core.local_state().await;
                    if total_energy > e || chain_height > h {
                        if chain_height > h + 5 { net.core.set_sync_target(chain_height).await; }
                        let resp1 = PrimusMessage::Sync(SyncMessage::Handshake(GalacticStatus::from_engine(h, e, cum)));
                        framed.send(bincode::serialize(&resp1)?.into()).await?;
                        let resp2 = PrimusMessage::Sync(SyncMessage::RequestCrystals { start: h + 1, end: chain_height });
                        framed.send(bincode::serialize(&resp2)?.into()).await?;
                    }
                }
                PrimusMessage::NewReaction(ref data, _) => {
                    let (net_c, d, m, p) = (net.clone(), data.clone(), message.clone(), peer_ip.clone());
                    let sem = semaphore.clone();
                    tokio::spawn(async move {
                        let _permit = sem.acquire().await;
                        if let Err(e) = net_c.core.push_bytes(&d).await {
                            log::warn!("Shield blocked reaction from {}: {}", p, e);
                        } else if let Some(ref g) = net_c.gossip { g.spread(m, Some(p)).await; }
                    });
                }
                PrimusMessage::NewCrystal(ref data, _) => {
                    let (net_c, d, m, p) = (net.clone(), data.clone(), message.clone(), peer_ip.clone());
                    let sem = semaphore.clone();
                    tokio::spawn(async move {
                        let _permit = sem.acquire().await;
                        if let Ok(()) = net_c.core.on_crystal(d).await
                            && let Some(ref g) = net_c.gossip { g.spread(m, Some(p)).await; }
                    });
                }
                PrimusMessage::GetCrystal(idx) => {
                    if let Some(b) = net.core.get_crystal_bytes(idx).await {
                        framed.send(bincode::serialize(&PrimusMessage::CrystalResponse(b))?.into()).await?;
                    }
                }
                PrimusMessage::CrystalResponse(data) => {
                    let _ = net.core.on_crystal(data).await;
                }
                PrimusMessage::Sync(sync_msg) => match sync_msg {
                    SyncMessage::Handshake(their_status) => {
                        let (h, e, cum) = net.core.local_state().await;
                        let our_status = GalacticStatus::from_engine(h, e, cum);
                        if their_status.is_more_dominant_than(&our_status) {
                            net.core.set_sync_target(their_status.crystal_index).await;
                            let resp = PrimusMessage::Sync(SyncMessage::RequestCrystals { start: h + 1, end: their_status.crystal_index });
                            framed.send(bincode::serialize(&resp)?.into()).await?;
                        }
                    }
                    SyncMessage::RequestCrystals { start, end } => {
                        for idx in start..=end {
                            if let Some(b) = net.core.get_crystal_bytes(idx).await {
                                framed.send(bincode::serialize(&PrimusMessage::CrystalResponse(b))?.into()).await?;
                            }
                        }
                    }
                    SyncMessage::InventoryResponse(crystals) => {
                        for b in crystals {
                            let _ = net.core.on_crystal(b).await;
                        }
                        if !net.core.is_syncing().await { net.core.finish_sync().await; }
                    }
                },
                PrimusMessage::FetchState { address } => {
                    let resp = match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        net.core.get_atom_state(address),
                    ).await {
                        Ok(Ok((mass, nonce, last_hash, element))) =>
                            PrimusMessage::StateResponse { mass, nonce, last_hash, element },
                        Ok(Err(e)) =>
                            PrimusMessage::NodeError { reason: e.to_string() },
                        Err(_) =>
                            PrimusMessage::NodeError { reason: "FetchState timed out".to_string() },
                    };
                    framed.send(bincode::serialize(&resp)?.into()).await?;
                }
                PrimusMessage::SubmitReaction { reaction_bytes } => {
                    // GravityShield pre-filter — same invariant as gossip path (SPEC §2).
                    // All reaction ingress must pass structural validation before core.
                    use crate::gravity_shield::GravityShield;
                    let shield = GravityShield::new();
                    let resp = match tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        async {
                            shield.filter_bytes(&reaction_bytes)
                                .inspect_err(|e| {
                                    net.frame_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                })?;
                            net.core.push_bytes(&reaction_bytes).await
                        },
                    ).await {
                        Ok(Ok(())) => PrimusMessage::ReactionAck {
                            reaction_hash: *blake3::hash(&reaction_bytes).as_bytes(),
                        },
                        Ok(Err(e)) => PrimusMessage::NodeError { reason: e.to_string() },
                        Err(_) => PrimusMessage::NodeError { reason: "SubmitReaction timed out".to_string() },
                    };
                    framed.send(bincode::serialize(&resp)?.into()).await?;
                }
                PrimusMessage::GetProof { address } => {
                    let resp = match tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        net.core.on_get_proof(address),
                    ).await {
                        Ok(Ok(proof)) => PrimusMessage::ProofResponse(proof),
                        Ok(Err(e))    => PrimusMessage::NodeError { reason: e.to_string() },
                        Err(_)        => PrimusMessage::NodeError { reason: "GetProof timed out".to_string() },
                    };
                    framed.send(bincode::serialize(&resp)?.into()).await?;
                }
                _ => {}
            }
        }
        net.dht.remove_peer_addr(&peer_ip).await;
        net.tcp_cache.remove(&peer_ip);
        log::info!("Connection closed by peer {}", peer_ip);
        Ok(())
    })
}

#[async_trait::async_trait]
impl<H: CoreHandle> crate::dht::NodePinger for PrimusNetwork<H> {
    async fn ping(&self, nr: &PrimusNR) -> bool {
        let addr = nr.addr();
        let sessions = self.quic_sessions.clone();
        let net = self.clone();

        tokio::time::timeout(std::time::Duration::from_secs(5), async move {
            if let Some(session) = sessions.get(&addr) {
                let msg = PrimusMessage::Ping;
                if let Ok(payload) = bincode::serialize(&msg) {
                    return session.send_gossip(&payload).await.is_ok();
                }
            }
            net.send_to_peer(&addr.to_string(), PrimusMessage::Ping).await.is_ok()
        }).await.unwrap_or(false)
    }
}
