use std::fmt;

// ── Frame size constant ───────────────────────────────────────────────────────

// Frame size must match primus-core's LengthDelimitedCodec limit.
// Previous value of 32 MiB caused silent connection drops against live nodes.
// AUDIT_REPORT.md DIV-001 fix. primus-core SPECIFICATION.md §7.
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

/// Custom error type for SDK operations including Node and Transport failures.
///
/// # Example
/// ```
/// use primus_sdk::error::PrimusSdkError;
/// let err = PrimusSdkError::SequenceMismatch;
/// assert_eq!(err.to_string(), "Node rejected transaction due to sequence mismatch");
/// ```
#[derive(Debug)]
pub enum PrimusSdkError {
    SequenceMismatch,
    ThermalThrottling { cooldown_secs: u64 },
    NodeUnreachable,
    NodeError { reason: String },
    Transport(String),
    ProofTooOld {
        proof_height: u64,
        current_height: u64,
    },
}

impl fmt::Display for PrimusSdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequenceMismatch => write!(f, "Node rejected transaction due to sequence mismatch"),
            Self::ThermalThrottling { cooldown_secs } => write!(f, "Node returned thermal throttle signal. Cooldown: {}s", cooldown_secs),
            Self::NodeUnreachable => write!(f, "Node is unreachable"),
            Self::NodeError { reason } => write!(f, "Generic node error: {}", reason),
            Self::Transport(err) => write!(f, "Transport error: {}", err),
            Self::ProofTooOld { proof_height, current_height } => {
                write!(f, "Merkle proof is too old (proof height: {}, current height: {})", proof_height, current_height)
            }
        }
    }
}

impl std::error::Error for PrimusSdkError {}
