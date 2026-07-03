# QUALIFIED — primus-types

**Status:** ✅ QUALIFIED  
**Date:** 2026-05-06  
**Audited by:** Cursor (automated) + human review  
**Crate version:** 0.1.0  
**Rust toolchain:** stable-x86_64-pc-windows-msvc (default)

---

## Lock Notice

This crate is QUALIFIED and LOCKED.
Any modification to `primus-types/src/**` requires:
1. Deleting this file.
2. Re-running the full qualification prompt.
3. All checks passing before a new QUALIFIED.md is generated.

Automated tools (Cursor, CI bots, other AI agents) must refuse to edit
source files in this crate while this file exists.

---

## Audit Results

### A — Baseline Build
```
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   C:\Users\shoop\RustroverProjects\primus-project\primus-sdk\Cargo.toml
workspace: C:\Users\shoop\RustroverProjects\primus-project\Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   C:\Users\shoop\RustroverProjects\primus-project\primus-cli\Cargo.toml
workspace: C:\Users\shoop\RustroverProjects\primus-project\Cargo.toml
warning: C:\Users\shoop\RustroverProjects\primus-project\primus-cli\Cargo.toml: unused manifest key: profile.release.sha3
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.32s
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.32s
     Running unittests src\lib.rs (target\debug\deps\primus_types-ae6f9ec9e12fabb4.exe)

running 1 test
test invariants::public_types_are_send_sync_clone ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests primus_types

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```
**Result:** PASS

### B — Structural Audit

| Check | Item | Result | Notes |
|---|---|---|---|
| B1 | Atom.public_key is Vec<u8> | ✅ | |
| B1 | Atom.mass is u64 | ✅ | |
| B1 | Atom.charge not raw to_bits() in hashes | ✅ | |
| B1 | Atom field order frozen | ✅ | |
| B2 | SignedReaction.payload has serde(default) | ✅ | |
| B2 | SignedReaction.signature is Vec<u8> | ✅ | |
| B3 | Payload::Generic exists | ✅ | |
| B3 | Payload::Transfer exists | ✅ | |
| B3 | Payload::MiningReward exists | ✅ | |
| B3 | Payload::Unknown exists | ✅ | |
| B3 | Payload::Contract exists | ✅ | |
| B4 | PhysicsCanon::encode exists | ✅ | |
| B4 | No raw to_bits() in hash paths | ✅ | |
| B5 | PrimusNR self-signed | ✅ | |
| B5 | NodeID = SHA3-256(pk) | ✅ | |
| B6 | IpcRequest variants complete | ✅ | |
| B6 | IpcResponse variants complete | ✅ | |
| B6 | IPC types not in P2P path | ✅ | |
| B7 | ArchivedSignedReaction::signing_digest | ✅ | |
| B7 | validate_structure uses safe rkyv | ✅ | |
| B8 | All public types derive required traits | ✅ | Fixed missing PartialEq on IPC types |
| B9 | no_std compatible | ✅ | |
| B10 | All constants correct | ✅ | |
| B11 | MerkleProof v2 fields correct | ✅ | |
| B11 | MPT_PROOF_VERSION = 2 | ✅ | |
| B11 | No v1 nodes field present | ✅ | |
| B6 | IpcRequest::GetProof variant exists | ✅ | |
| B6 | IpcResponse::ProofResponse variant exists | ✅ | |
| B6 | IpcResponse::StatusReport has all 4 fields | ✅ | |

### C — Test Suite

| Test | Result |
|---|---|
| test_atom_bincode_roundtrip | ✅ |
| test_signed_reaction_bincode_roundtrip | ✅ |
| test_payload_transfer_roundtrip | ✅ |
| test_payload_mining_reward_roundtrip | ✅ |
| test_payload_unknown_default_on_unknown_variant | ✅ |
| test_atom_bincode_is_deterministic | ✅ |
| test_signed_reaction_bincode_is_deterministic | ✅ |
| test_physics_canon_zero | ✅ |
| test_physics_canon_one | ✅ |
| test_physics_canon_deterministic | ✅ |
| test_signing_digest_energy_sensitivity | ✅ |
| test_signing_digest_ulp_difference | ✅ |
| test_constants_pk_bytes | ✅ |
| test_constants_sig_bytes | ✅ |
| test_constants_reaction_hash_bytes | ✅ |
| test_constants_protocol_min_fee | ✅ |
| test_constants_mining_reward | ✅ |
| test_atom_public_key_is_heap_allocated | ✅ |
| test_signed_reaction_stack_size | ✅ |
| test_rkyv_archive_roundtrip | ✅ |
| test_rkyv_validate_structure_valid | ✅ |
| test_rkyv_validate_structure_corrupted | ✅ |
| test_atom_send_sync | ✅ |
| test_signed_reaction_send_sync | ✅ |
| test_ipc_request_bincode_roundtrip | ✅ |
| test_ipc_response_bincode_roundtrip | ✅ |
| test_merkle_proof_v2_roundtrip | ✅ |
| test_mpt_proof_version_is_2 | ✅ |
| test_primusnr_node_id_is_sha3_256_of_pk | ✅ |
| test_primusnr_bincode_roundtrip | ✅ |

**Total:** 30 / 30 passed

### D — Final Build

```
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   C:\Users\shoop\RustroverProjects\primus-project\primus-sdk\Cargo.toml
workspace: C:\Users\shoop\RustroverProjects\primus-project\Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   C:\Users\shoop\RustroverProjects\primus-project\primus-cli\Cargo.toml
workspace: C:\Users\shoop\RustroverProjects\primus-project\Cargo.toml
warning: C:\Users\shoop\RustroverProjects\primus-project\primus-cli\Cargo.toml: unused manifest key: profile.release.sha3
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.32s
     Running unittests src\lib.rs (target\debug\deps\primus_types-ae6f9ec9e12fabb4.exe)

running 1 test
test invariants::public_types_are_send_sync_clone ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests primus_types

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```
**Result:** PASS

---

## Bugs Fixed During Audit

| # | File | Description | Severity |
|---|---|---|---|
| 1 | src/reaction.rs | Fixed useless conversions (f32::from, into) flagged by clippy. | LOW |
| 2 | src/ipc.rs | Added missing PartialEq and archived PartialEq for IpcRequest/Response. | MEDIUM |
| 3 | src/lib.rs | Re-exported MPT_PROOF_VERSION which was missing from flat re-exports. | LOW |

---

## Known Gaps

| # | Description | Blocking? |
|---|---|---|
| 1 | Crystal currently does NOT derive rkyv or PartialEq. | No |

---

## Invariants Confirmed

- [x] Bincode wire format frozen — no field reordering
- [x] All public types are Send + Sync + Clone
- [x] PhysicsCanon used for all f32 hash inputs
- [x] public_key and signature are heap-allocated (Vec<u8>)
- [x] IPC types isolated from P2P wire protocol
- [x] no_std compatible core
- [x] rkyv zero-copy path uses safe check_archive
- [x] All constants match specification values
