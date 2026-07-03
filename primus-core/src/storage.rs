use primus_storage::{Changeset, StorageError, UNDO_WINDOW};
use primus_storage::MerklePatriciaTrie;

use primus_storage::mpt_store::SledMptStore;
use primus_storage::types::{GlobalMetrics, UndoLog};
use anyhow::{Context, Result};
use primus_types::atom::Atom;
use primus_types::{Crystal, MerkleProof};

use sha3::{Digest, Sha3_256};
use sled::Db;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tokio::sync::RwLock;

pub struct PrimusStorage {
    db: Db,
    mpt: Mutex<MerklePatriciaTrie<SledMptStore>>,
    root: RwLock<Option<[u8; 32]>>,
    gc_log: sled::Tree,
}

impl PrimusStorage {
    pub fn new(path: &str) -> Result<Self> {
        let db = sled::open(path)
            .with_context(|| format!("Failed to open Sled database at '{}'", path))?;

        let store = SledMptStore::new(&db)?;

        let root = if let Some(root_bytes) = db.get("mpt_root")? {
            let arr: [u8; 32] = root_bytes.as_ref().try_into().context("mpt_root is corrupt")?;
            Some(arr)
        } else {
            None
        };

        let trie = if let Some(r) = root {
            MerklePatriciaTrie::with_root(store, r)
        } else {
            MerklePatriciaTrie::new(store)
        };

        let gc_log = db.open_tree("mpt_gc_log")?;

        Ok(Self { db, mpt: Mutex::new(trie), root: RwLock::new(root), gc_log })
    }

    pub fn get_db(&self) -> &Db {
        &self.db
    }

    pub fn clone_db(&self) -> Db {
        self.db.clone()
    }

    /// Construct a PrimusStorage from an already-open sled::Db.
    ///
    /// Used by on_get_proof in primus-core bridge to safely call the async
    /// get_proof() without holding a tokio::MutexGuard across an .await point
    /// (which would make the future !Send). sled::Db is Arc-backed so the
    /// cloned handle shares the same on-disk database — proofs are live.
    pub fn open_readonly(db: Db) -> Result<Self> {
        let store = SledMptStore::new(&db)?;
        let root = if let Some(root_bytes) = db.get("mpt_root")? {
            let arr: [u8; 32] = root_bytes.as_ref().try_into().context("mpt_root is corrupt")?;
            Some(arr)
        } else {
            None
        };
        let trie = if let Some(r) = root {
            MerklePatriciaTrie::with_root(store, r)
        } else {
            MerklePatriciaTrie::new(store)
        };
        let gc_log = db.open_tree("mpt_gc_log")?;
        Ok(Self { db, mpt: Mutex::new(trie), root: RwLock::new(root), gc_log })
    }

    // ── Atom I/O ──────────────────────────────────────────────────────────────

    pub fn get_atom(&self, pk: &[u8]) -> Result<Option<Atom>> {
        let mut key = b"atom_".to_vec();
        key.extend_from_slice(pk);
        match self.db.get(&key)? {
            Some(b) => Ok(Some(bincode::deserialize(&b)?)),
            None => Ok(None),
        }
    }

    pub fn save_contract(&self, code_hash: [u8; 32], code: &[u8]) -> Result<()> {
        let mut key = b"contract_".to_vec();
        key.extend_from_slice(&code_hash);
        self.db.insert(key, code)?;
        Ok(())
    }

    pub fn load_contract(&self, code_hash: [u8; 32]) -> Result<Option<Vec<u8>>> {
        let mut key = b"contract_".to_vec();
        key.extend_from_slice(&code_hash);
        match self.db.get(key)? {
            Some(b) => Ok(Some(b.to_vec())),
            None => Ok(None),
        }
    }

    pub fn get_all_atoms(&self) -> Result<BTreeMap<Vec<u8>, Atom>> {
        let mut atoms = BTreeMap::new();
        for item in self.db.scan_prefix(b"atom_") {
            let (key, value) = item?;
            if key.len() <= 5 { continue; }
            let pk = key[5..].to_vec();
            if let Ok(atom) = bincode::deserialize::<Atom>(&value) {
                atoms.insert(pk, atom);
            }
        }
        Ok(atoms)
    }

    // ── Commit ────────────────────────────────────────────────────────────────

    pub async fn commit_changeset(&self, changeset: &Changeset, crystal_index: u64) -> Result<[u8; 32]> {
        // 1. Pre-process outside the lock to minimize critical section duration.
        // We prepare hashes, serializations, and clones here.
        let mut updates = Vec::with_capacity(changeset.inner.len());
        for (pk, atom) in &changeset.inner {
            let mut hasher = Sha3_256::new();
            hasher.update(pk);
            let trie_key: [u8; 32] = hasher.finalize().into();

            let value = bincode::serialize(atom)?;
            updates.push((trie_key, value, pk.clone(), atom.clone()));
        }

        // 2. Critical section: Update the Merkle-Patricia Trie.
        // The lock is only held for the duration of the trie structure modification.
        let new_root = {
            let mut mpt = self.mpt.lock().unwrap();

            // Record old root BEFORE mutating — this is what GC will clean up later
            let old_root = mpt.root();

            for (trie_key, value, _, _) in &updates {
                mpt.insert(trie_key, value.clone())?;
            }
            let new_root = mpt.root().unwrap_or([0u8; 32]);

            // Store (crystal_index → old_root) for GC
            if let Some(old) = old_root {
                self.gc_log.insert(
                    crystal_index.to_le_bytes(),
                    &old,
                )?;
            }

            new_root
        };

        // INTDIV-001 fix: replaced blocking_write() with .write().await
        // blocking_write() panics inside tokio async runtime.
        *self.root.write().await = Some(new_root);

        // 3. Sled Persistence: Raw atom storage, contract code, and root tracking.
        // Sled is thread-safe; these operations can proceed without the Trie lock.
        for (_, _, pk, atom) in updates {
            let mut key = b"atom_".to_vec();
            key.extend_from_slice(&pk);
            self.db.insert(key, bincode::serialize(&atom)?)?;
        }

        for (hash, code) in &changeset.contracts {
            self.save_contract(*hash, code)?;
        }

        self.db.insert("mpt_root", &new_root)?;
        let hist_key = format!("mpt_root_{}", crystal_index);
        self.db.insert(hist_key.as_bytes(), &new_root)?;

        Ok(new_root)
    }


    // ── Merkle ────────────────────────────────────────────────────────────────

    pub async fn get_proof(&self, pk: &[u8]) -> Result<MerkleProof> {
        let mut hasher = Sha3_256::new();
        hasher.update(pk);
        let trie_key: [u8; 32] = hasher.finalize().into();

        // INTDIV-001 fix: replaced blocking_read() with .read().await
        let root = *self.root.read().await;

        let store = SledMptStore::new(&self.db)?;
        let read_trie = MerklePatriciaTrie::with_root_opt(store, root);
        read_trie.prove(&trie_key)
    }

    pub async fn current_root(&self) -> [u8; 32] {
        // INTDIV-001 fix: replaced blocking_read() with .read().await
        self.root.read().await.unwrap_or([0u8; 32])
    }

    /// Return the MPT root hash at a specific crystal index.
    ///
    /// # Errors
    ///
    /// - `StorageError::ProofTooOld` if `crystal_index < current_tip - UNDO_WINDOW`.
    ///   The root has been pruned. The caller should request a more recent proof.
    /// - `Ok(None)` if the index is within UNDO_WINDOW but the root key is missing
    ///   (node was not yet at that height, or genesis edge case).
    pub fn root_at(&self, crystal_index: u64) -> Result<Option<[u8; 32]>, StorageError> {
        let tip = if let Some(idx_bytes) = self.db.get("latest_index").map_err(StorageError::Sled)? {
            let arr: [u8; 8] = idx_bytes.as_ref().try_into()
                .map_err(|_| StorageError::Other(anyhow::anyhow!("latest_index corrupt")))?;
            u64::from_le_bytes(arr)
        } else {
            0
        };

        // Reject requests beyond the undo window
        if tip > UNDO_WINDOW && crystal_index < tip - UNDO_WINDOW {
            return Err(StorageError::ProofTooOld {
                index: crystal_index,
                tip,
                window: UNDO_WINDOW,
            });
        }

        // Look up the historical root
        let hist_key = format!("mpt_root_{}", crystal_index);
        match self.db.get(hist_key.as_bytes()).map_err(StorageError::Sled)? {
            Some(b) => {
                let arr: [u8; 32] = b.as_ref()
                    .try_into()
                    .map_err(|_| StorageError::Other(
                        anyhow::anyhow!("mpt_root_{} is corrupt", crystal_index)
                    ))?;
                Ok(Some(arr))
            }
            None => Ok(None),
        }
    }

    // ── Crystal I/O ───────────────────────────────────────────────────────────

    pub fn get_crystal(&self, index: u64) -> Result<Option<Crystal>> {
        let key = format!("crystal_{}", index);
        match self.db.get(&key)? {
            Some(b) => Ok(Some(bincode::deserialize(&b)?)),
            None => Ok(None),
        }
    }

    pub fn get_crystal_latest(&self) -> Result<Option<Crystal>> {
        if let Some(idx_bytes) = self.db.get("latest_index")? {
            let arr: [u8; 8] = idx_bytes.as_ref().try_into().context("latest_index corrupt")?;
            let index = u64::from_le_bytes(arr);
            return self.get_crystal(index);
        }
        Ok(None)
    }

    pub fn save_crystal(&self, crystal: &Crystal) -> Result<()> {
        let key = format!("crystal_{}", crystal.index);
        let data = bincode::serialize(crystal)?;
        self.db.insert(&key, data)?;
        self.db.insert("latest_index", &crystal.index.to_le_bytes())?;
        Ok(())
    }

    pub fn delete_crystal(&self, index: u64) -> Result<()> {
        let crystal_key = format!("crystal_{}", index);
        self.db.remove(crystal_key.as_bytes())?;
        Ok(())
    }

    // ── UndoLog I/O ───────────────────────────────────────────────────────────

    pub fn save_undo_log(&self, log: &UndoLog) -> Result<()> {
        let key = format!("undo_{}", log.crystal_index);
        let data = bincode::serialize(log)?;
        self.db.insert(key.as_bytes(), data)?;
        Ok(())
    }

    pub fn get_undo_log(&self, crystal_index: u64) -> Result<Option<UndoLog>> {
        let key = format!("undo_{}", crystal_index);
        match self.db.get(key.as_bytes())? {
            Some(b) => Ok(Some(bincode::deserialize(&b)?)),
            None => Ok(None),
        }
    }

    pub fn delete_undo_log(&self, crystal_index: u64) -> Result<()> {
        let key = format!("undo_{}", crystal_index);
        self.db.remove(key.as_bytes())?;
        Ok(())
    }

    pub fn prune_undo_window(&self, current_height: u64) {
        use primus_storage::UNDO_WINDOW;
        if current_height <= UNDO_WINDOW { return; }
        let cutoff = current_height - UNDO_WINDOW;
        for idx in 0..cutoff {
            let key = format!("undo_{}", idx);
            let _ = self.db.remove(key.as_bytes());

            let gc_key = idx.to_le_bytes();
            if let Ok(Some(old_root_bytes)) = self.gc_log.get(gc_key) {
                if let Ok(arr) = old_root_bytes.as_ref().try_into() {
                    let old_root: [u8; 32] = arr;
                    let mut mpt = self.mpt.lock().unwrap();
                    match mpt.gc_since(old_root) {
                        Ok(n) if n > 0 => log::debug!(
                            "MPT GC: crystal #{} freed {} orphan nodes", idx, n
                        ),
                        Ok(_) => {}
                        Err(e) => log::warn!("MPT GC failed for crystal #{}: {}", idx, e),
                    }
                }
                let _ = self.gc_log.remove(gc_key);
            }
        }
    }

    pub fn restore_atoms(&self, pre_images: &BTreeMap<Vec<u8>, Option<Atom>>) -> Result<()> {
        for (pk, maybe_atom) in pre_images {
            let mut key = b"atom_".to_vec();
            key.extend_from_slice(pk);
            match maybe_atom {
                Some(atom) => {
                    self.db.insert(&key, bincode::serialize(atom)?)?;
                }
                None => {
                    self.db.remove(&key)?;
                }
            }
        }
        Ok(())
    }

    // ── Global Metrics ────────────────────────────────────────────────────────

    pub fn save_global_metrics(&self, metrics: &GlobalMetrics) -> Result<()> {
        let data = bincode::serialize(metrics)?;
        self.db.insert(b"global_metrics", data)?;
        Ok(())
    }

    pub fn get_global_metrics(&self) -> Result<Option<GlobalMetrics>> {
        match self.db.get(b"global_metrics")? {
            Some(b) => Ok(Some(bincode::deserialize(&b)?)),
            None => Ok(None),
        }
    }

    // ── Flush ─────────────────────────────────────────────────────────────────

    pub fn flush(&self, metrics: Option<&GlobalMetrics>) -> Result<()> {
        if let Some(m) = metrics {
            self.save_global_metrics(m)?;
        }
        self.db.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use primus_types::Crystal;

    #[test]
    fn root_at_returns_proof_too_old_error() {
        let dir = std::env::temp_dir().join(format!("primus_test_storage_{}", rand::random::<u32>()));
        let storage = PrimusStorage::new(dir.to_str().unwrap()).unwrap();

        // Simulate being at height UNDO_WINDOW + 10
        // by directly inserting a fake latest_index
        let fake_tip: u64 = UNDO_WINDOW + 10;
        storage.get_db()
            .insert("latest_index", &fake_tip.to_le_bytes())
            .unwrap();

        // Request a root older than the window
        let result = storage.root_at(0);
        assert!(matches!(result, Err(StorageError::ProofTooOld { .. })),
                "Expected ProofTooOld, got {:?}", result);

        // Request a root within the window — should not error
        let result2 = storage.root_at(fake_tip - 1);
        assert!(result2.is_ok(), "Expected Ok for recent index, got {:?}", result2);
    }
}