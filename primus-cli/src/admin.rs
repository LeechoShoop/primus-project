use anyhow::{Context, Result, anyhow};
use primus_types::{IpcRequest, IpcResponse};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) async fn connect_ipc(port: u16) -> Result<impl AsyncReadExt + AsyncWriteExt + Unpin> {
    let path = crate::utils::get_secure_ipc_path(port)?;
    
    #[cfg(unix)]
    {
        crate::utils::verify_ipc_ownership(&path)?;
        use tokio::net::UnixStream;
        UnixStream::connect(&path)
            .await
            .context("Failed to connect to secure IPC socket. Is primus-core running?")
    }

    #[cfg(windows)]
    {
        crate::utils::verify_ipc_ownership(&path)?;
        use tokio::net::windows::named_pipe::ClientOptions;
        
        let mut last_err = None;
        for _i in 0..30 {
            match ClientOptions::new().open(&path) {
                Ok(client) => return Ok(client),
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
            }
        }
        
        Err(last_err.unwrap()).context(format!("Failed to connect to secure Named Pipe ({}) after 30 retries. Is primus-core running?", path))
    }
}

pub async fn shutdown(wallet_path: String, port: u16) -> Result<()> {
    println!("Initiating admin shutdown...");

    let mut stream = connect_ipc(port).await?;

    // 1. Request a challenge nonce from the node
    let challenge = get_challenge(&mut stream).await?;

    // 2. Sign the nonce with the architect key
    let signature = sign_challenge_on_large_stack(&wallet_path, challenge)?;

    // 3. Send authenticated AdminShutdown
    let req = IpcRequest::AdminShutdown { signature };
    send_request(&mut stream, &req).await?;

    match receive_response(&mut stream).await? {
        IpcResponse::Ok => println!("✅ Node shutdown initiated."),
        IpcResponse::Error(e) => return Err(anyhow!("Node rejected command: {}", e)),
        _ => return Err(anyhow!("Unexpected response from node")),
    }
    Ok(())
}

pub async fn connect_peer(addr: String, wallet_path: String, port: u16) -> Result<()> {
    println!("Instructing node to connect to peer: {}", addr);

    let mut stream = connect_ipc(port).await?;

    let challenge = get_challenge(&mut stream).await?;
    let signature = sign_challenge_on_large_stack(&wallet_path, challenge)?;

    let req = IpcRequest::AdminConnectPeer {
        addr: addr.clone(),
        signature,
    };
    send_request(&mut stream, &req).await?;

    match receive_response(&mut stream).await? {
        IpcResponse::Ok => println!("✅ Node connecting to peer: {}", addr),
        IpcResponse::Error(e) => return Err(anyhow!("Node rejected command: {}", e)),
        _ => return Err(anyhow!("Unexpected response from node")),
    }
    Ok(())
}

pub async fn status(port: u16) -> Result<()> {
    let mut stream = connect_ipc(port).await?;
    let req = IpcRequest::Status;
    send_request(&mut stream, &req).await?;
    
    match receive_response(&mut stream).await? {
        IpcResponse::StatusReport {
            height,
            peers,
            cache_size,
            frame_drops,
        } => {
            println!("Node Status:");
            println!("  Height:      {}", height);
            println!("  Peers:       {}", peers);
            println!("  TCP Cache:   {}", cache_size);
            println!("  Frame Drops: {}", frame_drops);
        }
        IpcResponse::Error(e) => return Err(anyhow!("Node rejected command: {}", e)),
        _ => return Err(anyhow!("Unexpected response from node")),
    }
    Ok(())
}

// ── Challenge-Response Helpers ────────────────────────────────────────────────

async fn get_challenge<S: AsyncReadExt + AsyncWriteExt + Unpin>(stream: &mut S) -> Result<Vec<u8>> {
    send_request(stream, &IpcRequest::GetChallenge).await?;
    match receive_response(stream).await? {
        IpcResponse::Challenge(nonce) => Ok(nonce),
        IpcResponse::Error(e) => Err(anyhow!("Failed to get challenge: {}", e)),
        _ => Err(anyhow!("Unexpected response to GetChallenge")),
    }
}

/// ML-DSA-87 key derivation requires a large stack. Run in a dedicated thread.
fn sign_challenge_on_large_stack(wallet_path: &str, challenge: Vec<u8>) -> Result<Vec<u8>> {
    let path = wallet_path.to_string();
    std::thread::Builder::new()
        .name("ipc-signer".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || -> Result<Vec<u8>> {
            use primus_sdk::Wallet;
            let wallet = Wallet::load(Path::new(&path))
                .context("Failed to load wallet for admin signing")?;
            Ok(wallet.sign(&challenge))
        })
        .context("Failed to spawn signer thread")?
        .join()
        .map_err(|_| anyhow!("Signer thread panicked"))?
}

pub(crate) async fn send_request<S: AsyncWriteExt + Unpin>(stream: &mut S, req: &IpcRequest) -> Result<()> {
    let bytes = bincode::serialize(req)?;
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

pub(crate) async fn receive_response<S: AsyncReadExt + Unpin>(stream: &mut S) -> Result<IpcResponse> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 1024 * 1024 {
        return Err(anyhow!("IPC response too large ({} bytes)", len));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(bincode::deserialize(&buf)?)
}
