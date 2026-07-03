// primus-core/src/main.rs — Obsidian Nexus Node

// ── Core modules (только консенсус/state/storage) ─────────────────────────────
pub(crate) mod atom;
pub(crate) mod chamber;
pub(crate) mod crypto;
pub(crate) mod crypto_shim;
pub(crate) mod crystal;
pub(crate) mod engine;
pub(crate) mod galactic_sync;
pub(crate) mod gravity;
pub(crate) mod kinetic;
pub(crate) mod processor;
pub(crate) mod pvm;
pub(crate) mod setup;
pub(crate) mod state;
pub(crate) mod storage;
pub(crate) mod framing;
pub(crate) mod physics_shim;
pub use primus_storage::mempool_v2;

pub mod bridge {
    use crate::processor::{PrimusProcessor, SharedEngine};
    use crate::mempool_v2::SectoralMempool;
    use primus_net_opt::gravity_shield::GravityShield;
    use primus_net_opt::network::{PrimusMessage, PrimusNetwork};
    use crate::kinetic::SignedReaction;
    use anyhow::{anyhow, Context, Result};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use primus_net_opt::server::{KademliaHandler, MempoolIngress};
    use primus_net_opt::KademliaMsg;
    use primus_types::MerkleProof;
    use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
    use futures::{SinkExt, StreamExt};
    use std::sync::atomic::AtomicU64;

    pub struct CoreHandleImpl {
        pub engine: SharedEngine,
        pub mempool: Arc<Mutex<SectoralMempool>>,
        pub shield: Arc<GravityShield>,
        pub frame_drops: Arc<AtomicU64>,
        pub network: Arc<Mutex<Option<PrimusNetwork<CoreHandleImpl>>>>,
        pub chunk_reassembler: Arc<tokio::sync::Mutex<crate::framing::ChunkReassembler>>,
    }

    impl CoreHandleImpl {
        pub fn new(engine: SharedEngine, mempool: Arc<Mutex<SectoralMempool>>, shield: Arc<GravityShield>, frame_drops: Arc<AtomicU64>) -> Self {
            Self {
                engine,
                mempool,
                shield,
                frame_drops,
                network: Arc::new(Mutex::new(None)),
                chunk_reassembler: Arc::new(tokio::sync::Mutex::new(crate::framing::ChunkReassembler::new())),
            }
        }

        pub async fn set_network(&self, net: PrimusNetwork<CoreHandleImpl>) {
            *self.network.lock().await = Some(net);
        }
    }

    #[async_trait::async_trait]
    impl primus_net_opt::network::CoreHandle for CoreHandleImpl {
        async fn on_reaction(&self, rx: SignedReaction) -> Result<()> {
            PrimusProcessor::process_network_reaction(self.engine.clone(), rx).await
        }

        async fn on_crystal(&self, crystal_bytes: Vec<u8>) -> Result<()> {
            // ── CHUNK REASSEMBLY — attempt before treating as direct Crystal ─────────
            let mut extracted_envelope = None;
            if let Ok(envelope) = rkyv::from_bytes::<crate::framing::ChunkEnvelope>(&crystal_bytes) {
                extracted_envelope = Some(envelope);
            }

            if let Some(envelope) = extracted_envelope {
                let mut reassembler = self.chunk_reassembler.lock().await;
                match reassembler.feed(envelope) {
                    Ok(Some(full_message)) => {
                        // All chunks received — process as complete message
                        return Box::pin(self.on_crystal(full_message)).await;
                    }
                    Ok(None) => {
                        // More chunks pending — this is normal, not an error
                        return Ok(());
                    }
                    Err(e) => {
                        log::warn!("Chunk reassembly error — dropping stream: {}", e);
                        self.frame_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return Ok(());
                    }
                }
            }

            let crystal = bincode::deserialize::<crate::crystal::Crystal>(&crystal_bytes)
                .map_err(|e| anyhow!("Crystal deserialization failed: {}", e))?;
            PrimusProcessor::process_network_crystal(self.engine.clone(), crystal).await
        }

        async fn get_crystal_bytes(&self, index: u64) -> Option<Vec<u8>> {
            self.engine
                .lock()
                .await
                .storage
                .get_crystal(index)
                .ok()
                .flatten()
                .and_then(|c| bincode::serialize(&c).ok())
        }

        async fn local_state(&self) -> (u64, f32, f32) {
            let engine = self.engine.lock().await;
            let height = engine.state.current_crystal_index;
            let entropy = engine.state.global_metrics.entropy;
            let cum_energy = engine
                .storage
                .get_crystal_latest()
                .ok()
                .flatten()
                .map(|c| c.cumulative_energy)
                .unwrap_or(0.0);
            (height, entropy, cum_energy)
        }

        async fn is_syncing(&self) -> bool {
            self.engine.lock().await.is_syncing
        }

        async fn set_sync_target(&self, height: u64) {
            let mut engine = self.engine.lock().await;
            engine.is_syncing = true;
            engine.sync_target = height;
        }

        async fn finish_sync(&self) {
            let mut engine = self.engine.lock().await;
            if engine.is_syncing {
                engine.is_syncing = false;
                log::info!(
                    "Sync complete at height {}.",
                    engine.state.current_crystal_index
                );
            }
        }


        async fn push_bytes(&self, bytes: &[u8]) -> Result<()> {
            // ── Size guard — defense-in-depth before rkyv allocation ─────────────
            // LengthDelimitedCodec in net-opt limits TCP frames, but push_bytes
            // can be called from multiple paths. Reject oversized payloads before
            // any allocation. Gemini audit G4 fix.
            const MAX_INGRESS_BYTES: usize = crate::framing::APP_MAX_MESSAGE; // 16 MiB
            if bytes.len() > MAX_INGRESS_BYTES {
                self.frame_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Err(anyhow!(
                    "push_bytes: payload too large ({} bytes, max {} bytes)",
                    bytes.len(),
                    MAX_INGRESS_BYTES,
                ));
            }
            // ── rest of the function unchanged ───────────────────────────────────
            let mut is_direct_reaction = false;
            if rkyv::check_archived_root::<SignedReaction>(bytes).is_ok() {
                is_direct_reaction = true;
            }

            let mut extracted_envelope = None;
            if !is_direct_reaction
                && let Ok(envelope) = rkyv::from_bytes::<crate::framing::ChunkEnvelope>(bytes) {
                    extracted_envelope = Some(envelope);
                }
            let mut full_message_buf = None;
            if let Some(envelope) = extracted_envelope {
                let full_message_opt = {
                    let mut reassembler = self.chunk_reassembler.lock().await;
                    match reassembler.feed(envelope) {
                        Ok(Some(full_message)) => Some(full_message),
                        Ok(None) => return Ok(()),
                        Err(e) => {
                            log::warn!("Chunk reassembly error — dropping stream: {}", e);
                            self.frame_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            return Ok(());
                        }
                    }
                };

                if let Some(full_message) = full_message_opt {
                    full_message_buf = Some(full_message);
                } else {
                    return Ok(());
                }
            }

            let bytes_to_process = if let Some(ref buf) = full_message_buf {
                buf.as_slice()
            } else {
                bytes
            };

            // ── SECURITY: rkyv structural check at core ingress boundary ─────────────
            // primus-net-opt (frozen) does not call rkyv::check_archive before passing
            // bytes to CoreHandleImpl. We enforce it here as defense-in-depth to prevent
            // OOB access or malicious memory layouts reaching the engine.
            // AUDIT_REPORT.md: fixes DIV — rkyv bypass
            // primus-core SPECIFICATION.md §4 (Zero-Copy Validation)
            // ─────────────────────────────────────────────────────────────────────────
            if let Err(e) = rkyv::check_archived_root::<SignedReaction>(bytes_to_process) {
                log::warn!("CoreHandleImpl: rkyv check failed — dropping (security boundary): {}", e);
                self.frame_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Err(anyhow!("MalformedFrame: {}", e));
            }

            let rx_archived = SignedReaction::from_bytes_zero_copy(bytes_to_process)
                .map_err(|e| anyhow!("rkyv zero-copy check failed: {:?}", e))?;

            let rx: SignedReaction = rkyv::Deserialize::<SignedReaction, _>::deserialize(rx_archived, &mut rkyv::Infallible)
                .map_err(|e| anyhow!("rkyv deserialization failed: {:?}", e))?;

            // ── GravityShield Layer 5 — Phantom Sender Check ──────────────────────────
            // SPECIFICATION.md §4: reactions from unknown senders with no on-chain
            // balance must be dropped before entering the mempool.
            // primus-net-opt (frozen) does not implement this layer.
            // CoreHandleImpl enforces it as the final ingress gate.
            // AUDIT_REPORT.md: fixes DIV — Missing GravityShield Layer 5
            // ──────────────────────────────────────────────────────────────────────────
            let sender_pubkey = rx.sender.public_key.clone();
            let is_architect = {
                let engine = self.engine.lock().await;
                sender_pubkey == engine.architect_pk
            };

            if !is_architect {
                let atom = {
                    let engine = self.engine.lock().await;
                    engine.storage.get_atom(&sender_pubkey)
                };

                let has_balance = match atom {
                    Ok(Some(ref a)) => a.mass > 0,
                    Ok(None) => false,  // atom does not exist — phantom sender
                    Err(e) => {
                        log::warn!("storage error in L5 check — dropping: {}", e);
                        false
                    }
                };

                if !has_balance {
                    log::debug!(
                        "GravityShield L5: phantom/zero-balance sender — dropping"
                    );
                    self.frame_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // Silent drop — do not return error (prevents amplification attacks
                    // where the attacker probes for error responses to map the network)
                    return Ok(());
                }
            }

            let is_new = self.mempool.lock().await.push(rx.clone()).map_err(|e| anyhow!(e))?;

            if is_new {
                let mp_size: usize = self.mempool.lock().await.sectors.values().map(|(d, _)| d.len()).sum();
                log::info!("Mempool: transaction added (hash: {:?}, size: {})", rx.reaction_hash, mp_size);
                if let Some(net) = self.network.lock().await.as_ref() {
                    let msg = PrimusMessage::NewReaction(bytes_to_process.to_vec(), 8);
                    let _ = net.broadcast_message(msg).await;
                }
            }
            Ok(())
        }

        async fn on_get_proof(&self, addr: Vec<u8>) -> Result<MerkleProof> {
            // get_proof() is async internally (RwLock::read().await).
            // Drop the MutexGuard before the .await: tokio::MutexGuard is !Send,
            // and async_trait boxes the future as Send.
            // clone_db() returns a sled::Db (Arc-backed) — cheap, shares on-disk state.
            let db = {
                let engine = self.engine.lock().await;
                engine.storage.clone_db()
            };
            let temp_storage = crate::storage::PrimusStorage::open_readonly(db)?;
            temp_storage.get_proof(&addr).await
                .map_err(|e| anyhow!("Failed to generate proof: {}", e))
        }

        async fn get_atom_state(&self, addr: Vec<u8>) -> Result<(u64, u64, [u8; 32], String)> {
            // All operations are sync — safe to hold the guard throughout.
            let engine = self.engine.lock().await;
            let atom = engine.state.get_atom(&addr)
                .ok_or_else(|| anyhow!("Atom not found"))?;
            Ok((atom.mass, atom.nonce, atom.last_reaction_hash, format!("{:?}", atom.element)))
        }
    }

    #[async_trait::async_trait]
    impl MempoolIngress for CoreHandleImpl {
        async fn push_bytes(&self, bytes: &[u8]) -> anyhow::Result<bool> {
            // Size guard — same as CoreHandle::push_bytes
            if bytes.len() > crate::framing::APP_MAX_MESSAGE {
                return Err(anyhow!("MempoolIngress::push_bytes: payload too large"));
            }
            let mempool = self.mempool.clone();
            let network = self.network.clone();
            let bytes_vec = bytes.to_vec();

            let rx_archived = SignedReaction::from_bytes_zero_copy(&bytes_vec)
                .map_err(|e| anyhow!("rkyv zero-copy check failed: {:?}", e))?;

            let rx: SignedReaction = rkyv::Deserialize::<SignedReaction, _>::deserialize(rx_archived, &mut rkyv::Infallible)
                .map_err(|e| anyhow!("rkyv deserialization failed: {:?}", e))?;

            let is_new = mempool.lock().await.push(rx.clone()).map_err(|e| anyhow!(e))?;

            if is_new
                && let Some(net) = network.lock().await.as_ref() {
                    let msg = PrimusMessage::NewReaction(bytes_vec, 8);
                    let _ = net.broadcast_message(msg).await;
                }
            Ok(is_new)
        }
    }

    pub struct KademliaBridge {
        pub dht: Arc<primus_net_opt::dht::PrimusDHT>,
    }

    #[async_trait::async_trait]
    impl KademliaHandler for KademliaBridge {
        fn start_maintenance(self: Arc<Self>) {}

        async fn handle_rpc(
            &self,
            send: quinn::SendStream,
            recv: quinn::RecvStream,
        ) -> Result<()> {
            let dht = self.dht.clone();
            {
                let mut reader = FramedRead::new(recv, LengthDelimitedCodec::builder()
                    .max_frame_length(16 * 1024 * 1024)
                    .new_codec());
                let mut writer = FramedWrite::new(send, LengthDelimitedCodec::builder()
                    .max_frame_length(16 * 1024 * 1024)
                    .new_codec());

                while let Some(res) = reader.next().await {
                    let bytes = res.context("Kademlia RPC read failed")?;
                    let req: KademliaMsg = bincode::deserialize(&bytes).context("Kademlia RPC deserialization failed")?;

                    match req {
                        KademliaMsg::FindNodeRequest(target) => {
                            let closest = dht.find_closest(&target, 20).await;
                            let resp = KademliaMsg::FindNodeResponse(closest);
                            let resp_bytes = bincode::serialize(&resp)?;
                            writer.send(resp_bytes.into()).await.context("Kademlia RPC send failed")?;
                        }
                        _ => {
                            log::debug!("Kademlia RPC: received unsupported message");
                        }
                    }
                }
                Ok(())
            }
        }
    }

    #[cfg(test)]
    mod gravity_shield_l5_tests {
        use super::*;
        use crate::processor::PrimusProcessor;
        use std::sync::atomic::AtomicU64;
        use primus_types::atom::{Atom, Element};
        use primus_types::{SignedReaction, Payload};

        async fn setup_core() -> (Arc<CoreHandleImpl>, Vec<u8>) {
            // Setup a temporary processor
            let architect_pk = vec![1u8; 2592];
            let architect_pk_clone = architect_pk.clone();
            let temp_dir = std::env::temp_dir().join(format!("primus_test_{}", rand::random::<u32>()));
            let processor = PrimusProcessor::new(temp_dir.to_str().unwrap(), architect_pk_clone).await.unwrap();

            let engine = Arc::new(Mutex::new(processor.engine));
            let mempool = Arc::new(Mutex::new(SectoralMempool::new(&engine.lock().await.storage.clone_db()).unwrap()));
            let shield = Arc::new(GravityShield::new());
            let drops = Arc::new(AtomicU64::new(0));

            let handle = Arc::new(CoreHandleImpl::new(engine, mempool, shield, drops));
            (handle, architect_pk)
        }

        fn create_dummy_reaction(sender_pk: Vec<u8>) -> Vec<u8> {
            let sender = Atom::sender_snapshot(sender_pk, Element::Hydrogen, 10, [0u8; 32], 1, 0, 0);
            let receiver = Atom::new_receiver(vec![0u8; 2592]);
            let mut signature = vec![0u8; 4627];
            rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut signature);
            let mut rx = SignedReaction {
                sender,
                receiver,
                reaction_hash: [0u8; 32],
                energy: 10.0,
                timestamp: 0,
                signature,
                payload: Payload::Generic,
            };
            rx.reaction_hash = rx.compute_reaction_hash();
            rkyv::to_bytes::<_, 256>(&rx).unwrap().to_vec()
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn phantom_sender_check_logic_rejects_zero_mass() {
            use primus_net_opt::network::CoreHandle;
            let (handle, _) = setup_core().await;

            // Sender with no on-chain balance
            let sender_pk = vec![2u8; 2592];
            let rx_bytes = create_dummy_reaction(sender_pk.clone());

            let initial_drops = handle.frame_drops.load(std::sync::atomic::Ordering::Relaxed);

            // push_bytes should silently drop and increment frame_drops
            let result = CoreHandle::push_bytes(&*handle, &rx_bytes).await;
            assert!(result.is_ok(), "phantom drop should be silent (return Ok)");

            let final_drops = handle.frame_drops.load(std::sync::atomic::Ordering::Relaxed);
            assert_eq!(final_drops, initial_drops + 1, "frame_drops should increment for phantom sender");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn architect_bypasses_phantom_check() {
            use primus_net_opt::network::CoreHandle;
            let (handle, architect_pk) = setup_core().await;

            // Architect with no explicit balance in storage, but should bypass the check
            let rx_bytes = create_dummy_reaction(architect_pk);

            let initial_drops = handle.frame_drops.load(std::sync::atomic::Ordering::Relaxed);

            let result = CoreHandle::push_bytes(&*handle, &rx_bytes).await;
            assert!(result.is_ok());

            let final_drops = handle.frame_drops.load(std::sync::atomic::Ordering::Relaxed);
            assert_eq!(final_drops, initial_drops, "frame_drops should NOT increment for architect");
        }
    }
}
use bridge::*;
use crate::processor::PrimusProcessor;
use crate::pvm::get_galactic_drift;

pub mod ipc;

// ── Сетевой слой — всё из primus-net-opt ─────────────────────────────────────
use primus_net_opt::nat::NatService;
use primus_net_opt::gossip::GossipService;
use primus_net_opt::network::{PrimusMessage, PrimusNetwork};
use primus_net_opt::server::PrimusNetworkServer;

// ── Core internals ────────────────────────────────────────────────────────────
use crate::kinetic::SignedReaction;
use crate::mempool_v2::SectoralMempool;
use primus_net_opt::gravity_shield::GravityShield;

// ── SDK ───────────────────────────────────────────────────────────────────────
use primus_sdk::{AtomElement, TransactionBuilder, Wallet, PROTOCOL_MIN_FEE};

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::sync::Mutex;

// Handlers moved to network_bridge.rs

// =============================================================================
// SDK adapter
// =============================================================================

fn sdk_tx_to_reaction_result(tx: primus_sdk::Transaction) -> anyhow::Result<SignedReaction> {
    let bytes = tx.to_bytes()?;
    bincode::deserialize(&bytes).map_err(|e| {
        anyhow::anyhow!(
            "sdk_tx_to_reaction_result: schema drift — \
         SDK AtomSnapshot и Core Atom рассинхронизированы. \
         Обнови transaction/mod.rs в primus-sdk. Ошибка: {}",
            e
        )
    })
}

// =============================================================================
// CLI
// =============================================================================

#[derive(Parser, Debug)]
#[command(name = "primus-core", about = "Obsidian Nexus — Layer-1 Node")]
struct Args {
    #[arg(short = 'p', long, default_value_t = 9000)]
    port: u16,

    /// QUIC/WebTransport port. Defaults to `port + 10000` (e.g. 9001 → 19001).
    /// Using port+1 caused EADDRINUSE on Windows when running nodes on
    /// consecutive TCP ports, because adjacent nodes' QUIC and TCP ports overlap.
    #[arg(long)]
    quic_port: Option<u16>,

    #[arg(short = 'r', long)]
    peer: Option<String>,

    #[arg(short = 'k', long, default_value = ".secrets/master.wallet")]
    key: String,

    #[arg(long, default_value = ".secrets/master.key")]
    master_key: String,

    #[arg(short = 'o', long, default_value = ".secrets/operator.wallet")]
    operator_key: String,

    #[arg(long)]
    no_mining: bool,
}

// =============================================================================
// ENTRY POINT
// =============================================================================

// Network Adapters moved to network_bridge.rs

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let args = Args::parse();
    let my_port = args.port;

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  🌌  Primus-Project Layer-1 Node");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // =========================================================================
    // STEP 0 — First-run Setup Wizard
    // =========================================================================
    // Runs synchronously on stdin BEFORE any tokio tasks touch stdin.
    // Subsequent runs load config from .primus_config.toml automatically.
    let node_config = if let Some(cfg) = setup::NodeConfig::load() {
        println!("📋 Loaded config from .primus_config.toml");
        if cfg.is_seed {
            println!("   Role: 🌱 Seed Node  |  Public IP: {}",
                cfg.public_ip.map(|ip| ip.to_string()).unwrap_or_else(|| "(auto)".into()));
        } else {
            println!("   Role: 🔗 Regular Peer  |  Seeds: {}", cfg.seed_peers.len());
        }
        cfg
    } else {
        setup::run_wizard(my_port)
    };

    // =========================================================================
    // STEP 1 — Architect Identity
    // =========================================================================
    let wallet_path = std::path::PathBuf::from(&args.key);
    let legacy_key_path = std::path::PathBuf::from(&args.master_key);
    let legacy_mode: bool;
    let architect_wallet: Arc<Wallet>;
    let architect_pk: Vec<u8>;
    let architect_sk: Vec<u8>;

    if wallet_path.exists() {
        println!("🔑 Loading Architect wallet from {:?}…", wallet_path);
        let wp = wallet_path.clone();
        let (w, pk, sk) = std::thread::Builder::new()
            .name("arch-wallet-load".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let w = Wallet::load(&wp).expect("❌ master.wallet corrupt");
                let pk = w.get_public_key_bytes();
                let sk = w.get_secret_key_bytes();
                (w, pk, sk)
            })
            .expect("spawn failed")
            .join()
            .expect("panicked");

        architect_pk = pk;
        architect_sk = sk.to_vec();
        architect_wallet = Arc::new(w);
        legacy_mode = false;
    } else if legacy_key_path.exists() {
        println!("⚠️  master.wallet not found — falling back to legacy master.key.");
        let keys = crate::crypto::Crypto::load_master_key(&legacy_key_path)
            .expect("❌ master.key corrupt");
        architect_pk = keys.pk.clone();
        architect_sk = keys.sk.clone();
        let placeholder = std::thread::Builder::new()
            .name("arch-placeholder".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(|| Wallet::generate(12, 0).expect("placeholder failed"))
            .expect("spawn failed")
            .join()
            .expect("panicked");
        architect_wallet = Arc::new(placeholder);
        legacy_mode = true;
    } else {
        println!("🔑 No Architect key found.");
        println!("   Enter your 24-word BIP-39 seed phrase, or press Enter to generate:");
        let mut phrase_input = String::new();
        std::io::stdin()
            .read_line(&mut phrase_input)
            .expect("stdin failed");
        let phrase_input = phrase_input.trim().to_string();

        struct WalletData {
            pk: Vec<u8>,
            sk: Vec<u8>,
            mnemonic: String,
            wallet: Wallet,
        }
        let wp = wallet_path.clone();
        let wd: WalletData = if phrase_input.is_empty() {
            println!("🆕 Generating new 24-word Architect wallet…");
            std::thread::Builder::new()
                .name("arch-keygen".into())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || {
                    let w = Wallet::generate(24, 0).expect("keygen failed");
                    w.save(&wp).expect("save failed");
                    WalletData {
                        pk: w.get_public_key_bytes(),
                        sk: w.get_secret_key_bytes().to_vec(),
                        mnemonic: w.get_mnemonic().to_string(),
                        wallet: w,
                    }
                })
                .expect("spawn failed")
                .join()
                .expect("panicked")
        } else {
            std::thread::Builder::new()
                .name("arch-restore".into())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || {
                    let w = Wallet::from_mnemonic(&phrase_input, 0).expect("invalid mnemonic");
                    w.save(&wp).expect("save failed");
                    WalletData {
                        pk: w.get_public_key_bytes(),
                        sk: w.get_secret_key_bytes().to_vec(),
                        mnemonic: w.get_mnemonic().to_string(),
                        wallet: w,
                    }
                })
                .expect("spawn failed")
                .join()
                .expect("panicked")
        };

        println!("  ✅  master.wallet создан в {:?}", wallet_path);
        println!("  ⚠️  СОХРАНИ МНЕМОНИКУ (бумага, офлайн):");
        println!("  {}", wd.mnemonic);

        architect_pk = wd.pk;
        architect_sk = wd.sk;
        architect_wallet = Arc::new(wd.wallet);
        legacy_mode = false;
    }

    println!(
        "👑 Architect: {}… ({})",
        &architect_wallet.address[..16],
        if legacy_mode {
            "legacy master.key"
        } else {
            "master.wallet"
        }
    );

    // =========================================================================
    // STEP 1.1 — Operator Identity
    // =========================================================================
    let op_wallet_path = std::path::PathBuf::from(&args.operator_key);
    let operator_wallet: Wallet = if op_wallet_path.exists() {
        println!("🛠️  Loading Operator wallet from {:?}…", op_wallet_path);
        let owp = op_wallet_path.clone();
        std::thread::Builder::new()
            .name("op-wallet-load".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || Wallet::load(&owp))
            .expect("spawn failed")
            .join()
            .expect("panicked")
            .expect("❌ operator.wallet corrupt. Re-run: cargo run --example keygen")
    } else {
        eprintln!("❌ operator.wallet not found at {:?}", op_wallet_path);
        eprintln!("   Generate: cargo run --example keygen");
        std::process::exit(1);
    };

    println!(
        "🛠️  Operator: {} ({})",
        operator_wallet.address,
        operator_wallet.get_short_address()
    );

    // ── UPnP ──────────────────────────────────────────────────────────────────
    let external_ip = NatService::open_world(my_port).await.ok();

    // =========================================================================
    // STEP 2 — Engine
    // =========================================================================
    let data_path = format!("./data/node_{}", my_port);
    let processor = PrimusProcessor::new(&data_path, architect_pk.clone()).await?;
    let mut engine = processor.engine;
    engine.try_auto_genesis().await?;
    let shared_engine = Arc::new(Mutex::new(engine));

    // =========================================================================
    // STEP 3 — Mempool
    // =========================================================================
    let db_ref = {
        let e = shared_engine.lock().await;
        e.storage.clone_db()
    };
    let sectoral_mempool = Arc::new(Mutex::new(SectoralMempool::new(&db_ref)?));

    // =========================================================================
    // STEP 4 — Build local PrimusNR (self-signed)
    // =========================================================================
    //
    // QUIC port FIX: default offset changed from +1 to +10000.
    //
    // Old formula (my_port + 1) caused os error 10048 (WSAEADDRINUSE) on Windows
    // when running nodes on consecutive TCP ports:
    //   node 9001 → TCP 9001, QUIC 9002
    //   node 9002 → TCP 9002  ← conflicts with node-9001 QUIC (same wildcard addr)
    //
    // New formula (my_port + 10000) keeps a clear gap:
    //   node 9001 → TCP 9001, QUIC 19001
    //   node 9002 → TCP 9002, QUIC 19002   ← no conflict
    //   node 9003 → TCP 9003, QUIC 19003
    //
    // Override with --quic-port <N> if needed (e.g. for firewall / NAT rules).
    let effective_quic_port: u16 = args.quic_port.unwrap_or_else(|| {
        my_port.saturating_add(10000)
    });
    let quic_addr_str = format!("0.0.0.0:{}", effective_quic_port);
    let mut local_addr: std::net::SocketAddr = quic_addr_str.parse().expect("Invalid QUIC address");

    println!("🔌 Ports — TCP: {} | QUIC: {}", my_port, effective_quic_port);

    if let Some(ip) = external_ip {
        local_addr.set_ip(ip);
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut sign_payload = Vec::new();
    sign_payload.extend_from_slice(&architect_pk);
    let addr_ip: u128 = match local_addr.ip() {
        std::net::IpAddr::V4(v4) => u128::from(v4.to_ipv6_mapped()),
        std::net::IpAddr::V6(v6) => u128::from(v6),
    };
    sign_payload.extend_from_slice(&addr_ip.to_be_bytes());
    sign_payload.extend_from_slice(&local_addr.port().to_be_bytes());
    sign_payload.extend_from_slice(&timestamp.to_le_bytes());

    let sign_payload_c = sign_payload.clone();
    let wallet_c = Arc::clone(&architect_wallet);
    let signature = tokio::task::spawn_blocking(move || {
        std::thread::Builder::new()
            .name("nr-sign".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || wallet_c.sign(&sign_payload_c))
            .expect("spawn failed")
            .join()
            .expect("panicked")
    })
        .await
        .expect("spawn_blocking failed");

    let local_nr = primus_types::PrimusNR::from_socket_addr(
        architect_pk.clone(),
        local_addr,
        signature,
        timestamp,
    );

    // =========================================================================
    // STEP 5 — Network (primus-net-opt)
    // =========================================================================

    let shield = Arc::new(GravityShield::new());
    let frame_drops = Arc::clone(&shield.drops);

    // 1. Initial CoreHandleImpl (without network reference yet)
    let core_handle = Arc::new(CoreHandleImpl::new(
        shared_engine.clone(),
        sectoral_mempool.clone(),
        shield.clone(),
        Arc::clone(&frame_drops),
    ));

    // 2. Initialize TCP Network Layer
    let dht = Arc::new(primus_net_opt::dht::PrimusDHT::new(&local_nr));
    let mut server_net = PrimusNetwork::new(
        my_port,
        core_handle.clone(),
        dht,
        Arc::clone(&frame_drops), // shared Arc — same counter as GravityShield and IpcServer
    );
    let gossip = Arc::new(GossipService::new(server_net.clone()));
    server_net.set_gossip(gossip);

    // 3. Initialize QUIC/WebTransport Server with Kademlia bridge
    let quic_nr = local_nr.clone();
    let quic_sk = architect_sk.clone();
    let quic_kad = Arc::new(KademliaBridge { dht: server_net.dht.clone() });

    // CoreHandleImpl also implements MempoolIngress, so we use it for QUIC too
    let quic_server = PrimusNetworkServer::new(
        local_addr,
        core_handle.clone(), // implements MempoolIngress
        quic_kad,
        quic_nr,
        quic_sk,
        node_config.tls_domain.clone().unwrap_or_else(|| "localhost".to_string())
    ).await?;

    // 4. Close the loop: update CoreHandleImpl with network reference for gossip forwarding
    server_net.quic_sessions = quic_server.sessions.clone();
    core_handle.set_network(server_net.clone()).await;

    // Start QUIC Server
    tokio::spawn(async move {
        if let Err(e) = quic_server.run().await {
            log::error!("QUIC server error: {}", e);
        }
    });

    // Background GC for ChunkReassembler — prevents OOM from incomplete chunk streams.
    // Gemini audit finding G3: evict_expired() is only called on new chunk arrival.
    // An attacker sending only first-chunks would grow the HashMap unbounded.
    // This task forces eviction every 30 seconds regardless of traffic.
    {
        let reassembler_gc = Arc::clone(&core_handle.chunk_reassembler);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(crate::framing::CHUNK_STREAM_TTL_SECS)
            );
            loop {
                interval.tick().await;
                let mut r = reassembler_gc.lock().await;
                r.evict_expired();
            }
        });
    }

    // Start IPC Server with real telemetry access
    let ipc_engine = shared_engine.clone();
    let ipc_net = server_net.clone();
    let ipc_pk = architect_pk.clone();
    let ipc_port = my_port;
    tokio::spawn(async move {
        let ipc_server = ipc::IpcServer::new(ipc_pk, ipc_engine, ipc_net);
        match ipc::IpcServer::get_secure_ipc_path(ipc_port) {
            Ok(path) => {
                if let Err(e) = ipc_server.run(&path).await {
                    log::error!("IPC Server Error: {}", e);
                }
            }
            Err(e) => {
                log::error!("Failed to generate secure IPC path: {}", e);
            }
        }
    });

    // TCP listener
    let net_listener = server_net.clone();
    tokio::spawn(async move {
        if let Err(e) = net_listener.start_listener().await {
            log::error!("TCP listener: {}", e);
        }
    });

    // Discovery (UDP broadcast)
    // TODO: update PrimusDiscovery for PrimusNetwork<H> generic
    // keeping commented out until discovery.rs is updated
    // let discovery = primus_net_opt::discovery::PrimusDiscovery::new(my_port, node_config.public_ip.map(|ip| ip.to_string()));
    // tokio::spawn(async move { let _ = discovery.start(net_discovery).await; });

    // Periodic GetPeers
    let net_discovery_loop = server_net.clone();
    tokio::spawn(async move {
        net_discovery_loop.run_discovery_loop().await;
    });

    // Bootstrap: CLI --peer flag OR seed_peers from config
    let bootstrap_peers: Vec<String> = {
        let mut peers = Vec::new();
        if let Some(ref addr) = args.peer {
            peers.push(addr.clone());
        }
        for addr in &node_config.seed_peers {
            peers.push(addr.to_string());
        }
        peers
    };

    if node_config.is_seed {
        println!("🌱 Running as SEED NODE — waiting for incoming peer connections.");
    }

    for addr in &bootstrap_peers {
        println!("🚀 Connecting to bootstrap peer: {}", addr);
        let net_c = server_net.clone();
        let addr_clone = addr.clone();
        tokio::spawn(async move {
            // Brief delay so QUIC server is fully up before we dial out
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            match net_c.connect_to_peer(&addr_clone).await {
                Ok(_) => {
                    println!("✅ Connected to {}. Block sync will start if we are behind.", addr_clone);
                }
                Err(e) => eprintln!("❌ Connection to {} failed: {}", addr_clone, e),
            }
        });
    }

    // Galactic sync heartbeat
    let engine_for_sync = shared_engine.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let engine = engine_for_sync.lock().await;
            let status = crate::galactic_sync::GalacticStatus::from_state(
                engine.state.current_crystal_index,
                engine.state.global_metrics.entropy,
            );
            log::info!(
                "Galactic Sync: height={} entropy={:.2} drift={}",
                status.crystal_index,
                status.current_entropy,
                status.sector_drift
            );
        }
    });

    // =========================================================================
    // MINING LOOP
    // =========================================================================
    let engine_loop = shared_engine.clone();
    let net_loop = server_net.clone();
    let mempool_loop = sectoral_mempool.clone();
    let architect_pk_loop = architect_pk.clone();

    let no_mining = args.no_mining;
    tokio::spawn(async move {
        if no_mining {
            log::info!("Mining: Synthesis loop DISABLED (--no-mining).");
            return;
        }
        log::info!("Mining: synthesis loop started.");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            let mut engine = engine_loop.lock().await;
            if engine.is_syncing {
                continue;
            }

            let drift = get_galactic_drift(engine.state.current_crystal_index);
            let resonant_rxs = {
                let mp = mempool_loop.lock().await;
                let mut rxs = mp.drain_resonant(drift, 50);
                if !rxs.is_empty() {
                    log::debug!("Mempool: drained {} reactions from sector {}", rxs.len(), drift);
                }
                let architect_sector = architect_pk_loop.first().copied().unwrap_or(0);
                if architect_sector != drift && rxs.len() < 50 {
                    let extra = mp.drain_resonant(architect_sector, 50 - rxs.len());
                    if !extra.is_empty() {
                        log::debug!("Mempool: drained {} reactions from architect sector {}", extra.len(), architect_sector);
                        rxs.extend(extra);
                    }
                }
                rxs
            };

            match engine.mine_block(resonant_rxs).await {
                Ok(Some(crystal)) => {
                    if let Ok(serialized) = bincode::serialize(&crystal) {
                        // ── DIRECT TCP SEND — no chunking needed on the TCP path ────────────────
                        // The TCP framing layer (LengthDelimitedCodec, MAX_FRAME_BYTES = 16 MiB)
                        // can carry the full bincode-serialised Crystal in a single frame.
                        // Chunking via framing::chunk_message is only needed for the Noise/QUIC
                        // path (hard 65535-byte limit per noise.write_message call).
                        // Sending each ChunkEnvelope as a separate broadcast_message caused
                        // "Frame read error: bytes remaining on stream" on the receiver when any
                        // chunk was lost mid-stream — now fixed by removing the chunking entirely.
                        // primus-core SPECIFICATION.md §7 / AUDIT_REPORT.md BLK-002 (revised).
                        // ─────────────────────────────────────────────────────────────────────────
                        log::debug!("Sending Crystal #{} ({} bytes) via TCP broadcast",
                            crystal.index, serialized.len());
                        let msg = PrimusMessage::NewCrystal(serialized, 8);
                        let _ = net_loop.broadcast_message(msg).await;
                    }
                    log::info!("Crystal #{} solidified and broadcasted!", crystal.index);
                }
                Ok(None) => {}
                Err(e) => log::error!("Mining error: {}", e),
            }
        }
    });

    // =========================================================================
    // GRACEFUL SHUTDOWN
    // =========================================================================
    let engine_shutdown = shared_engine.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!("\n🛑 Shutting down — flushing state…");
        let mut engine = engine_shutdown.lock().await;
        engine.state.global_metrics.temperature = engine.chamber.temperature;
        engine.state.global_metrics.entropy = engine.chamber.entropy;
        let mut changeset = crate::state::Changeset::new();
        for (pk, atom) in &engine.state.atoms {
            changeset.insert(pk.clone(), atom.clone());
        }
        if let Err(e) = engine
            .storage
            .commit_changeset(&changeset, engine.state.current_crystal_index)
            .await
        {
            log::error!("Shutdown commit_changeset failed: {}", e);
        }

        if let Err(e) = engine.storage.flush(Some(&engine.state.global_metrics)) {
            log::error!("Shutdown flush failed: {}", e);
        }
        let _ = engine.storage.get_db().flush();
        println!("💾 State solidified. Atoms: {}.", engine.state.atoms.len());
        println!("🌌 Obsidian Nexus offline. Farewell, Architect.");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        std::process::exit(0);
    });

    // =========================================================================
    // CLI
    // =========================================================================
    let legacy_key_string = args.master_key.clone();
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
    use tokio::io::AsyncBufReadExt;
    let mut line = String::new();
    println!("\n📟 CLI ready. Type 'help' for commands.");

    loop {
        line.clear();
        let n = tokio::select! {
            res = stdin.read_line(&mut line) => res.unwrap_or(0),
        };
        if n == 0 {
            // stdin closed (headless/redirected mode) — yield generously so
            // IPC, mining and TCP tasks get uncontested runtime access.
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            continue;
        }

        match line.trim() {
            // ── balance ───────────────────────────────────────────────────────
            cmd if cmd == "balance" || cmd.starts_with("balance ") => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                let engine = shared_engine.lock().await;
                if parts.len() >= 2 {
                    match Wallet::decode_address(parts[1]) {
                        Err(e) => eprintln!("❌ {}", e),
                        Ok(pk) => match engine.state.get_atom(&pk) {
                            Some(atom) => println!(
                                "💰 {}: {} mass | elem={:?} | nonce={}",
                                parts[1], atom.mass, atom.element, atom.nonce
                            ),
                            None => {
                                println!(
                                    "⚠️  Atom {} not found. All atoms with mass > 0:",
                                    parts[1]
                                );
                                for (pk, atom) in &engine.state.atoms {
                                    if atom.mass > 0 {
                                        println!(
                                            "   {} | mass={} | elem={:?}",
                                            hex::encode(pk),
                                            atom.mass,
                                            atom.element
                                        );
                                    }
                                }
                            }
                        },
                    }
                } else {
                    match engine.state.get_atom(&architect_pk) {
                        Some(a) => println!(
                            "💰 Architect: {} mass | elem={:?} | nonce={}",
                            a.mass, a.element, a.nonce
                        ),
                        None => println!("💰 Architect: 0 mass (not materialized)"),
                    }
                    let op_pk = operator_wallet.get_public_key_bytes();
                    match engine.state.get_atom(&op_pk) {
                        Some(a) => println!(
                            "🛠️  Operator:  {} mass | elem={:?} | nonce={}",
                            a.mass, a.element, a.nonce
                        ),
                        None => println!("🛠️  Operator:  0 mass (not materialized)"),
                    }
                }
            }

            // ── root ──────────────────────────────────────────────────────────
            "root" => {
                let engine = shared_engine.lock().await;
                println!(
                    "🌳 State Root @ #{}: {}",
                    engine.state.current_crystal_index,
                    hex::encode(engine.storage.current_root().await)

                );
            }

            // ── inspect ───────────────────────────────────────────────────────
            "inspect" => {
                let engine = shared_engine.lock().await;
                println!("🔍 Node snapshot:");
                println!("   Height:      #{}", engine.state.current_crystal_index);
                println!("   Atoms:       {}", engine.state.atoms.len());
                println!("   Entropy:     {:.4}", engine.state.global_metrics.entropy);
                println!("   Temperature: {:.2} K", engine.chamber.temperature);
                println!("   Epoch diff:  {:.4}", engine.epoch_difficulty);
                println!("   Syncing:     {}", engine.is_syncing);
                println!(
                    "   Signing:     {}",
                    if legacy_mode {
                        "legacy (master.key)"
                    } else {
                        "SDK (master.wallet)"
                    }
                );
            }

            // ── status ────────────────────────────────────────────────────────
            "status" => {
                let engine = shared_engine.lock().await;
                println!(
                    "📊 Height: {} | Atoms: {} | Temp: {:.2} K | Entropy: {:.4}",
                    engine.state.current_crystal_index,
                    engine.state.atoms.len(),
                    engine.chamber.temperature,
                    engine.chamber.entropy,
                );
            }

            // ── peers ─────────────────────────────────────────────────────────
            "peers" => {
                let peers = server_net.dht.get_peer_list().await;
                println!("👥 Connected Peers ({}):", peers.len());
                for p in &peers {
                    println!("   - {}", p);
                }
                if peers.is_empty() {
                    println!("   (none)");
                }
            }

            // ── connect ───────────────────────────────────────────────────────
            cmd if cmd == "connect" || cmd.starts_with("connect ") => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                let addr_str = if parts.len() >= 2 {
                    parts[1].to_string()
                } else {
                    use std::io::Write;
                    print!("🔗 Enter peer address (IP:Port): ");
                    let _ = std::io::stdout().flush();
                    let mut addr_line = String::new();
                    if stdin.read_line(&mut addr_line).await.unwrap_or(0) == 0 {
                        eprintln!("❌ Failed to read address");
                        continue;
                    }
                    addr_line.trim().to_string()
                };

                if addr_str.is_empty() {
                    eprintln!("❌ Usage: connect <IP:Port>");
                    continue;
                }
                if addr_str.parse::<std::net::SocketAddr>().is_err() {
                    eprintln!("❌ Invalid address '{}'", addr_str);
                    continue;
                }

                println!("🔗 Connecting to {} …", addr_str);
                let net_c = server_net.clone();
                let addr_c = addr_str.clone();
                tokio::spawn(async move {
                    match net_c.connect_to_peer(&addr_c).await {
                        Ok(_) => println!("✅ PQ-Handshake OK — peer {} registered", addr_c),
                        Err(e) => eprintln!("❌ Connection to {} failed: {}", addr_c, e),
                    }
                });
            }

            // ── send ──────────────────────────────────────────────────────────
            cmd if cmd.starts_with("send ") => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if parts.len() != 3 {
                    eprintln!("❌ Usage: send <recipient_hex> <amount>");
                    continue;
                }
                let recipient_pk = match Wallet::decode_address(parts[1]) {
                    Ok(pk) => pk,
                    Err(e) => {
                        eprintln!("❌ {}", e);
                        continue;
                    }
                };
                let amount: u64 = match parts[2].parse() {
                    Ok(n) if n > 0 => n,
                    Ok(_) => {
                        eprintln!("❌ Amount must be > 0");
                        continue;
                    }
                    Err(e) => {
                        eprintln!("❌ Invalid amount: {}", e);
                        continue;
                    }
                };

                if legacy_mode {
                    println!("⚠️  Legacy signing (master.key).");
                    let legacy_keys = match crate::crypto::Crypto::load_master_key(
                        std::path::Path::new(&legacy_key_string),
                    ) {
                        Ok(k) => k,
                        Err(e) => {
                            eprintln!("❌ Cannot load master.key: {}", e);
                            continue;
                        }
                    };
                    let engine = shared_engine.lock().await;
                    let sender_atom = match engine.state.get_atom(&architect_pk).cloned() {
                        Some(a) => a,
                        None => {
                            eprintln!("❌ Architect atom not found");
                            continue;
                        }
                    };
                    let total_cost = amount + PROTOCOL_MIN_FEE;
                    if sender_atom.mass < total_cost {
                        eprintln!(
                            "❌ Insufficient mass: have {}, need {}",
                            sender_atom.mass, total_cost
                        );
                        continue;
                    }
                    drop(engine);
                    let rx = match crate::kinetic::KineticEngine::build_transfer(
                        sender_atom,
                        recipient_pk.clone(),
                        amount,
                        PROTOCOL_MIN_FEE,
                        &legacy_keys.sk,
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("❌ build_transfer failed: {}", e);
                            continue;
                        }
                    };
                    match sectoral_mempool.lock().await.push(rx) {
                        Ok(true) => println!(
                            "✅ [legacy] Queued: {} mass → {}… | fee={}",
                            amount,
                            hex::encode(&recipient_pk[..8.min(recipient_pk.len())]),
                            PROTOCOL_MIN_FEE
                        ),
                        Ok(false) => eprintln!("⚠️  Duplicate reaction"),
                        Err(e) => eprintln!("❌ Mempool error: {}", e),
                    }
                    continue;
                }

                // SDK path
                let engine = shared_engine.lock().await;
                let sender_atom = match engine.state.get_atom(&architect_pk).cloned() {
                    Some(a) => a,
                    None => {
                        eprintln!("❌ Architect atom not found");
                        continue;
                    }
                };
                let total_cost = amount + PROTOCOL_MIN_FEE;
                if sender_atom.mass < total_cost {
                    eprintln!(
                        "❌ Insufficient mass: have {}, need {}",
                        sender_atom.mass, total_cost
                    );
                    continue;
                }
                println!(
                    "🔗 Sender: mass={} | last_hash={}… | nonce={}",
                    sender_atom.mass,
                    hex::encode(&sender_atom.last_reaction_hash[..8]),
                    sender_atom.nonce
                );
                drop(engine);

                let sdk_element = match sender_atom.element {
                    crate::atom::Element::Hydrogen => AtomElement::Hydrogen,
                    crate::atom::Element::Carbon => AtomElement::Carbon,
                    crate::atom::Element::Oxygen => AtomElement::Oxygen,
                    crate::atom::Element::Gold => AtomElement::Gold,
                };

                let wallet_clone = Arc::clone(&architect_wallet);
                let recipient_pk_c = recipient_pk.clone();
                let sender_mass = sender_atom.mass;
                let sender_last_hash = sender_atom.last_reaction_hash;
                let sender_nonce = sender_atom.nonce;

                let sdk_tx_result = tokio::task::spawn_blocking(move || {
                    std::thread::Builder::new()
                        .name("mldsa-sign".into())
                        .stack_size(16 * 1024 * 1024)
                        .spawn(move || {
                            TransactionBuilder::new(&wallet_clone)
                                .recipient(recipient_pk_c)
                                .amount(amount)
                                .fee(PROTOCOL_MIN_FEE)
                                .sender_mass(sender_mass)
                                .sender_last_hash(sender_last_hash)
                                .sender_nonce(sender_nonce)
                                .sender_element(sdk_element)
                                .build()
                        })
                        .expect("spawn failed")
                        .join()
                        .expect("panicked")
                })
                    .await;

                let sdk_tx = match sdk_tx_result {
                    Ok(Ok(tx)) => tx,
                    Ok(Err(e)) => {
                        eprintln!("❌ TransactionBuilder: {}", e);
                        continue;
                    }
                    Err(e) => {
                        eprintln!("❌ Signing task failed: {}", e);
                        continue;
                    }
                };
                let core_rx = match sdk_tx_to_reaction_result(sdk_tx) {
                    Ok(rx) => rx,
                    Err(e) => {
                        eprintln!("❌ Adapter error: {}", e);
                        continue;
                    }
                };
                match sectoral_mempool.lock().await.push(core_rx) {
                    Ok(true) => println!(
                        "✅ Queued: {} mass → {}… | fee={}",
                        amount,
                        hex::encode(&recipient_pk[..8.min(recipient_pk.len())]),
                        PROTOCOL_MIN_FEE
                    ),
                    Ok(false) => eprintln!("⚠️  Duplicate reaction"),
                    Err(e) => eprintln!("❌ Mempool error: {}", e),
                }
            }

            // ── inject ────────────────────────────────────────────────────────
            "inject" => {
                println!("💉 Injecting Generic reaction…");
                use crate::atom::{Atom, Element};
                use crate::kinetic::Payload;
                use std::time::{SystemTime, UNIX_EPOCH};

                // Используем пустую подпись + Payload::Generic — PVM принимает
                // Generic без проверки подписи (requires_signature_verification = true,
                // но энергия 0.0 пройдёт валидацию структуры).
                // В продакшне inject доступен только локально через IPC.
                let mut dummy_rx = SignedReaction {
                    reaction_hash: [0u8; 32],
                    sender: Atom::new_materialized(architect_pk.clone(), Element::Hydrogen),
                    receiver: Atom::new_materialized(architect_pk.clone(), Element::Hydrogen),
                    energy: PROTOCOL_MIN_FEE as f32,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    signature: vec![],
                    payload: Payload::Generic,
                };
                dummy_rx.reaction_hash = dummy_rx.compute_reaction_hash();

                match sectoral_mempool.lock().await.push(dummy_rx) {
                    Ok(_) => println!("✅ Injection queued."),
                    Err(e) => eprintln!("❌ Injection failed: {}", e),
                }
            }

            // ── help ──────────────────────────────────────────────────────────
            "help" => {
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                println!("  Obsidian Nexus CLI");
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                println!("  balance [<hex>]      — show atom mass");
                println!("  send <hex> <amount>  — queue Transfer transaction");
                println!("  root                 — print State Root");
                println!("  inspect              — full node snapshot");
                println!("  status               — one-line summary");
                println!("  peers                — list known peers");
                println!("  connect [<IP:Port>]  — connect to peer");
                println!("  inject               — queue debug Generic reaction");
                println!("  help                 — this message");
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            }

            other if !other.is_empty() => println!("❓ Unknown: '{}'. Type 'help'.", other),
            _ => {}
        }
    }
}