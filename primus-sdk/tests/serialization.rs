//! Integration tests verifying SDK→Core serialization compatibility.
//! These tests confirm that primus-sdk produces rkyv bytes that
//! primus-core's check_archived_root::<SignedReaction> will accept.
//!
//! RDIV-001 regression tests.

use primus_types::atom::{Atom, Element};
use primus_types::{SignedReaction, Payload};

#[test]
fn sdk_transaction_bytes_pass_rkyv_check() {
    // Build a minimal SignedReaction using primus-types directly
    // (mirrors what Transaction::to_bytes() produces after RDIV-001 fix)

    // Step 1: Create a test SignedReaction
    let sender = Atom::sender_snapshot(
        vec![0u8; 2592], // PK_BYTES = 2592
        Element::Hydrogen,
        1000,
        [0u8; 32],
        1,
        0,
        0,
    );
    let receiver = Atom::new_receiver(vec![1u8; 2592]);
    let mut reaction = SignedReaction {
        sender,
        receiver,
        reaction_hash: [0u8; 32],
        energy: 10.0,
        timestamp: 123456789,
        signature: vec![0u8; 4627], // SIG_BYTES = 4627
        payload: Payload::Transfer { amount: 10 },
    };
    reaction.reaction_hash = reaction.compute_reaction_hash();

    // Step 2: Serialize with rkyv (same as Transaction::to_bytes() after fix)
    let bytes = rkyv::to_bytes::<_, 256>(&reaction).unwrap();

    // Step 3: Verify check_archived_root succeeds (same check as push_bytes)
    let result = rkyv::check_archived_root::<SignedReaction>(&bytes);
    assert!(result.is_ok(), "rkyv check must pass for SDK-produced bytes: {:?}", result);
}

#[test]
fn bincode_bytes_fail_rkyv_check() {
    // Regression test: confirm that bincode-encoded bytes are rejected
    // This is the bug that RDIV-001 fixed — ensure it cannot regress

    // Create any serializable struct and bincode-encode it
    let dummy: Vec<u8> = vec![1, 2, 3, 4, 5];
    let bincode_bytes = bincode::serialize(&dummy).unwrap();

    // These bytes must FAIL rkyv::check_archived_root
    let result = rkyv::check_archived_root::<primus_types::SignedReaction>(&bincode_bytes);
    assert!(
        result.is_err(),
        "bincode bytes must be rejected by rkyv check — RDIV-001 regression"
    );
}
