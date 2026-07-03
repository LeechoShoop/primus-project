// =============================================================================
// primus-types/src/peer.rs
//
// PrimusNR — Node Record for a peer on the Obsidian Nexus network.
// NoiseHandshakePayload — ephemeral payload used in the Noise XX handshake.
//
// STD-GATING RULES:
//   The networking types (SocketAddr, IpAddr, Ipv6Addr) live behind
//   #[cfg(feature = "std")] because they are not available in no_std.
//   The core PrimusNR struct itself is always available (no_std + alloc):
//   addr_ip/addr_port store the address as primitives, and addr() /
//   from_socket_addr() are compiled only when std is present.
//
//   PrimusNR::verify() requires the `verify` feature (implies `std`).
//   It is a transitional shim — the canonical location for ML-DSA-87
//   operations is primus-core. See Cargo.toml for the migration plan.
//
// RKYV NOTE:
//   SocketAddr is not rkyv-compatible (lacks Archive impls), which is why
//   addr_ip is stored as u128 (IPv6 canonical; IPv4-mapped for v4) and
//   addr_port as u16. The addr() method reconstructs SocketAddr on demand.
// =============================================================================

use crate::Vec;
use serde::{Deserialize, Serialize};

/// A Node Record for a peer on the Primus network.
///
/// Self-signed by the node's ML-DSA-87 key over the fields
/// `[public_key || addr_ip_be || addr_port_be || timestamp_le]`.
/// Peers must call `verify()` before accepting a NR into the routing table.
#[derive(
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[archive(check_bytes)]
#[archive_attr(derive(Debug, PartialEq))]
#[repr(C)]
pub struct PrimusNR {
    /// ML-DSA-87 verifying key (PK_BYTES = 2592 bytes).
    pub public_key: Vec<u8>,

    /// IPv6 representation of the node's IP address.
    /// IPv4 addresses are stored as IPv4-mapped IPv6 (::ffff:x.x.x.x).
    /// Use `addr()` to reconstruct a `SocketAddr` (requires `std` feature).
    pub addr_ip: u128,

    /// Port number for the node's listener.
    pub addr_port: u16,

    /// ML-DSA-87 signature over `[public_key || addr_ip_be || addr_port_be || timestamp_le]`.
    pub signature: Vec<u8>,

    /// Unix timestamp in seconds at which the node record was signed.
    pub timestamp: u64,
}

impl PrimusNR {
    /// Compute the NodeID: SHA3-256(public_key).
    ///
    /// The NodeID is the stable identifier for a peer in the Kademlia DHT.
    /// It does not change when the node's IP address changes; only when the
    /// node generates a new keypair.
    pub fn node_id(&self) -> [u8; 32] {
        use sha3::{Digest, Sha3_256};
        let mut hasher = Sha3_256::new();
        hasher.update(&self.public_key);
        let mut res = [0u8; 32];
        res.copy_from_slice(&hasher.finalize());
        res
    }

    /// Reconstruct the `SocketAddr` from the stored ip/port fields.
    ///
    /// IPv4-mapped IPv6 addresses (::ffff:x.x.x.x) are returned as
    /// `IpAddr::V4` for ergonomic use in std networking code.
    #[cfg(feature = "std")]
    pub fn addr(&self) -> std::net::SocketAddr {
        use std::net::{IpAddr, Ipv6Addr};
        let ip = Ipv6Addr::from(self.addr_ip);
        let ip = match ip.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(ip),
        };
        std::net::SocketAddr::new(ip, self.addr_port)
    }

    /// Construct a `PrimusNR` from a `SocketAddr`.
    ///
    /// The caller is responsible for providing a valid `signature` produced
    /// by the node's ML-DSA-87 signing key over the canonical message
    /// `[public_key || addr_ip_be || addr_port_be || timestamp_le]`.
    #[cfg(feature = "std")]
    pub fn from_socket_addr(
        public_key: Vec<u8>,
        addr: std::net::SocketAddr,
        signature: Vec<u8>,
        timestamp: u64,
    ) -> Self {
        use std::net::IpAddr;
        let addr_ip = match addr.ip() {
            IpAddr::V4(v4) => v4.to_ipv6_mapped().into(),
            IpAddr::V6(v6) => u128::from(v6),
        };
        Self {
            public_key,
            addr_ip,
            addr_port: addr.port(),
            signature,
            timestamp,
        }
    }

    /// Verify the self-signature of this Node Record.
    ///
    /// Returns `true` if and only if `signature` is a valid ML-DSA-87
    /// signature by `public_key` over
    /// `[public_key || addr_ip_be || addr_port_be || timestamp_le]`.
    ///
    /// # Transitional note
    ///
    /// This method is gated behind the `verify` feature flag because
    /// ML-DSA-87 signature verification belongs architecturally in
    /// primus-core. This shim exists to allow primus-net-opt to call
    /// verify() without depending on primus-core. It will be removed
    /// once primus-core exposes a CryptoEngine trait that net-opt can
    /// depend on via the types crate.
    ///
    /// Do not add further ml-dsa call sites in this crate.
    #[cfg(feature = "verify")]
    pub fn verify(&self) -> bool {
        use ml_dsa::signature::Verifier;
        use ml_dsa::{MlDsa87, VerifyingKey};

        // The signed message is: public_key || addr_ip (big-endian) ||
        //                        addr_port (big-endian) || timestamp (little-endian).
        // Field encodings match from_socket_addr to ensure the signer and
        // verifier use the same byte sequence.
        let mut msg = Vec::with_capacity(self.public_key.len() + 16 + 2 + 8);
        msg.extend_from_slice(&self.public_key);
        msg.extend_from_slice(&self.addr_ip.to_be_bytes());
        msg.extend_from_slice(&self.addr_port.to_be_bytes());
        msg.extend_from_slice(&self.timestamp.to_le_bytes());

        let Ok(pk_bytes) = self.public_key[..].try_into() else {
            return false;
        };
        let pk = VerifyingKey::<MlDsa87>::decode(pk_bytes);

        let Ok(sig_bytes) = self.signature[..].try_into() else {
            return false;
        };
        let Some(sig) = ml_dsa::Signature::<MlDsa87>::decode(sig_bytes) else {
            return false;
        };

        pk.verify(&msg, &sig).is_ok()
    }
}

/// Payload carried in the Noise XX handshake initiator and responder messages.
///
/// The `ephemeral_sig` field is an ML-DSA-87 signature by `nr.public_key`
/// over the Noise ephemeral public key bytes, binding the static identity
/// to the ephemeral session key and preventing identity misbinding attacks.
#[derive(
    Serialize, Deserialize, Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[archive(check_bytes)]
#[archive_attr(derive(Debug))]
#[repr(C)]
pub struct NoiseHandshakePayload {
    /// The node record of the peer initiating or responding to the handshake.
    pub nr: PrimusNR,

    /// ML-DSA-87 signature over the Noise ephemeral key, preventing
    /// identity misbinding. Must be verified by the handshake handler
    /// in primus-net-opt before accepting the session.
    pub ephemeral_sig: Vec<u8>,
}
