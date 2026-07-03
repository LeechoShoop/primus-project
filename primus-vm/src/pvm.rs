// =============================================================================
// primus-vm/src/pvm.rs — Primus Virtual Machine (Migrated from primus-core)
//
// SEQUENCE CHECK (v2 — Nonce-based, replaces last_reaction_hash snapshot):
//
//   Step 1 — ML-DSA-87 signature:
//     C::verify(sender_pk, &rx.signing_digest(), &rx.signature)
//
//   Step 4 — Anti-replay:
//     on_chain.nonce == rx.sender.nonce
//     After Transfer confirms: on_chain.nonce += 1
//
//   MiningReward: does NOT touch sender.nonce.  Reward blocks never
//   invalidate pending user transactions.
//
//   Why nonce instead of last_reaction_hash:
//     last_reaction_hash advances every block (MiningReward updates it).
//     A mempool tx built before a reward block would always fail the old check.
//     Nonce only advances on confirmed user Transfers — transactions stay valid
//     across any number of reward-only blocks.
//
// SECURITY INVARIANTS (unchanged):
//   1. Zero signature bypasses on user transactions.
//   2. Every Transfer / Generic MUST carry a valid ML-DSA-87 signature.
//   3. MiningReward is the sole signature-exempt path.
//   4. All arithmetic uses checked/saturating ops.
//   5. Thermal capacity limit (1000.0) still applies.
// =============================================================================

use primus_types::atom::{Atom, Element, QuantumState};
use primus_types::payload::Payload;
use primus_types::reaction::SignedReaction;
use primus_storage::Changeset;

use crate::context::{CryptoVerifier, ExecutionContext};
use crate::error::PvmError;
use crate::physics;
use sha3::Digest;

/// Marker struct for the Primus Virtual Machine's native execution path.
pub struct PVM;

/// Helper: get an atom by pk, checking changeset first then state.
/// Returns an owned clone since StateView returns owned Atoms.
fn lookup_atom(
    changeset: &Changeset,
    state: &dyn crate::context::StateView,
    pk: &[u8],
) -> Option<Atom> {
    changeset.get(pk).cloned().or_else(|| state.get_atom(pk))
}

impl PVM {
    /// Execute a single reaction against the current state.
    ///
    /// This is a helper for `PayloadDispatcher`.
    pub fn execute_single<C: CryptoVerifier>(
        ctx: &ExecutionContext<C>,
        rx: &SignedReaction,
        changeset: &mut Changeset,
        atoms_iter: &std::collections::BTreeMap<Vec<u8>, Atom>,
        total_crystal_heat: &mut f32,
        total_consumed_entropy: &mut f32,
        galactic_drift: u8,
        reactions: &[SignedReaction], // needed for quantum entanglement check
    ) -> Result<(), PvmError> {
        let thermal_capacity = physics::THERMAL_CAPACITY;

        // ── 0. Basic sanity ───────────────────────────────────────────────
        if rx.energy < 0.0 {
            return Err(PvmError::NegativeEnergy);
        }

        // ── MINING REWARD fast path ───────────────────────────────────────
        if let Payload::MiningReward { amount } = &rx.payload {
            if rx.receiver.public_key != ctx.architect_pk {
                return Err(PvmError::InvalidRewardRecipient);
            }

            let arch_opt = lookup_atom(changeset, ctx.state, ctx.architect_pk);

            match arch_opt {
                Some(arch) => {
                    let mut a = arch.clone();
                    a.mass = a.mass.saturating_add(*amount);
                    a.last_reaction_hash = rx.reaction_hash;
                    changeset.insert(ctx.architect_pk.to_vec(), a);
                }
                None => {
                    return Err(PvmError::AtomNotFound);
                }
            }
            return Ok(()); // no entropy tax, no heat
        }

        // ── 1. ML-DSA-87 SIGNATURE VERIFICATION ──────────────────────────
        {
            let digest = rx.signing_digest();
            let is_owner = C::verify(&rx.sender.public_key, &digest, &rx.signature);
            let is_architect = C::verify(ctx.architect_pk, &digest, &rx.signature);

            if !is_owner && !is_architect {
                return Err(PvmError::SignatureRejected {
                    reason: format!(
                        "sender {:02x?}. Neither owner nor Architect validated against signing_digest {:02x?}.",
                        &rx.sender.public_key[..4.min(rx.sender.public_key.len())],
                        &digest[..4]
                    ),
                });
            }
        }

        // ── 2. Physics ────────────────────────────────────────────────────
        let resonance =
            physics::calculate_orbital_resonance(&rx.sender.public_key, galactic_drift);
        let gravity_assist =
            physics::calculate_gravity_assist_from_iter(atoms_iter.iter(), &rx.sender.public_key);
        let local_curvature = (physics::get_spacetime_curvature(&rx.reaction_hash, ctx.current_temp)
            - gravity_assist
            - resonance)
            .max(0.0);

        *total_crystal_heat += local_curvature;
        if *total_crystal_heat > thermal_capacity {
            return Err(PvmError::ThermalLimitExceeded);
        }

        let local_macro_shift = physics::calculate_macro_shift(local_curvature);
        let mut operation_complexity = 100u64;

        // ── 3. Quantum Logic ──────────────────────────────────────────────
        if let Some(on_chain_sender) = ctx.state.get_atom(&rx.sender.public_key) {
            match on_chain_sender.quantum_state {
                QuantumState::Entangled(ref partner_id) => {
                    operation_complexity += 500;
                    let partner_active = reactions.iter().any(|r| {
                        r.sender.public_key == partner_id
                            || r.receiver.public_key == partner_id
                    });
                    if !partner_active {
                        return Err(PvmError::QuantumCollapse);
                    }
                }
                QuantumState::Superposition(_) => {
                    operation_complexity += 250;
                }
                QuantumState::Stable => {}
            }
        }

        let complexity_scaled = if local_macro_shift > 0.0 {
            (operation_complexity as f32 * (1.0 + local_macro_shift)) as u64
        } else {
            operation_complexity
        };

        if complexity_scaled > crate::wasm::limits::MAX_SAFE_COMPLEXITY {
            return Err(PvmError::ArithmeticOverflow);
        }
        let tax = physics::calculate_entropy_tax(complexity_scaled, local_curvature);
        *total_consumed_entropy += tax as f32 * 0.01;

        // ── 4. NONCE-BASED SEQUENCE CHECK (Anti-Replay) ───────────────────
        let on_chain_sender_opt = lookup_atom(changeset, ctx.state, &rx.sender.public_key);

        if let Some(ref on_chain) = on_chain_sender_opt
            && on_chain.nonce != rx.sender.nonce {
                return Err(PvmError::NonceMismatch {
                    on_chain: on_chain.nonce,
                    tx_nonce: rx.sender.nonce,
                });
            }

        // ── 5. Payload dispatch ───────────────────────────────────────────
        match &rx.payload {
            Payload::Transfer { amount } => {
                let transfer_amount = *amount;
                let fee = rx.energy as u64;
                let total_cost = transfer_amount
                    .checked_add(fee)
                    .ok_or(PvmError::ArithmeticOverflow)?;

                let mut sender = on_chain_sender_opt.clone().ok_or(PvmError::AtomNotFound)?;

                if sender.mass < total_cost {
                    return Err(PvmError::InsufficientMass {
                        has: sender.mass,
                        needs: total_cost,
                    });
                }

                sender.mass -= total_cost;
                sender.last_reaction_hash = rx.reaction_hash;
                sender.nonce = sender.nonce.saturating_add(1);

                apply_decay(&mut sender, ctx.crystal_index);
                evolve(&mut sender);

                changeset.insert(sender.public_key.clone(), sender);

                let mut receiver = lookup_atom(changeset, ctx.state, &rx.receiver.public_key)
                    .unwrap_or_else(|| {
                        let mut a = rx.receiver.clone();
                        a.mass = 0;
                        a
                    });

                apply_decay(&mut receiver, ctx.crystal_index);
                receiver.mass = receiver.mass.saturating_add(transfer_amount);
                evolve(&mut receiver);

                changeset.insert(receiver.public_key.clone(), receiver);
            }

            Payload::Generic => {
                if let Some(on_chain) = on_chain_sender_opt.as_ref() {
                    let total_cost = (rx.energy as u64)
                        .checked_add(tax)
                        .ok_or(PvmError::ArithmeticOverflow)?;
                    let mut sender = on_chain.clone();
                    sender.mass = sender.mass.checked_sub(total_cost).ok_or(
                        PvmError::InsufficientMass {
                            has: sender.mass,
                            needs: total_cost,
                        },
                    )?;
                    sender.last_reaction_hash = rx.reaction_hash;
                    sender.nonce = sender.nonce.saturating_add(1);
                    changeset.insert(sender.public_key.clone(), sender);
                } else if rx.sender.public_key == ctx.architect_pk {
                    let mut genesis = rx.sender.clone();
                    genesis.mass = 1_000_000_000;
                    genesis.last_reaction_hash = rx.reaction_hash;
                    changeset.insert(rx.sender.public_key.clone(), genesis);
                } else {
                    return Err(PvmError::AtomNotFound);
                }

                let mut receiver = lookup_atom(changeset, ctx.state, &rx.receiver.public_key)
                    .unwrap_or_else(|| rx.receiver.clone());
                receiver.mass = receiver.mass.saturating_add(rx.energy as u64);
                changeset.insert(receiver.public_key.clone(), receiver);

                if tax > 0
                    && let Some(arch) = lookup_atom(changeset, ctx.state, ctx.architect_pk) {
                        let mut a = arch.clone();
                        a.mass = a.mass.saturating_add(tax);
                        changeset.insert(ctx.architect_pk.to_vec(), a);
                    }
            }

            Payload::Contract { code } => {
                if code.len() > crate::wasm::limits::MAX_MODULE_SIZE_BYTES {
                    return Err(PvmError::WasmBackendError("Module too large".into()));
                }

                let mut sender = on_chain_sender_opt.clone().ok_or(PvmError::AtomNotFound)?;
                let storage_cost = (code.len() as u64) * 100; // 100 mass per byte

                if sender.mass < storage_cost {
                    return Err(PvmError::InsufficientMass {
                        has: sender.mass,
                        needs: storage_cost,
                    });
                }

                sender.mass -= storage_cost;
                sender.nonce = sender.nonce.saturating_add(1);
                sender.last_reaction_hash = rx.reaction_hash;
                
                let code_hash: [u8; 32] = {
                    let mut hasher = sha3::Sha3_256::new();
                    hasher.update(code);
                    hasher.finalize().into()
                };

                changeset.insert(sender.public_key.clone(), sender);
                changeset.insert_contract(code_hash, code.clone());
            }

            Payload::ContractCall { .. } => {
                return Err(PvmError::WasmBackendError("ContractCall must be handled by PayloadDispatcher".into()));
            }

            Payload::MiningReward { .. } => {
                unreachable!("PVM: MiningReward must have been handled at the start of execute_single.");
            }

            Payload::Unknown => {
                return Err(PvmError::UnknownPayload);
            }
        }

        Ok(())
    }

    /// Execute a batch of reactions against the current state.
    ///
    /// Deprecated: Use PayloadDispatcher::execute instead.
    pub fn execute_payload<C: CryptoVerifier>(
        ctx: &ExecutionContext<C>,
        payload: &[SignedReaction],
        atoms_iter: &std::collections::BTreeMap<Vec<u8>, Atom>,
    ) -> Result<(Changeset, f32), PvmError> {
        let galactic_drift = physics::get_galactic_drift(ctx.crystal_index);
        let mut total_crystal_heat = 0.0_f32;
        let mut total_consumed_entropy = 0.0_f32;
        let mut changeset = Changeset::new();

        for rx in payload {
            Self::execute_single::<C>(
                ctx,
                rx,
                &mut changeset,
                atoms_iter,
                &mut total_crystal_heat,
                &mut total_consumed_entropy,
                galactic_drift,
                payload,
            )?;
        }

        Ok((changeset, total_consumed_entropy))
    }
}

// =============================================================================
// Atom evolution helpers (extracted from primus-core::atom::AtomCoreExt)
//
// These are duplicated here so primus-vm does not depend on primus-core.
// The logic is identical to AtomCoreExt::apply_decay and AtomCoreExt::evolve.
// =============================================================================

/// Apply entropy decay based on how long the atom has been inactive.
fn apply_decay(atom: &mut Atom, current_index: u64) -> u64 {
    let age = current_index.saturating_sub(atom.last_active_index);
    let stability_threshold = 100_u64.saturating_sub(atom.neutron_count as u64 * 5);
    if age > stability_threshold {
        let decay_rate = 0.001 + (atom.neutron_count as f32 * 0.0005);
        let decay_amount = (atom.mass as f32 * decay_rate) as u64;
        if decay_amount > 0 {
            atom.mass = atom.mass.saturating_sub(decay_amount);
            return decay_amount;
        }
    }
    0
}

/// Evolve the atom's element based on mass thresholds.
fn evolve(atom: &mut Atom) {
    match atom.element {
        Element::Hydrogen if atom.mass >= 4000 => {
            atom.element = Element::Oxygen;
            atom.charge = 3.44;
        }
        Element::Oxygen if atom.mass >= 6000 => {
            atom.element = Element::Carbon;
            atom.charge = 2.55;
        }
        Element::Carbon if atom.mass >= 50000 => {
            atom.element = Element::Gold;
            atom.charge = 2.54;
        }
        _ => {}
    }
}
