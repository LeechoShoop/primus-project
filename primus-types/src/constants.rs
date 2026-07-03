// =============================================================================
// primus-types/src/constants.rs
//
// Every numeric constant that appears in the wire format or protocol logic
// must be defined here and only here. If primus-core and primus-sdk each
// define PK_BYTES = 2592 independently, they will drift. One will be updated
// when ml-dsa changes parameter sets; the other won't. A 2-byte mismatch in
// a byte-length check is a security hole, not a compile error.
// =============================================================================

/// ML-DSA-87 verifying key size in bytes.
/// Source: FIPS 204 Table 2, parameter set ML-DSA-87.
pub const PK_BYTES: usize = 2592;

/// ML-DSA-87 signature size in bytes.
/// Source: FIPS 204 Table 2, parameter set ML-DSA-87.
pub const SIG_BYTES: usize = 4627;

/// SHA3-256 output size. Used for reaction_hash and last_reaction_hash.
pub const REACTION_HASH_BYTES: usize = 32;

/// Minimum network fee in mass units. Burned by the protocol on every Transfer.
///
/// The PVM enforces this floor; the SDK's TransactionBuilder defaults to it.
/// Compared via `PhysicsCanon::encode(energy) >= PROTOCOL_MIN_FEE` — never
/// via raw `f32 as u64` truncation.
/// Never set to zero — a zero fee makes the mempool vulnerable to spam.
pub const PROTOCOL_MIN_FEE: u64 = 10;

/// Mass credited to the Architect per confirmed crystal (block reward).
/// Injected as `Payload::MiningReward` by `engine::build_mining_reward_rx()`.
pub const MINING_REWARD_AMOUNT: u64 = 10;

/// Domain separation tag for MiningReward reaction hash derivation.
///
/// Full derivation: `SHA3-256(MINING_REWARD_TAG || crystal_index_le8 || architect_pk)`
///
/// Any MiningReward whose reaction_hash does not match this formula must be
/// rejected by the PVM before signature verification is attempted.
pub const MINING_REWARD_TAG: &[u8] = b"PRIMUS_MINING_REWARD_V1";

/// Domain separation tag used by the SDK's `derive_seed()` function.
///
/// Defined here — not in primus-sdk — so that the derivation path is
/// auditable from primus-types and cannot silently diverge between the
/// SDK and any future key derivation tooling.
pub const SEED_DOMAIN_TAG: &[u8] = b"PRIMUS_SDK";

/// MPT proof wire format version. Bump when MerkleProof fields change.
/// v1 = full nodes (Vec<Vec<u8>>)
/// v2 = compact siblings (Vec<[u8;32]> + Vec<PathStep>)
pub const MPT_PROOF_VERSION: u8 = 2;
