// =============================================================================
// engine.rs — Primus Physics Engine (Mainnet-Ready)
//
// CHANGES vs previous revision (State Root Mismatch fixes):
//
//   BUG 1 FIX — synthesize_with_gravity() now receives (prev_hash, crystal_index)
//     so GravityEngine::generate_roll() is fully deterministic. The old call
//     passed only the gravity engine; live CPU/RAM entropy is completely removed
//     from the synthesis path. See chamber.rs and gravity.rs for the full fix.
//
//   BUG 4 FIX — Crystal timestamp is now derived from the parent block's
//     timestamp + target_block_time instead of SystemTime::now(). This ensures
//     both the miner and every verifying node compute the same timestamp, making
//     calculate_density() (which hashes the full Crystal struct) identical.
//     The wall clock is used only as a fallback for block #1 (genesis child).
//
//   BUG 5 FIX — apply_mining_reward() has been REMOVED from mine_block().
//     The mining reward is now baked directly into PVM::execute_payload() via
//     a synthetic MiningReward ReactionResult injected at the start of
//     confirmed_rxs. This guarantees the reward is included in the state root
//     that both the miner and all verifying nodes compute. The old approach
//     called apply_mining_reward() AFTER finalize_solidification() assigned
//     crystal.state_root, meaning the miner's persisted root never matched
//     the root any other node would calculate.
//
// CONCURRENCY NOTE (unchanged from previous revision):
//   The engine is wrapped in Arc<Mutex<>>. The RwLock migration path described
//   in the previous revision header is still the recommended next step.
// =============================================================================

use crate::atom::Atom;
use crate::chamber::ReactionChamber;
use crate::crystal::{Crystal, CrystalExt};
use crate::gravity::GravityEngine;
use crate::physics_shim::to_vm_thermal;

use crate::kinetic::{Payload, SignedReaction};
use crate::storage::PrimusStorage;
use anyhow::{Context, Result};
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::Arc;

pub const DIFFICULTY_EPOCH: u64 = 100;
pub const DIFFICULTY_CLAMP: (f32, f32) = (0.25, 64.0);
const MAX_NONCE_ATTEMPTS: u64 = 100_000;

pub struct GenesisConfig {
    pub initial_supply: u64,
    pub timestamp: u64,
    pub server_seed_env: String,
}

impl Default for GenesisConfig {
    fn default() -> Self {
        let mut config = Self {
            initial_supply: 1_000_000_000,
            timestamp: 1735689600,
            server_seed_env: "PRIMUS_SERVER_SEED".to_string(),
        };
        let content = std::fs::read_to_string("genesis.toml")
            .unwrap_or_else(|_| include_str!("../genesis.toml").to_string());
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("initial_supply = ")
                && let Ok(v) = val.parse() { config.initial_supply = v; }
            if let Some(val) = line.strip_prefix("timestamp = ")
                && let Ok(v) = val.parse() { config.timestamp = v; }
            if let Some(val) = line.strip_prefix("server_seed_env = ") {
                config.server_seed_env = val.trim_matches('"').to_string();
            }
        }
        config
    }
}

pub struct PrimusEngine {
    pub storage: PrimusStorage,
    pub state: crate::state::StateTree,
    pub chamber: ReactionChamber,
    pub target_block_time: u64,
    pub architect_pk: Vec<u8>,
    pub is_syncing: bool,
    pub sync_target: u64,
    pub epoch_difficulty: f32,
    pub wasm_runtime: Option<Arc<dyn primus_vm::WasmRuntime>>,
}

impl PrimusEngine {
    pub async fn new(
        storage: PrimusStorage,
        architect_pk: Vec<u8>,
        state: crate::state::StateTree,
    ) -> Result<Self> {
        let mut chamber = ReactionChamber::new();
        chamber.temperature = state.global_metrics.temperature;
        chamber.entropy = state.global_metrics.entropy;

        let wasm_runtime: Option<Arc<dyn primus_vm::WasmRuntime>> = {
            #[cfg(feature = "wasmtime-backend")]
            {
                use primus_vm::wasm::wasmtime_backend::WasmtimeRuntime;
                match WasmtimeRuntime::new() {
                    Ok(r) => Some(Arc::new(r)),
                    Err(e) => {
                        eprintln!("⚠️ WASM: Failed to initialize Wasmtime runtime: {}", e);
                        None
                    }
                }
            }
            #[cfg(not(feature = "wasmtime-backend"))]
            None
        };

        let mut engine = Self {
            storage,
            state,
            chamber,
            target_block_time: 10,
            architect_pk,
            is_syncing: false,
            sync_target: 0,
            epoch_difficulty: 1.0,
            wasm_runtime,
        };

        let config = GenesisConfig::default();
        let server_seed = std::env::var(&config.server_seed_env)
            .unwrap_or_else(|_| "primus_alpha_seed_2026".to_string());
        if server_seed == "primus_alpha_seed_2026" {
            eprintln!("⚠️  WARNING: Using default PRIMUS_SERVER_SEED. Set a unique value via environment variable for mainnet.");
        }

        engine.try_auto_genesis().await?;
        Ok(engine)
    }

    // =========================================================================
    // GENESIS PROTOCOL
    // =========================================================================
    pub async fn try_auto_genesis(&mut self) -> Result<()> {
        if !self.state.atoms.is_empty() {
            return Ok(());
        }
        println!("🌱 Empty state detected. Executing Genesis Protocol...");

        let config = GenesisConfig::default();
        let mut genesis_atom =
            Atom::new_materialized(self.architect_pk.clone(), crate::atom::Element::Hydrogen);
        genesis_atom.mass = config.initial_supply;

        let mut changeset = crate::state::Changeset::new();
        changeset.insert(self.architect_pk.clone(), genesis_atom);

        self.state.apply_changeset(changeset.clone());
        self.storage
            .commit_changeset(&changeset, self.state.current_crystal_index)
            .await
            .context("Genesis: failed to persist initial state")?;

        // ── NEW: Create and save Genesis Crystal (Block 0) ──
        let state_root = self.storage.current_root().await;
        
        use crate::crystal::CrystalExt;
        let genesis_crystal = crate::crystal::Crystal {
            index: 0,
            prev_hash: [0u8; 32],
            state_root,
            reactions: vec![],
            reactions_data: vec![],
            timestamp: config.timestamp, // deterministic genesis time
            nonce: 0,
            cumulative_energy: 0.0,
            final_temp: 1000.0,
            final_entropy: 0.5,
        };
        
        let _density = genesis_crystal.calculate_density();
        // Since it's genesis, we don't strictly enforce a PoW check, we just save it.
        self.storage.save_crystal(&genesis_crystal).context("Genesis: failed to save crystal")?;

        println!("✅ Genesis successful! Crystal #0 created, Architect materialized with 1,000,000,000 mass.");
        Ok(())
    }

    // =========================================================================
    // BLOCK PREPARATION
    // =========================================================================
    pub fn prepare_next_physical_state(
        &mut self,
    ) -> Result<(u64, [u8; 32], f32, u64, GravityEngine)> {
        let last_block = self
            .storage
            .get_crystal_latest()
            .context("prepare_next_physical_state: storage error")?;

        let mut last_index = 0u64;
        let mut prev_hash = [0u8; 32];
        let mut inherited_energy = 0.0f32;
        let mut parent_timestamp = 0u64;

        if let Some(ref block) = last_block {
            last_index = block.index;
            prev_hash = block.calculate_density();
            inherited_energy = block.cumulative_energy;
            parent_timestamp = block.timestamp;
            self.chamber.inherit_history(block);
        } else {
            self.chamber.update_expansion(1.10);
        }

        // BUG 4 FIX: derive the new block's timestamp from the parent rather
        // than reading the wall clock. This makes Crystal::calculate_density()
        // identical on the miner and on every verifying node.
        //
        // Fallback to wall clock ONLY for the very first child of genesis
        // (parent_timestamp == 0), since there is no prior block to anchor to.
        let deterministic_timestamp = if parent_timestamp > 0 {
            parent_timestamp + self.target_block_time
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("System clock error")?
                .as_secs()
        };

        // Difficulty adjustment (unchanged logic, preserved for reference)
        let time_diff = if parent_timestamp > 0 {
            // Use the previous block's real timestamp vs its parent for the
            // actual elapsed time measurement; the derived timestamp above is
            // just the canonical value for this block's header.
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(parent_timestamp)
        } else {
            self.target_block_time
        };

        let mut difficulty = if time_diff < self.target_block_time {
            println!("🔥 Fast network ({}s). Increasing difficulty.", time_diff);
            1.2f32
        } else {
            0.95
        };
        difficulty += self.chamber.entropy * 0.005;
        if self.chamber.temperature > 100.0 {
            difficulty *= 1.5;
            println!(
                "🌡️ Chamber Temp {:.2}K — difficulty elevated.",
                self.chamber.temperature
            );
        }

        let config = GenesisConfig::default();
        let server_seed = std::env::var(&config.server_seed_env)
            .unwrap_or_else(|_| "primus_alpha_seed_2026".to_string());
        let gravity = GravityEngine::new(&server_seed, 0.92 * difficulty);
        Ok((
            last_index,
            prev_hash,
            inherited_energy,
            deterministic_timestamp,
            gravity,
        ))
    }

    // =========================================================================
    // EPOCH DIFFICULTY
    // =========================================================================
    pub fn calculate_epoch_difficulty(&self) -> f32 {
        let current_height = self.state.current_crystal_index;
        if current_height < DIFFICULTY_EPOCH {
            return self.epoch_difficulty;
        }

        let epoch_start = current_height.saturating_sub(DIFFICULTY_EPOCH);

        let start_ts = self
            .storage
            .get_crystal(epoch_start)
            .ok()
            .flatten()
            .map(|c| c.timestamp)
            .unwrap_or(0);
        let end_ts = self
            .storage
            .get_crystal_latest()
            .ok()
            .flatten()
            .map(|c| c.timestamp)
            .unwrap_or(0);

        if start_ts == 0 || end_ts <= start_ts {
            return self.epoch_difficulty;
        }

        let elapsed = (end_ts - start_ts) as f32;
        let ideal = (DIFFICULTY_EPOCH as f32) * (self.target_block_time as f32);
        let ratio = ideal / elapsed;
        let new_diff = self.epoch_difficulty * ratio;
        let clamped = new_diff.clamp(DIFFICULTY_CLAMP.0, DIFFICULTY_CLAMP.1);

        println!(
            "📐 Epoch difficulty recalculated: {:.4} → {:.4} (elapsed={:.0}s ideal={:.0}s)",
            self.epoch_difficulty, clamped, elapsed, ideal
        );
        clamped
    }

    // =========================================================================
    // MINING REWARD — internal helper only
    // =========================================================================

    /// Build the canonical MiningReward reaction that grants 10 mass to the
    /// Architect. This is injected as the FIRST element of the confirmed
    /// transaction list before PVM::execute_payload runs, so the reward is
    /// baked into the state root that the crystal carries.
    ///
    /// BUG 5 FIX: the old apply_mining_reward() called save_state_changes()
    /// AFTER finalize_solidification() had already assigned crystal.state_root,
    /// meaning the miner's on-disk root never matched the root any verifying
    /// node would compute. By injecting the reward here, it participates in
    /// the PVM execution that produces calculated_root, which is then assigned
    /// to crystal.state_root in finalize_solidification().
    fn build_mining_reward_rx(&self, crystal_index: u64) -> Option<SignedReaction> {
        let architect = self.state.get_atom(&self.architect_pk)?.clone();

        // A synthetic reaction: architect sends 0 mass to itself, just to
        // trigger the reward credit in the PVM's Transfer branch.
        // Use the canonical mining_reward_hash from primus-types.
        let reaction_hash = SignedReaction::mining_reward_hash(crystal_index, &self.architect_pk);

        Some(SignedReaction {
            sender: architect.clone(),
            receiver: architect,
            reaction_hash,
            energy: 0.0,
            timestamp: 0,      // canonical zero — not hashed directly
            signature: vec![], // signed by Architect implicitly; PVM accepts architect_pk
            payload: Payload::MiningReward { amount: 10 },
        })
    }

    // =========================================================================
    // MINE BLOCK
    // =========================================================================
    pub async fn mine_block(&mut self, resonant_rxs: Vec<SignedReaction>) -> Result<Option<Crystal>> {
        let rx_count = resonant_rxs.len();
        for rx in resonant_rxs {
            self.chamber.inject_reaction(rx);
        }

        // BUG 4 FIX: deterministic_timestamp is now derived from the parent
        // block, not from SystemTime::now() at Crystal::new() time.
        let (idx, prev_hash, energy, deterministic_timestamp, gravity) =
            self.prepare_next_physical_state()?;

        // BUG 1 FIX: pass (prev_hash, crystal_index) to synthesize_with_gravity
        // so that generate_roll() is deterministic on all nodes.
        let candidate_rxs = self.chamber.synthesize_with_gravity(
            &gravity,
            &prev_hash,
            idx + 1, // the index this block will carry
        );

        // PVM pre-filter — invalid transactions are logged and dropped.
        let mut confirmed: Vec<_> = candidate_rxs
            .into_iter()
            .filter(|rx| {
                match crate::pvm::execute_payload(
                    &self.state,
                    &self.storage,
                    std::slice::from_ref(rx),
                    self.chamber.temperature,
                    &self.architect_pk,
                    self.wasm_runtime.as_deref(),
                ) {

                    Ok(_) => true,
                    Err(e) => {
                        println!(
                            "⚠️ Mining: Tx {:02x?} rejected by PVM: {}",
                            &rx.reaction_hash[..4],
                            e
                        );
                        false
                    }
                }
            })
            .collect();

        // BUG 5 FIX: prepend the mining reward so it is included in the PVM
        // execution inside finalize_solidification() and therefore participates
        // in state_root calculation. apply_mining_reward() is NOT called after
        // finalize_solidification() — that was the old double-credit bug.
        if let Some(reward_rx) = self.build_mining_reward_rx(idx + 1) {
            confirmed.insert(0, reward_rx);
        }

        // BUG 4 FIX: set crystal timestamp deterministically before any hashing.
        let mut crystal = Crystal::new(idx + 1, prev_hash);
        crystal.timestamp = deterministic_timestamp;
        crystal.reactions_data = confirmed;
        crystal.cumulative_energy = energy;
        crystal.reactions = crystal
            .reactions_data
            .iter()
            .map(|r| r.reaction_hash)
            .collect();
        crystal.optimize_lattice();

        // Epoch difficulty update
        if crystal.index.is_multiple_of(DIFFICULTY_EPOCH) && crystal.index > 0 {
            self.epoch_difficulty = self.calculate_epoch_difficulty();
            println!("📐 New epoch_difficulty: {:.4}", self.epoch_difficulty);
        }

        // Proof of Work mining loop
        // NOTE: raw spec-unit temperature is kept for logging below (correct K values).
        // For PoW difficulty calculation, temperature is normalized via to_vm_thermal()
        // so that both miner and verifier compute identical targets regardless of spec units.
        let temp = self.chamber.temperature;          // raw spec value — logging only
        let pow_temp = to_vm_thermal(temp as f64) as f32; // normalized — PoW only
        let diff = self.epoch_difficulty;

        let solved = (0..MAX_NONCE_ATTEMPTS).any(|_| {
            if crystal.check_proof_of_work(pow_temp, diff) {
                true
            } else {
                crystal.nonce = crystal.nonce.wrapping_add(1);
                false
            }
        });

        if !solved {
            log::debug!(
                "⛏️ Crystal #{}: PoW unsolved after {} nonces, yielding.",
                crystal.index,
                MAX_NONCE_ATTEMPTS
            );
            return Ok(None);
        }

        println!(
            "⛏️ Crystal #{} solved! nonce={} temp={:.2}K diff={:.4}",
            crystal.index, crystal.nonce, temp, diff
        );

        crate::processor::PrimusProcessor::finalize_solidification(self, &mut crystal)
            .await
            .with_context(|| format!("Solidification failed for Crystal #{}", crystal.index))?;

        // BUG 5 FIX: apply_mining_reward() is intentionally NOT called here.
        // The reward is already baked into the confirmed transaction list above
        // and has been committed by finalize_solidification().

        self.storage
            .get_db()
            .flush()
            .context("Sled flush failed after mine_block")?;

        if rx_count > 0 || !crystal.reactions_data.is_empty() {
            println!(
                "✨ Crystal #{} solidified with {} reactions.",
                crystal.index,
                crystal.reactions_data.len()
            );
        }
        Ok(Some(crystal))
    }
}
