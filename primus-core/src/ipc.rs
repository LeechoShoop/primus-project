use crate::crypto::Crypto;
use anyhow::{Result, anyhow};
use dashmap::DashMap;
use primus_types::{IpcRequest, IpcResponse};
use rand::RngCore;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;

use crate::processor::SharedEngine;
use crate::bridge::CoreHandleImpl;
use primus_net_opt::network::{CoreHandle, PrimusNetwork};

pub struct IpcServer {
    architect_pk: Vec<u8>,
    challenges: Arc<DashMap<u64, [u8; 32]>>,
    engine: SharedEngine,
    network: PrimusNetwork<CoreHandleImpl>,
}

impl IpcServer {
    pub fn new(
        architect_pk: Vec<u8>,
        engine: SharedEngine,
        network: PrimusNetwork<CoreHandleImpl>,
    ) -> Self {
        Self {
            architect_pk,
            challenges: Arc::new(DashMap::new()),
            engine,
            network,
        }
    }

    pub fn get_secure_ipc_path(port: u16) -> Result<String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut path = if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
                std::path::PathBuf::from(runtime_dir)
            } else {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
                let mut p = std::path::PathBuf::from(home);
                p.push(".primus");
                p.push("run");
                p
            };
            std::fs::create_dir_all(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            path.push(format!("primus-{}.sock", port));
            Ok(path.to_string_lossy().to_string())
        }

        #[cfg(windows)]
        {
            let output = std::process::Command::new("whoami")
                .args(["/user", "/fo", "csv", "/nh"])
                .output()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let parts: Vec<&str> = stdout.trim().split(',').collect();
            if parts.len() == 2 {
                let sid = parts[1].trim_matches('"');
                Ok(format!(r"\\.\pipe\primus-nexus-{}-{}", port, sid))
            } else {
                Err(anyhow!("Failed to retrieve user SID on Windows for secure IPC path"))
            }
        }
    }

    #[cfg(unix)]
    pub async fn run(self, path: &str) -> Result<()> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        println!("🔌 IPC: Unix Domain Socket active at {}", path);

        let mut conn_id_gen = 0u64;
        let challenges = self.challenges.clone();
        let pk = self.architect_pk.clone();
        let engine = self.engine.clone();
        let network = self.network.clone();

        loop {
            let (stream, _) = listener.accept().await?;
            conn_id_gen += 1;
            let cid = conn_id_gen;
            let challenges_c = challenges.clone();
            let pk_c = pk.clone();
            let engine_c = engine.clone();
            let network_c = network.clone();

            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(stream, cid, challenges_c, pk_c, engine_c, network_c).await {
                    eprintln!("⚠️ IPC Error (conn {}): {}", cid, e);
                }
            });
        }
    }

    #[cfg(windows)]
    pub async fn run(self, pipe_name: &str) -> Result<()> {
        let pipe_full_path = if pipe_name.starts_with(r"\\.\pipe\") {
            pipe_name.to_string()
        } else {
            format!(r"\\.\pipe\{}", pipe_name)
        };
        log::info!("🔌 IPC: Named Pipe active at {}", pipe_full_path);
        println!("🔌 IPC: Named Pipe active at {}", pipe_full_path);

        let mut conn_id_gen = 0u64;
        let challenges = self.challenges.clone();
        let pk = self.architect_pk.clone();
        let engine = self.engine.clone();
        let network = self.network.clone();

        // Pre-create the first listening instance so the pipe exists immediately.
        let mut pending = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_full_path)?;

        loop {
            // Wait for a client to connect on the *current* pending instance.
            pending.connect().await?;

            conn_id_gen += 1;
            let cid = conn_id_gen;

            // --- RACE-FREE HANDOFF ---
            // Create the NEXT listening instance before handing off `pending` to
            // the spawned task. This guarantees the pipe always has a server
            // instance waiting, eliminating the "The system cannot find the file
            // specified" (os error 2) gap that existed in the previous loop.
            let next = ServerOptions::new()
                .first_pipe_instance(false)
                .create(&pipe_full_path)?;

            // Swap: the just-connected instance goes to the handler thread.
            let connected = std::mem::replace(&mut pending, next);

            let challenges_c = challenges.clone();
            let pk_c = pk.clone();
            let engine_c = engine.clone();
            let network_c = network.clone();

            tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(connected, cid, challenges_c, pk_c, engine_c, network_c).await {
                    eprintln!("⚠️ IPC Error (conn {}): {}", cid, e);
                }
            });
        }
    }

    async fn handle_connection<S>(
        mut stream: S,
        cid: u64,
        challenges: Arc<DashMap<u64, [u8; 32]>>,
        pk: Vec<u8>,
        engine: SharedEngine,
        network: PrimusNetwork<CoreHandleImpl>,
    ) -> Result<()>
    where
        S: AsyncReadExt + AsyncWriteExt + Unpin,
    {
        loop {
            let mut len_buf = [0u8; 4];
            if stream.read_exact(&mut len_buf).await.is_err() {
                break;
            }
            let len = u32::from_le_bytes(len_buf) as usize;
            if len > 1024 * 1024 {
                return Err(anyhow!("IPC request too large"));
            }

            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;

            let req: IpcRequest = bincode::deserialize(&buf)?;
            let resp = match req {
                IpcRequest::Status => {
                    let e = engine.lock().await;
                    let peer_list = network.dht.get_peer_list().await;
                    let frame_drops = network.frame_drops.load(Ordering::Relaxed);
                    IpcResponse::StatusReport {
                        height: e.state.current_crystal_index,
                        peers: peer_list.len(),
                        cache_size: network.tcp_cache.len(),
                        frame_drops,
                    }
                }
                IpcRequest::GetChallenge => {
                    let mut nonce = [0u8; 32];
                    rand::thread_rng().fill_bytes(&mut nonce);
                    challenges.insert(cid, nonce);
                    IpcResponse::Challenge(nonce.to_vec())
                }
                IpcRequest::AdminShutdown { signature } => {
                    if let Some(nonce) = challenges.get(&cid) {
                        let pk_c = pk.clone();
                        let nonce_c = *nonce;
                        let sig_c = signature.clone();
                        let is_valid = tokio::task::spawn_blocking(move || {
                            std::thread::Builder::new()
                                .name("ipc-verify".into())
                                .stack_size(16 * 1024 * 1024)
                                .spawn(move || Crypto::verify(&pk_c, &nonce_c, &sig_c))
                                .expect("spawn failed")
                                .join()
                                .expect("panicked")
                        })
                            .await
                            .expect("spawn_blocking failed");

                        if is_valid {
                            println!("🛑 IPC: Authenticated Admin Shutdown command received.");
                            let resp_bytes = bincode::serialize(&IpcResponse::Ok)?;
                            stream
                                .write_all(&(resp_bytes.len() as u32).to_le_bytes())
                                .await?;
                            stream.write_all(&resp_bytes).await?;
                            let _ = stream.flush().await;
                            std::process::exit(0);
                        } else {
                            IpcResponse::Error("Invalid ML-DSA-87 signature".to_string())
                        }
                    } else {
                        IpcResponse::Error("No challenge issued for this connection".to_string())
                    }
                }
                IpcRequest::AdminConnectPeer { addr, signature } => {
                    if let Some(nonce) = challenges.get(&cid) {
                        let pk_c = pk.clone();
                        let nonce_c = *nonce;
                        let sig_c = signature.clone();
                        let is_valid = tokio::task::spawn_blocking(move || {
                            std::thread::Builder::new()
                                .name("ipc-verify".into())
                                .stack_size(16 * 1024 * 1024)
                                .spawn(move || Crypto::verify(&pk_c, &nonce_c, &sig_c))
                                .expect("spawn failed")
                                .join()
                                .expect("panicked")
                        })
                            .await
                            .expect("spawn_blocking failed");

                        if is_valid {
                            println!("🚀 IPC: Authenticated Admin ConnectPeer received: {}", addr);
                            IpcResponse::Ok
                        } else {
                            IpcResponse::Error("Invalid ML-DSA-87 signature".to_string())
                        }
                    } else {
                        IpcResponse::Error("No challenge issued for this connection".to_string())
                    }
                }
                IpcRequest::GetProof { address } => {
                    match network.core.on_get_proof(address).await {
                        Ok(proof) => IpcResponse::ProofResponse(proof),
                        Err(e) => IpcResponse::Error(e.to_string()),
                    }
                }
            };

            let resp_bytes = bincode::serialize(&resp)?;
            stream
                .write_all(&(resp_bytes.len() as u32).to_le_bytes())
                .await?;
            stream.write_all(&resp_bytes).await?;
        }
        challenges.remove(&cid);
        Ok(())
    }
}