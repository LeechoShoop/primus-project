// =============================================================================
// primus-cli/src/client.rs — Pure Binary Node Client (no HTTP/JSON)
//
// ARCHITECTURE:
//   All communication with the node happens over a single TCP connection using
//   the same length-prefixed bincode framing that peer nodes use internally
//   (see primus-core/src/network.rs).
//
//   Wire format (identical to network.rs):
//     [4-byte big-endian frame length] ++ [bincode-serialized PrimusMessage]
//
// SUPPORTED QUERIES:
//   FetchState  { address }  → StateResponse { mass, nonce, last_hash, element }
//   SubmitReaction { bytes } → Ack | Error
//
// ERROR HANDLING:
//   • Connection refused  → clear user-facing message, Err propagated to caller.
//   • Timeout (5 s)       → treated as an unreachable node.
//   • Unexpected response → Err with the variant name so callers can debug.
// =============================================================================

use anyhow::Result;
use primus_sdk::AtomElement;
use primus_sdk::error::PrimusSdkError;
use primus_types::peer::PrimusNR;
use primus_types::proof::MerkleProof;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

/// Noise_XX pattern — matches primus-net-opt and primus-core specification.
/// primus-core SPECIFICATION.md §3: X25519/ChaChaPoly_SHA256.
const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_SHA256";

/// X25519 Diffie-Hellman key length in bytes.
const NOISE_KEY_LEN: usize = 32;

// ── Frame size cap (must match primus-core's LengthDelimitedCodec limit) ─────
// AUDIT_REPORT.md DIV-001 fix. primus-core SPECIFICATION.md §7.
// Mirrors primus_sdk::error::MAX_FRAME_SIZE.
const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024; // 16 MiB

// ── Wire protocol messages ────────────────────────────────────────────────────
//
// This enum is the subset of PrimusMessage that the CLI uses.  It is kept
// local to the CLI crate so the CLI has no compile-time dependency on
// primus-core internals.  The node's handle_peer() recognizes these variants
// because they share the same bincode discriminants as the core PrimusMessage
// enum — both enums must stay in sync (same variant order, same field names).
//
// If you add a new variant here you MUST add it to primus-core/src/network.rs
// PrimusMessage at the same position.

/// Messages that the CLI sends to / receives from the node.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum CliMessage {
    // ── Existing peer-level variants (must stay first, in order) ─────────────
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
    NewReaction(Vec<u8>, u8),
    NewCrystal(Vec<u8>, u8),
    GetCrystal(u64),
    CrystalResponse(Vec<u8>),
    Sync(SyncPlaceholder),

    // ── CLI-specific variants (appended after all peer variants) ──────────────
    /// Query an atom's on-chain state.
    ///
    /// `address` is the raw ML-DSA-87 public-key bytes (2592 bytes), NOT the
    /// hex string.  Use `Wallet::decode_address()` to convert before calling.
    FetchState {
        address: Vec<u8>,
    },

    /// Node's reply to FetchState.
    StateResponse {
        mass: u64,
        nonce: u64,
        last_hash: [u8; 32],
        element: String,
    },

    /// Submit a signed, bincode-serialized ReactionResult for inclusion.
    ///
    /// `reaction_bytes` is the output of `Transaction::to_bytes()` from the
    /// SDK (identical wire format to a Core ReactionResult).
    SubmitReaction {
        reaction_bytes: Vec<u8>,
    },

    /// Node acknowledged the reaction and enqueued it in the mempool.
    ReactionAck {
        reaction_hash: [u8; 32],
    },

    /// Node rejected the request; `reason` is a human-readable explanation.
    NodeError {
        reason: String,
    },
    GetProof {
        address: Vec<u8>,
    },
    ProofResponse(MerkleProof),
}

// Placeholder so CliMessage can mirror the full PrimusMessage enum shape.
// The CLI never constructs or inspects Sync messages — it only needs the
// discriminant slot to stay aligned with the node's enum.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SyncPlaceholder;

// ── AtomState — the structured result the CLI commands consume ────────────────

/// On-chain state of a single atom, returned by `NodeClient::get_atom_state`.
pub struct AtomState {
    pub mass: u64,
    pub nonce: u64,
    pub last_hash: [u8; 32],
    pub element: AtomElement,
}

// ── NoiseTransport ────────────────────────────────────────────────────────────

pub enum Transport {
    Noise(NoiseTransport),
    Plain(TcpStream),
}

pub struct NoiseTransport {
    pub stream: TcpStream,
    pub noise: snow::StatelessTransportState,
    pub send_nonce: u64,
    pub recv_nonce: u64,
    pub read_buf: Vec<u8>,
}

impl NoiseTransport {
    pub async fn send_raw(&mut self, plaintext: &[u8]) -> Result<(), PrimusSdkError> {
        let app_len = plaintext.len() as u32;
        if app_len > MAX_FRAME_BYTES {
            return Err(PrimusSdkError::Transport(format!("CLI: outgoing frame too large ({} bytes)", app_len)));
        }
        let mut app_frame = Vec::with_capacity(4 + plaintext.len());
        app_frame.extend_from_slice(&app_len.to_be_bytes());
        app_frame.extend_from_slice(plaintext);

        for chunk in app_frame.chunks(65519) {
            let mut ciphertext = vec![0u8; chunk.len() + 16];
            let len = self.noise.write_message(self.send_nonce, chunk, &mut ciphertext)
                .map_err(|e| PrimusSdkError::Transport(format!("Noise encryption failed: {}", e)))?;
            self.send_nonce += 1;
            
            let len_bytes = (len as u16).to_be_bytes();
            self.stream.write_all(&len_bytes).await
                .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
            self.stream.write_all(&ciphertext[..len]).await
                .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
        }
        self.stream.flush().await
            .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn fill_buf_to(&mut self, target_len: usize) -> Result<(), PrimusSdkError> {
        while self.read_buf.len() < target_len {
            let mut len_buf = [0u8; 2];
            self.stream.read_exact(&mut len_buf).await
                .map_err(|_| PrimusSdkError::NodeUnreachable)?;
            let len = u16::from_be_bytes(len_buf) as usize;
            
            let mut enc_buf = vec![0u8; len];
            self.stream.read_exact(&mut enc_buf).await
                .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                
            let mut out = vec![0u8; len];
            let out_len = self.noise.read_message(self.recv_nonce, &enc_buf, &mut out)
                .map_err(|e| PrimusSdkError::Transport(format!("Noise decryption failed: {}", e)))?;
            self.recv_nonce += 1;
            
            self.read_buf.extend_from_slice(&out[..out_len]);
        }
        Ok(())
    }

    pub async fn recv_raw(&mut self) -> Result<Vec<u8>, PrimusSdkError> {
        self.fill_buf_to(4).await?;
        let frame_len = u32::from_be_bytes([
            self.read_buf[0], self.read_buf[1], self.read_buf[2], self.read_buf[3]
        ]) as usize;
        
        if frame_len > MAX_FRAME_BYTES as usize {
            return Err(PrimusSdkError::Transport(format!("CLI: incoming frame too large ({} bytes)", frame_len)));
        }
        if frame_len == 0 {
            return Err(PrimusSdkError::Transport("CLI: empty frame".into()));
        }

        self.fill_buf_to(4 + frame_len).await?;
        let payload = self.read_buf[4..4 + frame_len].to_vec();
        self.read_buf.drain(0..4 + frame_len);
        
        Ok(payload)
    }
}

// ── NodeClient ────────────────────────────────────────────────────────────────

/// Lightweight client for a single Primus node.
/// Maintains a single Noise encrypted TCP connection for all calls.
pub struct NodeClient {
    pub timeout: Duration,
    pub last_known_nr: Option<PrimusNR>,
    pub transport: Transport,
}

impl NodeClient {
    pub async fn new_with_noise(
        host: &str,
        port: u16,
        static_key: &[u8],
    ) -> Result<Self, PrimusSdkError> {
        if static_key.len() != NOISE_KEY_LEN {
            return Err(PrimusSdkError::Transport(format!(
                "Noise static key must be {} bytes, got {}",
                NOISE_KEY_LEN,
                static_key.len()
            )));
        }

        let addr = format!("{}:{}", host, port);
        let timeout_dur = Duration::from_secs(5);

        let stream = timeout(timeout_dur, TcpStream::connect(&addr))
            .await
            .map_err(|_| PrimusSdkError::NodeUnreachable)?
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::ConnectionRefused {
                    PrimusSdkError::NodeUnreachable
                } else {
                    PrimusSdkError::Transport(format!("TCP connect to {} failed: {}", addr, e))
                }
            })?;

        let builder = snow::Builder::new(
            NOISE_PATTERN
                .parse()
                .map_err(|e| PrimusSdkError::Transport(format!("Noise pattern parse error: {e}")))?,
        );
        let mut handshake = builder
            .local_private_key(static_key)
            .build_initiator()
            .map_err(|e| {
                PrimusSdkError::Transport(format!(
                    "Noise initiator build failed for {addr}: {e}"
                ))
            })?;

        let (mut tcp_reader, mut tcp_writer) = tokio::io::split(stream);
        let mut buf = vec![0u8; 65535];

        while !handshake.is_handshake_finished() {
            eprintln!("Noise Handshake: turn={}, finished={}", handshake.is_my_turn(), handshake.is_handshake_finished());
            if handshake.is_my_turn() {
                let len = handshake.write_message(&[], &mut buf).map_err(|e| {
                    PrimusSdkError::Transport(format!("Noise write_message failed: {e}"))
                })?;
                let frame_len = (len as u16).to_be_bytes();
                tcp_writer.write_all(&frame_len).await
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                tcp_writer.write_all(&buf[..len]).await
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                tcp_writer.flush().await
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
            } else {
                let mut len_buf = [0u8; 2];
                timeout(Duration::from_millis(500), tcp_reader.read_exact(&mut len_buf)).await
                    .map_err(|_| PrimusSdkError::Transport("Handshake read timeout".into()))?
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                let frame_len = u16::from_be_bytes(len_buf) as usize;
                let mut frame = vec![0u8; frame_len];
                tcp_reader.read_exact(&mut frame).await
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                let mut payload = vec![0u8; frame_len];
                handshake.read_message(&frame, &mut payload).map_err(|e| {
                    PrimusSdkError::Transport(format!("Noise read_message failed: {e}"))
                })?;
            }
        }

        let noise = handshake.into_stateless_transport_mode().map_err(|e| {
            PrimusSdkError::Transport(format!("Noise into_stateless_transport_mode failed: {e}"))
        })?;

        let stream = tcp_reader.unsplit(tcp_writer);

        Ok(Self {
            timeout: timeout_dur,
            last_known_nr: None,
            transport: Transport::Noise(NoiseTransport {
                stream,
                noise,
                send_nonce: 0,
                recv_nonce: 0,
                read_buf: Vec::new(),
            }),
        })
    }

    pub async fn new_plain(host: &str, port: u16) -> Result<Self, PrimusSdkError> {
        let addr = format!("{}:{}", host, port);
        let timeout_dur = Duration::from_secs(5);

        let stream = timeout(timeout_dur, TcpStream::connect(&addr))
            .await
            .map_err(|_| PrimusSdkError::NodeUnreachable)?
            .map_err(|e| PrimusSdkError::Transport(format!("TCP connect failed: {e}")))?;

        Ok(Self {
            timeout: timeout_dur,
            last_known_nr: None,
            transport: Transport::Plain(stream),
        })
    }

    pub async fn new_with_ephemeral_noise(host: &str, port: u16) -> Result<Self, PrimusSdkError> {
        let mut static_key = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut static_key);
        Self::new_with_noise(host, port, &static_key).await
    }

    pub async fn new_auto(host: &str, port: u16) -> Result<Self, PrimusSdkError> {
        // Try plain TCP first — the node's P2P listener uses a plain
        // LengthDelimitedCodec (4-byte big-endian header). Attempting Noise first
        // sends a 2-byte-prefixed Noise handshake that the node's LDCodec
        // misinterprets as a partial 4-byte length header, causing
        // "Frame read error: bytes remaining on stream" when the failed
        // Noise attempt closes the connection. Plain always works for the node.
        match Self::new_plain(host, port).await {
            Ok(c) => Ok(c),
            Err(_) => Self::new_with_ephemeral_noise(host, port).await,
        }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    async fn send_msg(&mut self, msg: &CliMessage) -> Result<(), PrimusSdkError> {
        let bytes = bincode::serialize(msg)
            .map_err(|e| PrimusSdkError::Transport(format!("Serialization failed: {e}")))?;

        match &mut self.transport {
            Transport::Noise(noise) => noise.send_raw(&bytes).await,
            Transport::Plain(stream) => {
                stream.write_all(&(bytes.len() as u32).to_be_bytes()).await
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                stream.write_all(&bytes).await
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                stream.flush().await
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                Ok(())
            }
        }
    }

    async fn recv_msg(&mut self) -> Result<CliMessage, PrimusSdkError> {
        let payload = match &mut self.transport {
            Transport::Noise(noise) => noise.recv_raw().await?,
            Transport::Plain(stream) => {
                let mut len_buf = [0u8; 4];
                stream.read_exact(&mut len_buf).await
                    .map_err(|_| PrimusSdkError::NodeUnreachable)?;
                let len = u32::from_be_bytes(len_buf) as usize;
                if len > MAX_FRAME_BYTES as usize {
                    return Err(PrimusSdkError::Transport("Incoming frame too large".into()));
                }
                let mut buf = vec![0u8; len];
                stream.read_exact(&mut buf).await
                    .map_err(|e| PrimusSdkError::Transport(e.to_string()))?;
                buf
            }
        };
        bincode::deserialize(&payload)
            .map_err(|e| PrimusSdkError::Transport(format!("Deserialization failed: {e}")))
    }

    pub async fn get_atom_state(&mut self, address_hex: &str) -> Result<AtomState, PrimusSdkError> {
        let addr_bytes = primus_sdk::Wallet::decode_address(address_hex)
            .map_err(|e| PrimusSdkError::Transport(format!("Invalid address: {e}")))?;

        self.send_msg(&CliMessage::FetchState {
            address: addr_bytes.to_vec(),
        })
        .await?;

        match self.recv_msg().await? {
            CliMessage::StateResponse {
                mass,
                nonce,
                last_hash,
                element,
            } => Ok(AtomState {
                mass,
                nonce,
                last_hash,
                element: match element.as_str() {
                    "Hydrogen" => AtomElement::Hydrogen,
                    "Carbon" => AtomElement::Carbon,
                    "Oxygen" => AtomElement::Oxygen,
                    "Gold" => AtomElement::Gold,
                    _ => AtomElement::Hydrogen,
                },
            }),
            CliMessage::NodeError { reason } => Err(PrimusSdkError::NodeError { reason }),
            _ => Err(PrimusSdkError::Transport("Unexpected response".into())),
        }
    }

    pub async fn broadcast_tx(
        &mut self,
        wallet: &primus_sdk::Wallet,
        builder: primus_sdk::TransactionBuilder<'_>,
    ) -> Result<String, PrimusSdkError> {
        let mut builder = builder;
        let mut retry_done = false;

        loop {
            let tx = builder
                .clone()
                .build()
                .map_err(|e| PrimusSdkError::Transport(format!("Build failed: {e}")))?;
            let bytes = tx
                .to_bytes()
                .map_err(|e| PrimusSdkError::Transport(format!("Serialization failed: {e}")))?;

            self.send_msg(&CliMessage::SubmitReaction {
                reaction_bytes: bytes,
            })
            .await?;

            match self.recv_msg().await? {
                CliMessage::ReactionAck { reaction_hash } => {
                    return Ok(format!("Accepted (hash: {:02x?})", &reaction_hash[..4]));
                }
                CliMessage::NodeError { reason } => {
                    if reason.contains("Sequence Mismatch") && !retry_done {
                        let state = self.get_atom_state(&wallet.address).await?;
                        builder = builder.sender_nonce(state.nonce).sender_last_hash(state.last_hash);
                        retry_done = true;
                        continue;
                    }
                    return Err(PrimusSdkError::NodeError { reason });
                }
                other => {
                    return Err(PrimusSdkError::NodeError {
                        reason: format!("Unexpected response to SubmitReaction: {:?}", other),
                    });
                }
            }
        }
    }

    pub async fn get_crystal(&mut self, height: u64) -> Result<Vec<u8>, PrimusSdkError> {
        self.send_msg(&CliMessage::GetCrystal(height)).await?;
        match self.recv_msg().await? {
            CliMessage::CrystalResponse(bytes) => Ok(bytes),
            CliMessage::NodeError { reason } => Err(PrimusSdkError::NodeError { reason }),
            _ => Err(PrimusSdkError::Transport("Unexpected response".into())),
        }
    }

    pub async fn get_balance_proof(&mut self, address_hex: &str) -> Result<MerkleProof, PrimusSdkError> {
        let addr_bytes = primus_sdk::Wallet::decode_address(address_hex)
            .map_err(|e| PrimusSdkError::Transport(format!("Invalid address: {e}")))?;

        self.send_msg(&CliMessage::GetProof {
            address: addr_bytes.to_vec(),
        })
        .await?;

        match self.recv_msg().await? {
            CliMessage::ProofResponse(proof) => Ok(proof),
            CliMessage::NodeError { reason } => Err(PrimusSdkError::NodeError { reason }),
            _ => Err(PrimusSdkError::Transport("Unexpected response".into())),
        }
    }
}
