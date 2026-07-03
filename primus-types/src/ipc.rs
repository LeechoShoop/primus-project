use crate::{String, Vec};
use serde::{Deserialize, Serialize};

#[derive(
    Serialize, Deserialize, Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
#[repr(C)]
pub enum IpcRequest {
    Status,
    GetChallenge,
    AdminShutdown { signature: Vec<u8> },
    AdminConnectPeer { addr: String, signature: Vec<u8> },
    GetProof { address: Vec<u8> },
}

#[derive(
    Serialize, Deserialize, Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
#[repr(C)]
pub enum IpcResponse {
    Ok,
    Error(String),
    Challenge(Vec<u8>), // 32-byte nonce
    StatusReport {
        height: u64,
        peers: usize,
        #[serde(default)]
        cache_size: usize,
        #[serde(default)]
        frame_drops: u64,
    },
    ProofResponse(crate::MerkleProof),
}
