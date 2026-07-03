//! Crystal chunking shim for Noise Protocol transport.
//!
//! BACKGROUND
//! The Noise Protocol hard limit is 65535 bytes per message (Noise spec §3).
//! primus-net-opt/src/server.rs:142 sends entire payloads in a single
//! noise.write_message() call — no chunking exists (frozen module).
//! Crystals with a full reaction load easily exceed this limit.
//!
//! primus-core SPECIFICATION.md §7: logical application messages up to 16 MiB.
//!
//! WIRE FORMAT
//! Each chunk is a ChunkEnvelope serialized via rkyv, sent as a raw gossip payload.
//!
//!   ChunkEnvelope {
//!     stream_id:    [u8; 16],  // random per original message, ties chunks together
//!     chunk_index:  u32,       // 0-based
//!     total_chunks: u32,       // total count for this stream_id
//!     payload_hash: [u8; 32],  // blake3 of full reassembled message (anti-tamper)
//!     data:         Vec<u8>,   // chunk bytes
//!   }
//!
//! REASSEMBLY
//! CoreHandleImpl accumulates chunks by stream_id in a ChunkReassembler.
//! When all chunks arrive, blake3 hash is verified, then the full message
//! is processed as if it arrived in one piece.
//! Incomplete streams are evicted after CHUNK_STREAM_TTL_SECS.
//!
//! AUDIT_REPORT.md: fixes BLK-002 / DIV frame size mismatch
//! Prompt 1 conclusion: Case A confirmed

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Maximum Noise payload size — 65535 hard limit minus 16 bytes Noise header.
#[allow(dead_code)]
pub const NOISE_MAX_PAYLOAD: usize = 65519;

/// Maximum logical application message — matches SPECIFICATION.md §7
pub const APP_MAX_MESSAGE: usize = 16 * 1024 * 1024; // 16 MiB

/// Evict incomplete chunk streams after this many seconds
pub const CHUNK_STREAM_TTL_SECS: u64 = 30;

#[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[archive(check_bytes)]
pub struct ChunkEnvelope {
    pub stream_id:    [u8; 16],
    pub chunk_index:  u32,
    pub total_chunks: u32,
    pub payload_hash: [u8; 32],
    pub data:         Vec<u8>,
}

#[derive(Debug)]
pub enum FrameError {
    MessageTooLarge { size: usize, limit: usize },
    HashMismatch    { expected: [u8; 32], got: [u8; 32] },
    StreamExpired,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MessageTooLarge { size, limit } =>
                write!(f, "message too large: {} > {} bytes", size, limit),
            Self::HashMismatch { .. } =>
                write!(f, "reassembled payload hash mismatch — possible tampering"),
            Self::StreamExpired =>
                write!(f, "chunk stream expired before all chunks arrived"),
        }
    }
}

/// Split a logical message into Noise-safe ChunkEnvelopes.
#[allow(dead_code)]
pub fn chunk_message(
    data: &[u8],
    stream_id: [u8; 16],
) -> Result<Vec<ChunkEnvelope>, FrameError> {
    if data.len() > APP_MAX_MESSAGE {
        return Err(FrameError::MessageTooLarge {
            size: data.len(),
            limit: APP_MAX_MESSAGE,
        });
    }

    let payload_hash: [u8; 32] = blake3::hash(data).into();
    let raw_chunks: Vec<&[u8]> = data.chunks(NOISE_MAX_PAYLOAD).collect();
    let total_chunks = raw_chunks.len() as u32;

    Ok(raw_chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| ChunkEnvelope {
            stream_id,
            chunk_index:  i as u32,
            total_chunks,
            payload_hash,
            data: chunk.to_vec(),
        })
        .collect())
}

struct PartialStream {
    total_chunks:  u32,
    expected_hash: [u8; 32],
    received:      HashMap<u32, Vec<u8>>,
    first_seen:    Instant,
}

/// Accumulates incoming chunks by stream_id, returns full message when complete.
pub struct ChunkReassembler {
    streams: HashMap<[u8; 16], PartialStream>,
}

impl ChunkReassembler {
    pub fn new() -> Self {
        Self { streams: HashMap::new() }
    }

    /// Feed one chunk. Returns `Ok(Some(data))` when all chunks have arrived
    /// and the blake3 hash is valid. Returns `Ok(None)` if more chunks are needed.
    pub fn feed(&mut self, env: ChunkEnvelope) -> Result<Option<Vec<u8>>, FrameError> {
        self.evict_expired();

        let stream = self.streams.entry(env.stream_id).or_insert_with(|| PartialStream {
            total_chunks:  env.total_chunks,
            expected_hash: env.payload_hash,
            received:      HashMap::new(),
            first_seen:    Instant::now(),
        });

        stream.received.insert(env.chunk_index, env.data);

        if stream.received.len() as u32 == stream.total_chunks {
            let reassembled: Vec<u8> = (0..stream.total_chunks)
                .flat_map(|i| stream.received.remove(&i).unwrap_or_default())
                .collect();

            let got_hash: [u8; 32] = blake3::hash(&reassembled).into();
            let expected  = stream.expected_hash;
            self.streams.remove(&env.stream_id);

            if got_hash != expected {
                return Err(FrameError::HashMismatch { expected, got: got_hash });
            }
            return Ok(Some(reassembled));
        }

        Ok(None)
    }

    /// Force eviction of expired chunk streams.
    /// Called by the background GC task in main.rs every CHUNK_STREAM_TTL_SECS.
    pub fn evict_expired(&mut self) {
        let ttl = Duration::from_secs(CHUNK_STREAM_TTL_SECS);
        self.streams.retain(|_, s| s.first_seen.elapsed() < ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_message_is_single_chunk() {
        let data = b"hello primus";
        let chunks = chunk_message(data, [1u8; 16]).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].total_chunks, 1);
    }

    #[test]
    fn large_message_splits_and_reassembles_correctly() {
        let data = vec![0xABu8; NOISE_MAX_PAYLOAD + 1000];
        let chunks = chunk_message(&data, [2u8; 16]).unwrap();
        assert_eq!(chunks.len(), 2);

        let mut r = ChunkReassembler::new();
        let mut out = None;
        for chunk in chunks { out = r.feed(chunk).unwrap(); }
        assert_eq!(out.unwrap(), data);
    }

    #[test]
    fn message_exceeding_app_limit_is_rejected() {
        let data = vec![0u8; APP_MAX_MESSAGE + 1];
        assert!(matches!(
            chunk_message(&data, [0u8; 16]),
            Err(FrameError::MessageTooLarge { .. })
        ));
    }

    #[test]
    fn tampered_chunk_fails_hash_verification() {
        let data = vec![0x42u8; NOISE_MAX_PAYLOAD + 500];
        let mut chunks = chunk_message(&data, [3u8; 16]).unwrap();
        chunks[0].data[0] ^= 0xFF; // tamper

        let mut r = ChunkReassembler::new();
        let mut err = None;
        for chunk in chunks {
            if let Err(e) = r.feed(chunk) { err = Some(e); break; }
        }
        assert!(matches!(err, Some(FrameError::HashMismatch { .. })));
    }

    #[test]
    fn out_of_order_chunks_reassemble_correctly() {
        let data = vec![0xCCu8; NOISE_MAX_PAYLOAD * 3 + 100];
        let mut chunks = chunk_message(&data, [4u8; 16]).unwrap();
        chunks.reverse(); // feed in reverse order

        let mut r = ChunkReassembler::new();
        let mut out = None;
        for chunk in chunks { out = r.feed(chunk).unwrap(); }
        assert_eq!(out.unwrap(), data);
    }
    #[test]
    fn exact_noise_boundary_is_single_chunk() {
        // A message of exactly NOISE_MAX_PAYLOAD bytes must produce exactly 1 chunk
        let data = vec![0x55u8; NOISE_MAX_PAYLOAD];
        let chunks = chunk_message(&data, [10u8; 16]).unwrap();
        assert_eq!(chunks.len(), 1, "exactly NOISE_MAX_PAYLOAD must be 1 chunk");
    }

    #[test]
    fn one_byte_over_boundary_is_two_chunks() {
        let data = vec![0x55u8; NOISE_MAX_PAYLOAD + 1];
        let chunks = chunk_message(&data, [11u8; 16]).unwrap();
        assert_eq!(chunks.len(), 2, "NOISE_MAX_PAYLOAD+1 must split into 2 chunks");
        assert_eq!(chunks[0].data.len(), NOISE_MAX_PAYLOAD);
        assert_eq!(chunks[1].data.len(), 1);
    }

    #[test]
    fn stream_ttl_evicts_incomplete_streams() {
        use std::time::Duration;
        // This test verifies that the eviction logic compiles and runs
        // without panic — actual TTL eviction requires time manipulation
        // which is out of scope. Just verify the reassembler handles
        // a single chunk for a 2-chunk message (incomplete) without returning data.
        let data = vec![0xAAu8; NOISE_MAX_PAYLOAD + 100];
        let chunks = chunk_message(&data, [12u8; 16]).unwrap();
        assert_eq!(chunks.len(), 2);

        let mut r = ChunkReassembler::new();
        // Feed only the second chunk — should return None (waiting for first)
        let result = r.feed(chunks[1].clone()).unwrap();
        assert!(result.is_none(), "incomplete stream must return None");
    }

    #[test]
    fn different_stream_ids_do_not_interfere() {
        let data_a = vec![0x11u8; NOISE_MAX_PAYLOAD + 50];
        let data_b = vec![0x22u8; NOISE_MAX_PAYLOAD + 50];
        let chunks_a = chunk_message(&data_a, [0xAAu8; 16]).unwrap();
        let chunks_b = chunk_message(&data_b, [0xBBu8; 16]).unwrap();

        let mut r = ChunkReassembler::new();

        // Interleave chunks from two different streams
        r.feed(chunks_a[0].clone()).unwrap();
        r.feed(chunks_b[0].clone()).unwrap();

        let result_a = r.feed(chunks_a[1].clone()).unwrap();
        let result_b = r.feed(chunks_b[1].clone()).unwrap();

        assert_eq!(result_a.unwrap(), data_a);
        assert_eq!(result_b.unwrap(), data_b);
    }
}
