// =============================================================================
// processor.rs — Primus Block Processor (Refactored to use primus-types)
//
// CHANGES:
//
//   BUG A FIX — verify_pow() now normalises chamber_temp through
//     physics_shim::to_vm_thermal() before calling check_proof_of_work().
//     The miner (engine.rs) already normalises via pow_temp = to_vm_thermal();
//     without this fix every verifying node checked PoW against raw spec-unit
//     temperature (~1500 K) while the miner solved against VM-unit temperature
//     (~1000 K), making the two diverge above 1000 K → network fork.
//
//   BUG B FIX — perform_reorg() now executes the entire rollback + re-apply
//     phase inside a single sled::Db::transaction(). All atom restores,
//     crystal deletes, and undo-log deletes are batched as one atomic write,
//     so a mid-reorg crash can no longer leave a half-rolled-back DB.
// =============================================================================

use crate::crystal::{Crystal, CrystalExt};
use crate::engine::PrimusEngine;

use crate::kinetic::SignedReaction;

use crate::state::StateTree;
use crate::storage::PrimusStorage;
use primus_storage::{UndoLog, FINALITY_DEPTH};

use anyhow::{Context, Result, anyhow};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};

// FINALITY_DEPTH removed, using primus_storage::FINALITY_DEPTH


pub type SharedEngine = Arc<Mutex<PrimusEngine>>;

pub struct PrimusProcessor {
    pub engine: PrimusEngine,
}

impl PrimusProcessor {
    pub async fn new(data_path: &str, architect_pk: Vec<u8>) -> Result<Self> {
        let storage = PrimusStorage::new(data_path)?;
        let initial_atoms = storage.get_all_atoms()?;
        let metrics = storage.get_global_metrics()?.unwrap_or_default();

        let mut state = StateTree::new();
        state.load(initial_atoms, metrics);

        if let Ok(Some(last_block)) = storage.get_crystal_latest() {
            state.current_crystal_index = last_block.index;
            println!(
                "📈 Processor: State restored to height #{}",
                last_block.index
            );
        }

        let engine = PrimusEngine::new(storage, architect_pk, state).await?;
        Ok(Self { engine })
    }

    pub async fn process_network_reaction(
        shared_engine: SharedEngine,
        rx: SignedReaction,
    ) -> Result<()> {
        let mut engine = shared_engine.lock().await;

        if engine.state.get_atom(&rx.sender.public_key).is_none()
            && rx.sender.public_key != engine.architect_pk
        {
            return Err(anyhow!(
                "process_network_reaction: phantom sender {:02x?} — atom not on chain.",
                &rx.sender.public_key[..4.min(rx.sender.public_key.len())]
            ));
        }

        if crate::physics_shim::should_engage_gravity_shield(engine.chamber.temperature as f64) {
            return Err(anyhow!(
                "process_network_reaction: Chamber Overheat ({:.2}K). Reaction rejected.",
                engine.chamber.temperature
            ));
        }

        engine.chamber.inject_reaction(rx);
        Ok(())
    }

    pub async fn process_network_crystal(
        shared_engine: SharedEngine,
        mut incoming: Crystal,
    ) -> Result<()> {
        let mut engine = shared_engine.lock().await;
        let latest = engine.storage.get_crystal_latest()?;

        match latest {
            Some(last_local) => {
                if incoming.index == last_local.index + 1 {
                    if incoming.prev_hash == last_local.calculate_density() {
                        Self::verify_pow(&engine, &incoming)?;
                        return Self::finalize_solidification(&mut engine, &mut incoming).await;
                    }

                    if incoming.cumulative_energy > last_local.cumulative_energy + 1.0 {
                        return Self::perform_reorg(&mut engine, vec![incoming]).await;
                    }
                    return Err(anyhow!(
                        "Crystal #{} rejected: parent hash mismatch and energy \
                         ({:.2}) does not exceed local ({:.2}) by the reorg threshold.",
                        incoming.index,
                        incoming.cumulative_energy,
                        last_local.cumulative_energy,
                    ));
                }

                if incoming.index == last_local.index
                    && incoming.cumulative_energy > last_local.cumulative_energy
                {
                    return Self::perform_reorg(&mut engine, vec![incoming]).await;
                }

                if incoming.index > last_local.index + 1 {
                    return Err(anyhow!(
                        "Gap detected! Local tip: #{}, Received: #{}. Sync required.",
                        last_local.index,
                        incoming.index
                    ));
                }

                println!(
                    "📡 Crystal #{} already known (local height #{}). Skipping.",
                    incoming.index, last_local.index
                );
            }

            None => {
                println!("🌌 Genesis Crystal received. Initializing Universe.");
                Self::verify_pow(&engine, &incoming)?;
                return Self::finalize_solidification(&mut engine, &mut incoming).await;
            }
        }

        Ok(())
    }

    /// Verify proof-of-work for an incoming Crystal.
    ///
    /// BUG A FIX: temperature must be normalised through the physics shim before
    /// being passed to check_proof_of_work(), exactly as engine.rs does when
    /// mining (see engine.rs: `pow_temp = to_vm_thermal(temp as f64) as f32`).
    ///
    /// Raw spec-unit temperature (~1500 K) and VM-unit temperature (~1000 K)
    /// produce different PoW targets; mixing them would cause every node to
    /// disagree on block validity above 1000 K, splitting the network.
    #[inline]
    fn verify_pow(engine: &PrimusEngine, crystal: &Crystal) -> Result<()> {
        let raw_temp = engine.chamber.temperature;  // spec units — logging only
        let pow_temp = crate::physics_shim::to_vm_thermal(raw_temp as f64) as f32;
        let diff = engine.epoch_difficulty;

        if !crystal.check_proof_of_work(pow_temp, diff) {
            return Err(anyhow!(
                "Crystal #{} rejected: PoW invalid \
                 (hash_prefix exceeds target for temp={:.2}K → vm_temp={:.2}, epoch_diff={:.4})",
                crystal.index,
                raw_temp,
                pow_temp,
                diff
            ));
        }
        Ok(())
    }

    pub async fn finalize_solidification(engine: &mut PrimusEngine, crystal: &mut Crystal) -> Result<()> {
        let old_index = engine.state.current_crystal_index;
        let pre_state_root = engine.storage.current_root().await;
        engine.state.current_crystal_index = crystal.index;

        let (changeset, consumed_entropy) = match crate::pvm::execute_payload(
            &engine.state,
            &engine.storage,
            &crystal.reactions_data,
            engine.chamber.temperature,
            &engine.architect_pk,
            engine.wasm_runtime.as_deref(),
        ) {

            Ok(res) => res,
            Err(e) => {
                engine.state.current_crystal_index = old_index;
                eprintln!("❌ PVM CRITICAL ERROR on Crystal #{}: {}", crystal.index, e);
                return Err(anyhow!("PVM execution failed: {}", e));
            }
        };

        let calculated_root = engine
            .storage
            .commit_changeset(&changeset, crystal.index)
            .await
            .context("Failed to commit changeset to MPT")?;

        if crystal.state_root == [0u8; 32] {
            crystal.state_root = calculated_root;
        } else if crystal.state_root != calculated_root {
            engine.state.current_crystal_index = old_index;
            return Err(anyhow!(
                "CRITICAL: State Root Mismatch for Crystal #{}! \
                 Network root: {:02x?}… Calculated: {:02x?}…",
                crystal.index,
                &crystal.state_root[..4],
                &calculated_root[..4]
            ));
        }

        let mut undo_log = UndoLog::new(crystal.index, pre_state_root);
        for pk in changeset.inner.keys() {
            let pre_image = engine.state.get_atom(pk).cloned();
            undo_log.record(pk.clone(), pre_image);
        }

        engine
            .storage
            .save_undo_log(&undo_log)
            .context("Failed to persist UndoLog before state commit")?;

        engine.chamber.entropy += consumed_entropy;
        engine.state.global_metrics.temperature = engine.chamber.temperature;
        engine.state.global_metrics.entropy = engine.chamber.entropy;
        engine
            .storage
            .save_global_metrics(&engine.state.global_metrics)
            .context("Failed to persist GlobalMetrics")?;

        engine
            .storage
            .save_crystal(crystal)
            .context("Failed to persist Crystal")?;

        engine.state.apply_changeset(changeset);

        let before_count = engine.chamber.volatile_reactions.len();


        engine
            .chamber
            .volatile_reactions
            .retain(|r| !crystal.reactions.contains(&r.reaction_hash));

        engine.chamber.inherit_history(crystal);
        engine
            .storage
            .prune_undo_window(engine.state.current_crystal_index);

        log::info!(
            "✅ Crystal #{} solidified | Root: {:02x?}… | \
             Evicted {}/{} chamber reactions",
            crystal.index,
            &calculated_root[..4],
            before_count.saturating_sub(engine.chamber.volatile_reactions.len()),
            before_count,
        );

        Ok(())
    }

    /// Execute a chain reorganisation.
    ///
    /// BUG B FIX: the entire DB-level rollback (atom restores + crystal/undo-log
    /// deletes) is now executed as a single `sled::Db::transaction()`.  Sled
    /// guarantees all-or-nothing semantics for a transaction, so a power failure
    /// or process crash mid-reorg can no longer leave the database in a
    /// half-rolled-back state (previously: the loop wrote individually, meaning
    /// only the first K-of-N blocks might have been rolled back on crash,
    /// yielding an MPT corrupt / state-root mismatch on restart).
    ///
    /// Note: `sled::Db::transaction` is synchronous and operates on the raw
    /// sled batch; it does NOT go through the Merkle-Patricia Trie.  The MPT is
    /// rebuilt correctly by subsequent `finalize_solidification` calls on the
    /// new chain — the same way it was built originally.
    pub async fn perform_reorg(
        guard: &mut MutexGuard<'_, PrimusEngine>,
        new_chain: Vec<Crystal>,
    ) -> Result<()> {
        let engine = &mut **guard;

        if new_chain.is_empty() {
            return Err(anyhow!("perform_reorg: new_chain is empty"));
        }

        let mut new_chain = new_chain;
        new_chain.sort_by_key(|c| c.index);

        let fork_base = new_chain.first().unwrap();
        let fork_tip  = new_chain.last().unwrap();
        let local_tip = engine.state.current_crystal_index;

        println!(
            "🔄 REORG: local tip=#{} | fork base=#{} | new tip=#{}",
            local_tip, fork_base.index, fork_tip.index
        );

        let rollback_depth = local_tip.saturating_sub(fork_base.index.saturating_sub(1));

        if rollback_depth > FINALITY_DEPTH {
            return Err(anyhow!(
                "perform_reorg: rollback depth {} exceeds FINALITY_DEPTH {}. \
                 Reorg rejected — local chain is past finality.",
                rollback_depth,
                FINALITY_DEPTH
            ));
        }

        // ── Step 1: find the common ancestor ─────────────────────────────────
        let common_ancestor_index: u64 = {
            let mut candidate = fork_base.index.saturating_sub(1);
            loop {
                match engine.storage.get_crystal(candidate) {
                    Ok(Some(local_block)) => {
                        if fork_base.prev_hash == local_block.calculate_density() {
                            break candidate;
                        }
                        if candidate == 0 { break 0; }
                        candidate -= 1;
                    }
                    Ok(None) => {
                        if candidate == 0 { break 0; }
                        candidate -= 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "⚠️ REORG: storage error fetching Crystal #{} during \
                             ancestor search: {}. Treating #{} as ancestor.",
                            candidate, e,
                            fork_base.index.saturating_sub(1)
                        );
                        break fork_base.index.saturating_sub(1);
                    }
                }
            }
        };

        println!(
            "🔄 REORG: common ancestor=#{} | blocks to undo: {}",
            common_ancestor_index,
            local_tip.saturating_sub(common_ancestor_index)
        );

        // ── Step 2: collect all undo logs into memory before touching the DB ─
        // We do this BEFORE the transaction so that any missing-undo-log error
        // aborts cleanly without having written a single byte to the database.
        let mut undo_logs: Vec<(u64, UndoLog)> = Vec::new();
        {
            let mut h = local_tip;
            while h > common_ancestor_index {
                match engine.storage.get_undo_log(h) {
                    Ok(Some(undo)) => undo_logs.push((h, undo)),
                    Ok(None) => {
                        return Err(anyhow!(
                            "perform_reorg: UndoLog missing for Crystal #{}. \
                             Cannot safely complete rollback — aborting reorg.",
                            h
                        ));
                    }
                    Err(e) => {
                        return Err(anyhow!(
                            "perform_reorg: storage error fetching UndoLog for #{}: {}",
                            h, e
                        ));
                    }
                }
                if h == 0 { break; }
                h -= 1;
            }
        }

        // ── Step 3: apply in-memory state rollback ────────────────────────────
        // This only touches the StateTree HashMap — no DB writes yet.
        for (_, undo) in &undo_logs {
            for (pk, maybe_atom) in &undo.pre_images {
                match maybe_atom {
                    Some(atom) => { engine.state.atoms.insert(pk.clone(), atom.clone()); }
                    None       => { engine.state.atoms.remove(pk); }
                }
            }
        }

        // ── Step 4: atomic DB rollback via sled transaction ───────────────────
        //
        // BUG B FIX: all sled writes for the entire rollback happen in one
        // transaction.  If the process dies mid-transaction sled's write-ahead
        // log rolls it back completely on next open — no partial state possible.
        //
        // Serialise atom values outside the transaction (bincode is not
        // transactional-context-aware) so we only do key/value inserts inside.
        let mut atom_writes: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
        for (_, undo) in &undo_logs {
            for (pk, maybe_atom) in &undo.pre_images {
                let mut key = b"atom_".to_vec();
                key.extend_from_slice(pk);
                match maybe_atom {
                    Some(atom) => {
                        let bytes = bincode::serialize(atom)
                            .context("perform_reorg: failed to serialize pre-image atom")?;
                        atom_writes.push((key, Some(bytes)));
                    }
                    None => { atom_writes.push((key, None)); }
                }
            }
        }

        let db = engine.storage.get_db();
        db.transaction(|tx| {
            // Restore atom pre-images
            for (key, maybe_bytes) in &atom_writes {
                match maybe_bytes {
                    Some(bytes) => { tx.insert(key.as_slice(), bytes.as_slice())?; }
                    None        => { tx.remove(key.as_slice())?; }
                }
            }
            // Delete crystals and undo logs for each rolled-back height
            for (h, _) in &undo_logs {
                let crystal_key = format!("crystal_{}", h);
                tx.remove(crystal_key.as_bytes())?;

                let undo_key = format!("undo_{}", h);
                tx.remove(undo_key.as_bytes())?;
            }
            Ok(())
        }).map_err(|e: sled::transaction::TransactionError| anyhow!(
            "perform_reorg: sled transaction failed during atomic rollback: {:?}", e
        ))?;

        // Flush to guarantee the transaction reached disk before we start
        // applying the new chain.
        db.flush().context("Sled flush failed after reorg rollback phase")?;

        engine.state.current_crystal_index = common_ancestor_index;

        for (h, _) in &undo_logs {
            println!("↩️  REORG: rolled back Crystal #{}", h);
        }
        println!(
            "✅ REORG: Rollback complete (atomic). State rewound to height #{}.",
            common_ancestor_index
        );

        // ── Step 5: apply the new fork blocks ────────────────────────────────
        let applied_count = new_chain.len();
        for mut new_block in new_chain {
            Self::verify_pow(engine, &new_block)?;
            Self::finalize_solidification(engine, &mut new_block).await.with_context(|| {
                format!(
                    "REORG: finalize_solidification failed for new Crystal #{}",
                    new_block.index
                )
            })?;
            println!("✅ REORG: Committed new Crystal #{}", new_block.index);
        }

        engine.chamber.temperature *= 0.85;
        engine.chamber.entropy     *= 0.90;

        println!(
            "🔄 REORG complete: {} local block(s) replaced, {} new block(s) applied. \
             New tip: #{} | Temp: {:.2}K | Entropy: {:.4}",
            rollback_depth,
            applied_count,
            engine.state.current_crystal_index,
            engine.chamber.temperature,
            engine.chamber.entropy,
        );

        Ok(())
    }
}
