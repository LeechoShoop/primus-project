use primus_types::MerkleProof;

pub struct ProofBuilder;

impl ProofBuilder {
    /// Verify a proof without any storage access.
    /// Pure function — callable in WASM.
    pub fn verify(proof: &MerkleProof) -> bool {
        crate::mpt::verify_proof(proof)
    }
}
