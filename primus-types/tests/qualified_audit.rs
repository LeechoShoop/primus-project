// primus-types/tests/qualified_audit.rs

use primus_types::*;
use primus_types::constants::MPT_PROOF_VERSION;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_test_atom() -> Atom {
    Atom {
        public_key: vec![0u8; 2592],
        element: Element::Hydrogen,
        neutron_count: 0,
        mass: 1_000,
        charge: 0.0,
        last_reaction_hash: [0u8; 32],
        last_active_index: 0,
        nonce: 0,
        quantum_state: QuantumState::Stable,
    }
}

fn make_test_reaction() -> SignedReaction {
    let mut rx = SignedReaction {
        sender:        make_test_atom(),
        receiver:      make_test_atom(),
        reaction_hash: [0u8; 32],
        energy:        10.0,
        timestamp:     0,
        signature:     vec![0u8; 4627],
        payload:       Payload::Generic,
    };
    // Set correct reaction_hash so validate_structure() passes
    rx.reaction_hash = rx.compute_reaction_hash();
    rx
}

// ── SERIALIZATION ROUND-TRIP ─────────────────────────────────────────────────

#[test]
fn test_atom_bincode_roundtrip() {
    let atom = make_test_atom();
    let bytes = bincode::serialize(&atom).unwrap();
    let decoded: Atom = bincode::deserialize(&bytes).unwrap();
    assert_eq!(atom, decoded);
}

#[test]
fn test_signed_reaction_bincode_roundtrip() {
    let rx = make_test_reaction();
    let bytes = bincode::serialize(&rx).unwrap();
    let decoded: SignedReaction = bincode::deserialize(&bytes).unwrap();
    assert_eq!(rx, decoded);
}

#[test]
fn test_payload_transfer_roundtrip() {
    let p = Payload::Transfer { amount: 500 };
    let bytes = bincode::serialize(&p).unwrap();
    let decoded: Payload = bincode::deserialize(&bytes).unwrap();
    assert_eq!(p, decoded);
}

#[test]
fn test_payload_mining_reward_roundtrip() {
    let p = Payload::MiningReward { amount: 10 };
    let bytes = bincode::serialize(&p).unwrap();
    let decoded: Payload = bincode::deserialize(&bytes).unwrap();
    assert_eq!(p, decoded);
}

#[test]
fn test_payload_contract_roundtrip() {
    let p = Payload::Contract { code: vec![0x00, 0x61, 0x73, 0x6d] }; // WASM magic
    let bytes = bincode::serialize(&p).unwrap();
    let decoded: Payload = bincode::deserialize(&bytes).unwrap();
    assert_eq!(p, decoded);
}

#[test]
fn test_payload_contract_call_roundtrip() {
    let p = Payload::ContractCall {
        address: vec![1u8; 2592],
        data: vec![0xde, 0xad],
    };
    let bytes = bincode::serialize(&p).unwrap();
    let decoded: Payload = bincode::deserialize(&bytes).unwrap();
    assert_eq!(p, decoded);
}

/// Payload discriminants must be frozen at their wire values.
/// Generic=0, Transfer=1, MiningReward=2, Contract=3, ContractCall=4.
#[test]
fn test_payload_wire_discriminants() {
    // bincode encodes enum discriminants as u32 LE
    let generic_bytes   = bincode::serialize(&Payload::Generic).unwrap();
    let transfer_bytes  = bincode::serialize(&Payload::Transfer { amount: 0 }).unwrap();
    let reward_bytes    = bincode::serialize(&Payload::MiningReward { amount: 0 }).unwrap();
    let contract_bytes  = bincode::serialize(&Payload::Contract { code: vec![] }).unwrap();
    let call_bytes      = bincode::serialize(&Payload::ContractCall { address: vec![], data: vec![] }).unwrap();

    assert_eq!(&generic_bytes[..4],  &[0, 0, 0, 0], "Generic must be discriminant 0");
    assert_eq!(&transfer_bytes[..4], &[1, 0, 0, 0], "Transfer must be discriminant 1");
    assert_eq!(&reward_bytes[..4],   &[2, 0, 0, 0], "MiningReward must be discriminant 2");
    assert_eq!(&contract_bytes[..4], &[3, 0, 0, 0], "Contract must be discriminant 3");
    assert_eq!(&call_bytes[..4],     &[4, 0, 0, 0], "ContractCall must be discriminant 4");
}

#[test]
fn test_payload_unknown_default_on_unknown_variant() {
    let mut bytes = bincode::serialize(&Payload::Generic).unwrap();
    bytes[0] = 99; // unknown discriminant (99 > 5)
    let result: Payload = bincode::deserialize(&bytes).unwrap();
    assert_eq!(result, Payload::Unknown);
}

/// requires_signature_verification: false for MiningReward and Unknown.
#[test]
fn test_payload_signature_requirement() {
    assert!(Payload::Generic.requires_signature_verification());
    assert!(Payload::Transfer { amount: 1 }.requires_signature_verification());
    assert!(!Payload::MiningReward { amount: 1 }.requires_signature_verification());
    assert!(!Payload::Unknown.requires_signature_verification());
    assert!(Payload::Contract { code: vec![] }.requires_signature_verification());
    assert!(Payload::ContractCall { address: vec![], data: vec![] }.requires_signature_verification());
}

/// transfer_amount: Some only for Transfer and MiningReward.
#[test]
fn test_payload_transfer_amount() {
    assert_eq!(Payload::Transfer { amount: 42 }.transfer_amount(), Some(42));
    assert_eq!(Payload::MiningReward { amount: 10 }.transfer_amount(), Some(10));
    assert_eq!(Payload::Generic.transfer_amount(), None);
    assert_eq!(Payload::Contract { code: vec![] }.transfer_amount(), None);
    assert_eq!(Payload::ContractCall { address: vec![], data: vec![] }.transfer_amount(), None);
}

// ── FIELD ORDER / WIRE STABILITY ─────────────────────────────────────────────

#[test]
fn test_atom_bincode_is_deterministic() {
    let atom = make_test_atom();
    let a = bincode::serialize(&atom).unwrap();
    let b = bincode::serialize(&atom).unwrap();
    assert_eq!(a, b);
}

#[test]
fn test_signed_reaction_bincode_is_deterministic() {
    let rx = make_test_reaction();
    let a = bincode::serialize(&rx).unwrap();
    let b = bincode::serialize(&rx).unwrap();
    assert_eq!(a, b);
}

// ── PHYSICS CANON ────────────────────────────────────────────────────────────

#[test]
fn test_physics_canon_zero() {
    assert_eq!(PhysicsCanon::encode(0.0), 0);
}

#[test]
fn test_physics_canon_one() {
    assert_eq!(PhysicsCanon::encode(1.0), 1_000_000_000);
}

#[test]
fn test_physics_canon_deterministic() {
    let a = PhysicsCanon::encode(3.14159);
    let b = PhysicsCanon::encode(3.14159);
    assert_eq!(a, b);
}

/// Negative values must clamp to zero (not wrap).
#[test]
fn test_physics_canon_negative_clamp() {
    assert_eq!(PhysicsCanon::encode(-1.0), 0);
    assert_eq!(PhysicsCanon::encode(-999.9), 0);
}

/// Large values must not overflow — return u64::MAX instead of wrapping.
#[test]
fn test_physics_canon_overflow_guard() {
    let result = PhysicsCanon::encode(f32::MAX);
    assert_eq!(result, u64::MAX, "Large f32 must saturate to u64::MAX");
}

/// decode(encode(x)) round-trips within 1 ULP tolerance.
#[test]
fn test_physics_canon_decode_roundtrip() {
    let original = 150.5f32;
    let encoded  = PhysicsCanon::encode(original);
    let decoded  = PhysicsCanon::decode(encoded);
    assert!((decoded - original).abs() < 1e-6,
        "decode(encode(x)) should round-trip: got {decoded}, expected {original}");
}

#[test]
fn test_signing_digest_energy_sensitivity() {
    let mut rx1 = make_test_reaction();
    rx1.energy = 10.0;
    rx1.reaction_hash = rx1.compute_reaction_hash();

    let mut rx2 = rx1.clone();
    rx2.energy = 20.0;
    rx2.reaction_hash = rx2.compute_reaction_hash();

    assert_ne!(rx1.signing_digest(), rx2.signing_digest());
}

#[test]
fn test_signing_digest_ulp_difference() {
    let mut rx1 = make_test_reaction();
    rx1.energy = 10.0_f32;
    rx1.reaction_hash = rx1.compute_reaction_hash();

    // f32::from_bits(rx1.energy.to_bits() + 1) is one ULP above 10.0
    let mut rx2 = rx1.clone();
    rx2.energy = f32::from_bits(10.0_f32.to_bits() + 1);
    rx2.reaction_hash = rx2.compute_reaction_hash();

    // PhysicsCanon encodes both to different u64 values (1 ULP = ~1 unit difference)
    let enc1 = PhysicsCanon::encode(rx1.energy);
    let enc2 = PhysicsCanon::encode(rx2.energy);
    // The encoded values may or may not differ by 1 depending on scale,
    // but the digests must be recomputable and consistent
    assert_eq!(rx1.signing_digest(), rx1.signing_digest(), "digest must be deterministic");
    assert_eq!(rx2.signing_digest(), rx2.signing_digest(), "digest must be deterministic");
    // If ULP changes the encoding, digests differ
    if enc1 != enc2 {
        assert_ne!(rx1.signing_digest(), rx2.signing_digest());
    }
}

// ── VALIDATE_STRUCTURE ───────────────────────────────────────────────────────

#[test]
fn test_validate_structure_valid() {
    let rx = make_test_reaction();
    assert!(rx.validate_structure().is_ok());
}

#[test]
fn test_validate_structure_bad_pk_length() {
    let mut rx = make_test_reaction();
    rx.sender.public_key = vec![0u8; 100]; // wrong length
    // reaction_hash will now mismatch too, but pk check comes first
    assert!(matches!(rx.validate_structure(),
        Err(PrimusError::InvalidPublicKeyLength { .. })));
}

#[test]
fn test_validate_structure_fee_below_minimum() {
    let mut rx = make_test_reaction();
    rx.energy = 0.0; // below PROTOCOL_MIN_FEE = 10
    rx.reaction_hash = rx.compute_reaction_hash();
    assert!(matches!(rx.validate_structure(),
        Err(PrimusError::FeeBelowMinimum { .. })));
}

#[test]
fn test_validate_structure_hash_mismatch() {
    let mut rx = make_test_reaction();
    rx.reaction_hash = [0xde; 32]; // deliberately wrong
    assert!(matches!(rx.validate_structure(),
        Err(PrimusError::ReactionHashMismatch { .. })));
}

#[test]
fn test_mining_reward_exempts_signature_check() {
    // MiningReward with empty signature should not fail on SIG_BYTES check
    let mut rx = make_test_reaction();
    rx.payload = Payload::MiningReward { amount: 10 };
    rx.signature = vec![]; // empty — exempt
    rx.reaction_hash = rx.compute_reaction_hash();
    // Should not return InvalidSignatureLength
    let result = rx.validate_structure();
    assert!(!matches!(result, Err(PrimusError::InvalidSignatureLength { .. })));
}

// ── CONSTANTS ────────────────────────────────────────────────────────────────

#[test]
fn test_constants_pk_bytes()            { assert_eq!(PK_BYTES, 2592); }

#[test]
fn test_constants_sig_bytes()           { assert_eq!(SIG_BYTES, 4627); }

#[test]
fn test_constants_reaction_hash_bytes() { assert_eq!(REACTION_HASH_BYTES, 32); }

#[test]
fn test_constants_protocol_min_fee()    { assert_eq!(PROTOCOL_MIN_FEE, 10); }

#[test]
fn test_constants_mining_reward()       { assert_eq!(MINING_REWARD_AMOUNT, 10); }

#[test]
fn test_constants_mpt_proof_version()  { assert_eq!(MPT_PROOF_VERSION, 2u8); }

// ── MEMORY / STACK SAFETY ────────────────────────────────────────────────────

#[test]
fn test_atom_public_key_is_heap_allocated() {
    assert!(std::mem::size_of::<Atom>() < 512,
        "Atom is suspiciously large — public_key may be stack-allocated. Got: {}",
        std::mem::size_of::<Atom>());
}

#[test]
fn test_signed_reaction_stack_size() {
    assert!(std::mem::size_of::<SignedReaction>() < 1024,
        "SignedReaction is too large for stack — check for fixed arrays. Got: {}",
        std::mem::size_of::<SignedReaction>());
}

// ── RKYV ZERO-COPY ───────────────────────────────────────────────────────────

#[test]
fn test_rkyv_archive_roundtrip() {
    let rx = make_test_reaction();
    let bytes = rkyv::to_bytes::<_, 256>(&rx).unwrap();
    let archived = rkyv::check_archived_root::<SignedReaction>(&bytes).unwrap();
    // Basic field access zero-copy
    assert_eq!(archived.energy, rx.energy);
    assert_eq!(archived.timestamp, rx.timestamp);
}

#[test]
fn test_rkyv_validate_structure_valid() {
    let rx = make_test_reaction();
    let bytes = rkyv::to_bytes::<_, 256>(&rx).unwrap();
    let archived = rkyv::check_archived_root::<SignedReaction>(&bytes).unwrap();
    assert!(archived.validate_structure().is_ok());
}

#[test]
fn test_rkyv_validate_structure_corrupted() {
    let rx = make_test_reaction();
    let mut bytes = rkyv::to_bytes::<_, 256>(&rx).unwrap();
    // Corrupt the Payload discriminant (last field in SignedReaction)
    // In rkyv 0.7, enums are usually archived with their discriminant first.
    // We'll just corrupt several bytes at the end and hope to hit it, 
    // or better, target the Element discriminant in the Atom which is fixed size.
    // Element is at the start of Atom (after public_key Vec).
    // Let's just flip bits everywhere in a range.
    for i in (bytes.len() - 10)..bytes.len() {
        bytes[i] ^= 0xFF;
    }
    let result = rkyv::check_archived_root::<SignedReaction>(&bytes);
    assert!(result.is_err(), "Corrupted bytes should fail rkyv structural check");
}

#[test]
fn test_from_bytes_zero_copy_valid() {
    let rx = make_test_reaction();
    let bytes = rkyv::to_bytes::<_, 256>(&rx).unwrap();
    let result = SignedReaction::from_bytes_zero_copy(&bytes);
    assert!(result.is_ok());
}

#[test]
fn test_from_bytes_zero_copy_corrupted() {
    let rx = make_test_reaction();
    let mut bytes = rkyv::to_bytes::<_, 256>(&rx).unwrap();
    for i in (bytes.len() - 10)..bytes.len() {
        bytes[i] ^= 0xFF;
    }
    let result = SignedReaction::from_bytes_zero_copy(&bytes);
    assert!(matches!(result, Err(PrimusError::DeserializationFailed { .. })));
}

// ── SEND + SYNC ───────────────────────────────────────────────────────────────

#[test]
fn test_atom_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Atom>();
}

#[test]
fn test_signed_reaction_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SignedReaction>();
}

#[test]
fn test_payload_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Payload>();
}

#[test]
fn test_primusnr_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PrimusNR>();
}

// ── IPC ──────────────────────────────────────────────────────────────────────
/*
#[test]
fn test_ipc_request_bincode_roundtrip() {
    let variants: Vec<IpcRequest> = vec![
        IpcRequest::Status,
        IpcRequest::GetChallenge,
        IpcRequest::AdminShutdown { signature: vec![0u8; 4627] },
        IpcRequest::AdminConnectPeer {
            addr: "127.0.0.1:9000".to_string(),
            signature: vec![0u8; 4627],
        },
        IpcRequest::GetProof { address: vec![0u8; 2592] },
    ];
    for req in variants {
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: IpcRequest = bincode::deserialize(&bytes).unwrap();
        assert_eq!(req, decoded);
    }
}

#[test]
fn test_ipc_response_bincode_roundtrip() {
    let variants: Vec<IpcResponse> = vec![
        IpcResponse::Ok,
        IpcResponse::Error("test error".to_string()),
        IpcResponse::Challenge(vec![0u8; 32]),
        IpcResponse::StatusReport {
            height: 42,
            peers: 7,
            cache_size: 100,
            frame_drops: 3,
        },
    ];
    for resp in variants {
        let bytes = bincode::serialize(&resp).unwrap();
        let decoded: IpcResponse = bincode::deserialize(&bytes).unwrap();
        assert_eq!(resp, decoded);
    }
}
*/
// ── MERKLE PROOF ─────────────────────────────────────────────────────────────

#[test]
fn test_merkle_proof_v2_roundtrip() {
    use primus_types::proof::{MerkleProof, PathStep};
    let proof = MerkleProof {
        trie_key: [1u8; 32],
        value: Some(vec![0xAB, 0xCD]),
        root: [2u8; 32],
        siblings: vec![[3u8; 32], [4u8; 32]],
        path: vec![
            PathStep::Branch { nibble: 0 },
            PathStep::Extension { len: 2 },
            PathStep::Leaf,
        ],
    };
    let bytes = bincode::serialize(&proof).unwrap();
    let decoded: MerkleProof = bincode::deserialize(&bytes).unwrap();
    assert_eq!(proof, decoded);
}

#[test]
fn test_merkle_proof_exclusion() {
    use primus_types::proof::{MerkleProof, PathStep};
    // Exclusion proof: value = None
    let proof = MerkleProof {
        trie_key: [5u8; 32],
        value: None,
        root: [6u8; 32],
        siblings: vec![],
        path: vec![PathStep::Leaf],
    };
    let bytes = bincode::serialize(&proof).unwrap();
    let decoded: MerkleProof = bincode::deserialize(&bytes).unwrap();
    assert_eq!(proof.value, decoded.value);
    assert!(decoded.value.is_none());
}

// ── PRIMUSNR ─────────────────────────────────────────────────────────────────

#[test]
fn test_primusnr_node_id_is_sha3_256_of_pk() {
    use sha3::{Digest, Sha3_256};
    let pk = vec![42u8; 2592];
    let nr = PrimusNR {
        public_key: pk.clone(),
        addr_ip: 0,
        addr_port: 9000,
        signature: vec![0u8; 4627],
        timestamp: 0,
    };
    let expected: [u8; 32] = Sha3_256::digest(&pk).into();
    assert_eq!(nr.node_id(), expected);
}

#[test]
fn test_primusnr_bincode_roundtrip() {
    let nr = PrimusNR {
        public_key: vec![0u8; 2592],
        addr_ip: 0x0000_0000_0000_0000_0000_ffff_7f00_0001u128, // 127.0.0.1
        addr_port: 9000,
        signature: vec![0u8; 4627],
        timestamp: 1_700_000_000,
    };
    let bytes = bincode::serialize(&nr).unwrap();
    let decoded: PrimusNR = bincode::deserialize(&bytes).unwrap();
    assert_eq!(nr, decoded);
}

// ── GALACTIC SYNC ─────────────────────────────────────────────────────────────

#[test]
fn test_galactic_status_dominance() {
    let higher = GalacticStatus::from_state(100, 0.5);
    let lower  = GalacticStatus::from_state(50,  0.5);
    assert!(higher.is_more_dominant_than(&lower));
    assert!(!lower.is_more_dominant_than(&higher));
}

#[test]
fn test_galactic_status_tiebreak_by_energy() {
    let a = GalacticStatus::from_engine(100, 0.5, 1000.0);
    let b = GalacticStatus::from_engine(100, 0.5, 500.0);
    assert!(a.is_more_dominant_than(&b));
    assert!(!b.is_more_dominant_than(&a));
}
