// =============================================================================
// primus-types/src/error.rs
//
// PrimusError is the unified error type for structural validation failures.
//
// DESIGN RULES:
//   1. This enum covers only errors that can be detected without chain state
//      and without cryptographic operations. "Signature invalid" is NOT here
//      because it requires ml-dsa — that error lives in primus-core.
//      "Sender has insufficient mass" is NOT here because it requires the
//      StateTree — that error lives in the PVM.
//   2. Every variant carries enough context to produce a useful log message
//      without additional lookups. "InvalidPublicKeyLength { expected, actual }"
//      is more useful than "InvalidPublicKey".
//   3. The #[cfg(feature = "std")] gate on the std::error::Error impl follows
//      the standard no_std pattern: the core error type works in no_std,
//      but the std integration (for ? operator ergonomics in std code) is
//      feature-gated.
// =============================================================================

use crate::constants::REACTION_HASH_BYTES;

/// Structural validation errors that can be detected without chain state
/// or cryptographic operations.
#[derive(Debug, Clone, PartialEq)]
pub enum PrimusError {
    /// A public key had the wrong byte length.
    InvalidPublicKeyLength { expected: usize, actual: usize },

    /// A signature had the wrong byte length.
    InvalidSignatureLength { expected: usize, actual: usize },

    /// The network fee was below the protocol minimum.
    FeeBelowMinimum { minimum: u64, actual: u64 },

    /// The reaction_hash in the struct does not match the value computed
    /// from the transaction fields. The reaction was tampered with after
    /// construction, or was constructed with mismatched fields.
    ReactionHashMismatch {
        expected: [u8; REACTION_HASH_BYTES],
        actual: [u8; REACTION_HASH_BYTES],
    },

    /// Bincode deserialization failed. The `reason` field is a static
    /// string (not a String) to keep this variant no_std compatible.
    DeserializationFailed { reason: &'static str },

    /// An unknown payload variant was encountered. Produced when a node
    /// running older software receives a reaction with a future payload type.
    UnknownPayload,
}

// ── Display ───────────────────────────────────────────────────────────────────

// core::fmt::Display works in no_std (it uses core::fmt, not std::fmt).
impl core::fmt::Display for PrimusError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PrimusError::InvalidPublicKeyLength { expected, actual } => write!(
                f,
                "Public key length: expected {}, got {}",
                expected, actual
            ),
            PrimusError::InvalidSignatureLength { expected, actual } => {
                write!(f, "Signature length: expected {}, got {}", expected, actual)
            }
            PrimusError::FeeBelowMinimum { minimum, actual } => {
                write!(f, "Fee {} is below minimum {}", actual, minimum)
            }
            PrimusError::ReactionHashMismatch { expected, actual } => write!(
                f,
                "Reaction hash mismatch: expected {:02x?}…, got {:02x?}…",
                &expected[..4],
                &actual[..4]
            ),
            PrimusError::DeserializationFailed { reason } => {
                write!(f, "Deserialization failed: {}", reason)
            }
            PrimusError::UnknownPayload => write!(f, "Unknown payload variant — upgrade required"),
        }
    }
}

// ── std::error::Error (std only) ──────────────────────────────────────────────

#[cfg(feature = "std")]
impl std::error::Error for PrimusError {}
