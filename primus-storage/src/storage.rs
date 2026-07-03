/// Returned when a proof is requested for a crystal older than UNDO_WINDOW.
/// The MPT root for that crystal has been pruned from Sled.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Proof too old: crystal #{index} is more than {window} blocks behind \
             current tip #{tip}. Increase UNDO_WINDOW or request a more recent proof.")]
    ProofTooOld { index: u64, tip: u64, window: u64 },

    #[error("Crystal #{0} not found in storage")]
    CrystalNotFound(u64),

    #[error(transparent)]
    Sled(#[from] sled::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
