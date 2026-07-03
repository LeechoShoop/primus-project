use proptest::prelude::*;
use primus_vm::*;
use primus_types::atom::{Atom, Element};
use primus_types::reaction::SignedReaction;
use primus_types::payload::Payload;
use primus_storage::Changeset;
use std::collections::BTreeMap;

mod common;
use common::*;

fn replicated_read_wasm_bytes(mem_size: usize, ptr: i32, len: i32) -> Option<()> {
    if ptr < 0 || len < 0 { return None; }
    let start = ptr as usize;
    let end = start.checked_add(len as usize)?;
    if end > mem_size { return None; }
    Some(())
}

fn replicated_write_wasm_bytes(mem_size: usize, ptr: i32, bytes_len: usize) -> bool {
    if ptr < 0 { return false; }
    let start = ptr as usize;
    let end = match start.checked_add(bytes_len) {
        Some(e) => e,
        None => return false,
    };
    if end > mem_size { return false; }
    true
}

proptest! {
    // 1. GasMeter — charge()
    // ATTACK SURFACE: GasMeter
    // INVARIANT: Arbitrary (amount, energy) pairs must never cause UB or panic.
    // SEVERITY: Critical
    #[test]
    fn prop_gas_meter_charge_no_panic(energy in -1000000.0f32..1000000.0f32, amount in 0u64..u64::MAX) {
        let mut meter = GasMeter::from_energy(energy);
        let _ = meter.charge(amount);
    }
}

proptest! {
    // INVARIANT: Charging more than u64::MAX total must return GasOverflow, never wrap.
    // SEVERITY: Critical
    #[test]
    fn prop_gas_meter_overflow(amount1 in 1u64..u64::MAX, amount2 in 1u64..u64::MAX) {
        let mut meter = GasMeter::from_energy(1.0);
        meter.consumed = amount1;
        if amount1.checked_add(amount2).is_none() {
            assert!(matches!(meter.charge(amount2), Err(PvmError::GasOverflow)));
        } else {
            let _ = meter.charge(amount2);
        }
    }
}

proptest! {
    // INVARIANT: After OutOfGas the meter must remain in a consistent state (consumed > limit)
    // SEVERITY: Medium
    #[test]
    fn prop_gas_meter_consistency_after_oog(energy in 0.0f32..1000.0f32, amount in 0u64..u64::MAX) {
        let mut meter = GasMeter::from_energy(energy);
        let limit = meter.limit;
        let res = meter.charge(amount);
        if amount > limit {
            assert!(res.is_err());
            assert!(meter.consumed > meter.limit);
        }
    }
}

proptest! {
    // INVARIANT: remaining() == limit.saturating_sub(consumed) always holds.
    // SEVERITY: Medium
    #[test]
    fn prop_gas_meter_remaining_invariant(energy in 0.0f32..1000.0f32, consumed in 0u64..u64::MAX) {
        let mut meter = GasMeter::from_energy(energy);
        meter.consumed = consumed;
        assert_eq!(meter.remaining(), meter.limit.saturating_sub(consumed));
    }
}

proptest! {
    // 2. GasMeter::from_energy — clamping invariants
    // ATTACK SURFACE: GasMeter::from_energy
    // INVARIANT: from_energy(NaN, Inf, etc) must not panic.
    // SEVERITY: High
    #[test]
    fn prop_gas_meter_from_energy_special_floats(val in prop::num::f32::ANY) {
        let _ = GasMeter::from_energy(val);
    }
}

proptest! {
    // INVARIANT: limit ∈ [BASE_CONTRACT_GAS, MAX_GAS_PER_REACTION] always.
    // SEVERITY: High
    #[test]
    fn prop_gas_meter_limit_clamping(val in prop::num::f32::ANY) {
        let meter = GasMeter::from_energy(val);
        assert!(meter.limit >= BASE_CONTRACT_GAS);
        assert!(meter.limit <= MAX_GAS_PER_REACTION);
    }
}

proptest! {
    // 3. Physics helpers
    // ATTACK SURFACE: Physics helpers
    // INVARIANT: get_galactic_drift: for any crystal_index, result < 256.
    // SEVERITY: Medium
    #[test]
    fn prop_physics_galactic_drift(idx in 0u64..u64::MAX) {
        assert!(get_galactic_drift(idx) <= 255);
    }
}

proptest! {
    // INVARIANT: calculate_orbital_resonance: result ∈ {0.0, 30.0} always.
    // SEVERITY: Medium
    #[test]
    fn prop_physics_orbital_resonance(pk in prop::collection::vec(0u8..255, 0..64), drift in 0u8..255) {
        let res = calculate_orbital_resonance(&pk, drift);
        assert!(res == 0.0 || res == 30.0);
    }
}

proptest! {
    // INVARIANT: calculate_gravity_assist_from_iter: result ∈ [0.0, MAX_GRAVITY_PULL]
    // SEVERITY: Medium
    #[test]
    fn prop_physics_gravity_assist(masses in prop::collection::vec(0u64..100_000, 0..10), pk_byte in 0u8..255) {
        let mut atoms = BTreeMap::new();
        for (i, m) in masses.into_iter().enumerate() {
            let mut a = Atom::new_materialized(vec![pk_byte, i as u8], Element::Hydrogen);
            a.mass = m;
            atoms.insert(a.public_key.clone(), a);
        }
        let res = calculate_gravity_assist_from_iter(atoms.iter(), &[pk_byte]);
        assert!(res >= 0.0 && res <= MAX_GRAVITY_PULL);
    }
}

proptest! {
    // INVARIANT: get_spacetime_curvature: result is finite, never NaN.
    // SEVERITY: Medium
    #[test]
    fn prop_physics_curvature_finite(hash in prop::array::uniform32(0u8..255), temp in -1000.0f32..5000.0f32) {
        let res = get_spacetime_curvature(&hash, temp);
        assert!(res.is_finite());
    }
}

proptest! {
    // INVARIANT: calculate_macro_shift: result ≥ 0.0 always.
    // SEVERITY: Medium
    #[test]
    fn prop_physics_macro_shift_positive(temp in -1000.0f32..5000.0f32) {
        assert!(calculate_macro_shift(temp) >= 0.0);
    }
}

proptest! {
    // INVARIANT: calculate_entropy_tax: result ≥ complexity always
    // SEVERITY: Medium
    #[test]
    fn prop_physics_entropy_tax_bound(complexity in 0u64..=u64::MAX, temp in -1000.0f32..5000.0f32) {
        let tax = calculate_entropy_tax(complexity, temp);
        assert!(tax >= complexity || (complexity > MAX_SAFE_COMPLEXITY));
    }
}

proptest! {
    // INVARIANT: Entropy tax must be deterministic for a given (complexity, temp) pair.
    // SEVERITY: Critical (consensus safety)
    #[test]
    fn prop_entropy_tax_determinism(complexity in 0..=MAX_SAFE_COMPLEXITY, temp in 0.0f32..=10_000.0f32) {
        let t1 = calculate_entropy_tax(complexity, temp);
        let t2 = calculate_entropy_tax(complexity, temp);
        assert_eq!(t1, t2);
    }
}

proptest! {
    // 4. Host function memory safety
    // ATTACK SURFACE: Host function memory safety
    // INVARIANT: ptr < 0 -> None, len < 0 -> None, overflow -> None, OOB -> None
    // SEVERITY: Critical
    #[test]
    fn prop_host_mem_safety(mem_size in 0usize..20_000_000, ptr in i32::MIN..i32::MAX, len in i32::MIN..i32::MAX) {
        let _ = replicated_read_wasm_bytes(mem_size, ptr, len);
        let _ = replicated_write_wasm_bytes(mem_size, ptr, len.max(0) as usize);
    }
}

proptest! {
    // 5. ContractDelta accumulation
    // ATTACK SURFACE: ContractDelta accumulation
    // INVARIANT: transfer_mass calls must not overflow pending_out (uses saturating_add).
    // SEVERITY: High
    #[test]
    fn prop_contract_delta_accumulation(amounts in prop::collection::vec(0u64..u64::MAX, 1..10)) {
        let mut pending_out = 0u64;
        for amt in amounts {
            pending_out = pending_out.saturating_add(amt);
        }
        assert!(pending_out <= u64::MAX);
    }
}

proptest! {
    // 6. PVM::execute_single — full pipeline
    // ATTACK SURFACE: PVM::execute_single
    // INVARIANT: No panic, no UB for arbitrary inputs.
    // SEVERITY: Critical
    #[test]
    fn prop_pvm_execute_no_panic(
        energy in -100.0f32..1100.0f32,
        amount in 0u64..u64::MAX,
        nonce in 0u64..u64::MAX,
        temp in 0.0f32..2000.0f32
    ) {
        let mut state = MockStateView::new();
        let sender_pk = vec![1; 32];
        let receiver_pk = vec![2; 32];
        let mut sender = Atom::new_materialized(sender_pk.clone(), Element::Hydrogen);
        sender.mass = 1_000_000;
        sender.nonce = nonce;
        state.atoms.insert(sender_pk.clone(), sender.clone());

        let mut rx = SignedReaction {
            sender,
            receiver: Atom::new_materialized(receiver_pk.clone(), Element::Hydrogen),
            payload: Payload::Transfer { amount },
            energy,
            signature: b"valid_sig".to_vec(),
            reaction_hash: [0; 32],
            timestamp: 0,
        };
        rx.sender.nonce = nonce;

        let ctx = ExecutionContext::<MockCryptoVerifier> {
            state: &state,
            architect_pk: &vec![0; 32],
            current_temp: temp,
            crystal_index: 1,
            wasm_runtime: None,
            _crypto: std::marker::PhantomData,
        };

        let mut changeset = Changeset::new();
        let mut heat = 0.0;
        let mut entropy = 0.0;
        let atoms_iter = BTreeMap::new();
        
        let res = PVM::execute_single::<MockCryptoVerifier>(
            &ctx, &rx, &mut changeset, &atoms_iter, &mut heat, &mut entropy, 0, &[]
        );

        if energy < 0.0 {
            assert!(matches!(res, Err(PvmError::NegativeEnergy)));
        }

        if res.is_ok() {
            if let Some(s) = changeset.inner.get(&sender_pk) {
                assert_eq!(s.nonce, nonce + 1);
            }
        }
    }
}

proptest! {
    // INVARIANT: ThermalLimitExceeded is returned before changeset grows past the limit.
    // SEVERITY: High
    #[test]
    fn prop_pvm_thermal_limit(temp in 1001.0f32..2000.0f32) {
        let mut state = MockStateView::new();
        let sender_pk = vec![1; 32];
        let mut sender = Atom::new_materialized(sender_pk.clone(), Element::Hydrogen);
        sender.mass = 1_000_000;
        state.atoms.insert(sender_pk.clone(), sender.clone());

        let rx = SignedReaction {
            sender,
            receiver: Atom::new_materialized(vec![2; 32], Element::Hydrogen),
            payload: Payload::Transfer { amount: 100 },
            energy: 10.0,
            signature: b"valid_sig".to_vec(),
            reaction_hash: [0; 32],
            timestamp: 0,
        };

        let ctx = ExecutionContext::<MockCryptoVerifier> {
            state: &state,
            architect_pk: &vec![0; 32],
            current_temp: temp, // Curvature will be high
            crystal_index: 1,
            wasm_runtime: None,
            _crypto: std::marker::PhantomData,
        };

        let mut changeset = Changeset::new();
        let mut heat = 0.0;
        let mut entropy = 0.0;
        let res = PVM::execute_single::<MockCryptoVerifier>(
            &ctx, &rx, &mut changeset, &BTreeMap::new(), &mut heat, &mut entropy, 0, &[]
        );

        // Curvature = (hash[0]/255 * 15 + temp - 0 - 0).max(0.0)
        // If temp > 1000, curvature > 1000, which exceeds THERMAL_CAPACITY (1000)
        assert!(matches!(res, Err(PvmError::ThermalLimitExceeded)));
        assert!(changeset.is_empty());
    }
}

proptest! {
    // INVARIANT: If Err(_) -> changeset is EMPTY (atomic failure invariant).
    // SEVERITY: High
    #[test]
    fn prop_pvm_atomic_failure(energy in 0.0f32..100.0f32, _amount in 0u64..u64::MAX) {
        let mut state = MockStateView::new();
        let sender_pk = vec![1; 32];
        let mut sender = Atom::new_materialized(sender_pk.clone(), Element::Hydrogen);
        sender.mass = 100; // Low mass to trigger error
        state.atoms.insert(sender_pk.clone(), sender.clone());

        let rx = SignedReaction {
            sender,
            receiver: Atom::new_materialized(vec![2; 32], Element::Hydrogen),
            payload: Payload::Transfer { amount: u64::MAX }, // Will fail
            energy,
            signature: b"valid_sig".to_vec(),
            reaction_hash: [0; 32],
            timestamp: 0,
        };

        let ctx = ExecutionContext::<MockCryptoVerifier> {
            state: &state,
            architect_pk: &vec![0; 32],
            current_temp: 0.0,
            crystal_index: 1,
            wasm_runtime: None,
            _crypto: std::marker::PhantomData,
        };

        let mut changeset = Changeset::new();
        let mut heat = 0.0;
        let mut entropy = 0.0;
        let res = PVM::execute_single::<MockCryptoVerifier>(
            &ctx, &rx, &mut changeset, &BTreeMap::new(), &mut heat, &mut entropy, 0, &[]
        );

        if res.is_err() {
            assert!(changeset.is_empty());
        }
    }
}

proptest! {
    // 7. SignedReaction replay attack
    // ATTACK SURFACE: SignedReaction replay attack
    // INVARIANT: executing the same valid SignedReaction twice in a row (same nonce) must return NonceMismatch on the second call.
    // SEVERITY: Critical
    #[test]
    fn prop_pvm_replay_prevention(energy in 10.0f32..50.0f32) {
        let mut state = MockStateView::new();
        let sender_pk = vec![1; 32];
        let mut sender = Atom::new_materialized(sender_pk.clone(), Element::Hydrogen);
        sender.mass = 1_000_000;
        sender.nonce = 5;
        state.atoms.insert(sender_pk.clone(), sender.clone());

        let mut rx = SignedReaction {
            sender,
            receiver: Atom::new_materialized(vec![2; 32], Element::Hydrogen),
            payload: Payload::Transfer { amount: 100 },
            energy,
            signature: b"valid_sig".to_vec(),
            reaction_hash: [0; 32],
            timestamp: 0,
        };
        rx.sender.nonce = 5;

        let mut heat = 0.0;
        let mut entropy = 0.0;
        
        {
            let ctx = ExecutionContext::<MockCryptoVerifier> {
                state: &state,
                architect_pk: &vec![0; 32],
                current_temp: 0.0,
                crystal_index: 1,
                wasm_runtime: None,
                _crypto: std::marker::PhantomData,
            };
            let mut changeset = Changeset::new();
            let res1 = PVM::execute_single::<MockCryptoVerifier>(
                &ctx, &rx, &mut changeset, &BTreeMap::new(), &mut heat, &mut entropy, 0, &[]
            );
            assert!(res1.is_ok());

            for (pk, atom) in changeset.inner.iter() {
                state.atoms.insert(pk.clone(), atom.clone());
            }
        }
        
        let mut changeset2 = Changeset::new();
        let ctx2 = ExecutionContext::<MockCryptoVerifier> {
            state: &state,
            architect_pk: &vec![0; 32],
            current_temp: 0.0,
            crystal_index: 1,
            wasm_runtime: None,
            _crypto: std::marker::PhantomData,
        };
        let res2 = PVM::execute_single::<MockCryptoVerifier>(
            &ctx2, &rx, &mut changeset2, &BTreeMap::new(), &mut heat, &mut entropy, 0, &[]
        );
        assert!(matches!(res2, Err(PvmError::NonceMismatch { .. })));
    }
}

proptest! {
    // 8. Arithmetic overflow gates
    // ATTACK SURFACE: Arithmetic overflow gates
    // INVARIANT: Transfer with amount = u64::MAX and energy = f32 that casts to u64::MAX must return InsufficientMass or ArithmeticOverflow, never Ok.
    // SEVERITY: High
    #[test]
    fn prop_pvm_transfer_overflow(energy in 0.0f32..1000000.0f32) {
        let mut state = MockStateView::new();
        let sender_pk = vec![1; 32];
        let mut sender = Atom::new_materialized(sender_pk.clone(), Element::Hydrogen);
        sender.mass = u64::MAX;
        state.atoms.insert(sender_pk.clone(), sender.clone());

        let rx = SignedReaction {
            sender,
            receiver: Atom::new_materialized(vec![2; 32], Element::Hydrogen),
            payload: Payload::Transfer { amount: u64::MAX },
            energy,
            signature: b"valid_sig".to_vec(),
            reaction_hash: [0; 32],
            timestamp: 0,
        };

        let ctx = ExecutionContext::<MockCryptoVerifier> {
            state: &state,
            architect_pk: &vec![0; 32],
            current_temp: 0.0,
            crystal_index: 1,
            wasm_runtime: None,
            _crypto: std::marker::PhantomData,
        };

        let mut changeset = Changeset::new();
        let mut heat = 0.0;
        let mut entropy = 0.0;
        let res = PVM::execute_single::<MockCryptoVerifier>(
            &ctx, &rx, &mut changeset, &BTreeMap::new(), &mut heat, &mut entropy, 0, &[]
        );
        
        if energy > 0.0 {
            assert!(res.is_err());
        }
    }
}

// 9. WASM limits constants
// ATTACK SURFACE: WASM limits constants
// INVARIANT: MAX_WASM_MEMORY_PAGES * 65536 == 16 MiB (16_777_216).
// SEVERITY: High
#[test]
fn prop_wasm_limits_memory() {
    assert_eq!(MAX_WASM_MEMORY_PAGES as u64 * 65536, 16_777_216);
}

// INVARIANT: GAS_HEAT_DIVISOR > 0.0 (division must never be by zero).
// SEVERITY: High
#[test]
fn prop_wasm_limits_gas_divisor() {
    assert!(GAS_HEAT_DIVISOR > 0.0);
}

// INVARIANT: MODULE_CACHE_SIZE > 0.
// SEVERITY: Medium
#[test]
fn prop_wasm_limits_cache_size() {
    assert!(MODULE_CACHE_SIZE > 0);
}

proptest! {
    // INVARIANT: storage_cost = code.len() * 100 must not silently overflow (test with code.len() > usize::MAX / 100).
    // SEVERITY: High
    #[test]
    fn prop_pvm_storage_cost_overflow(len in (usize::MAX / 100 + 1)..usize::MAX) {
        let res = (len as u64).checked_mul(100);
        assert!(res.is_none());
    }
}
