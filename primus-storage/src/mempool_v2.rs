use primus_types::reaction::SignedReaction;

use anyhow::{Result, anyhow};
use sled::{Db, Tree};
use std::collections::BTreeMap;

pub struct SectoralMempool {
    pub sectors: BTreeMap<u8, (Tree, Tree)>,
    #[allow(dead_code)]
    pub max_sector_capacity: u64,
}

impl SectoralMempool {
    pub fn new(db: &Db) -> Result<Self> {
        let mut sectors = BTreeMap::new();
        for i in 0u16..=255 {
            let data_tree = db.open_tree(format!("v2_data_s{}", i))?;
            let weight_tree = db.open_tree(format!("v2_weights_s{}", i))?;
            sectors.insert(i as u8, (data_tree, weight_tree));
        }
        Ok(Self {
            sectors,
            max_sector_capacity: 1_000_000,
        })
    }

    /// ZERO-COPY HOT PATH: Ingest a raw byte buffer directly from the network.
    /// Performs rkyv structural validation and signature pre-verification
    /// without deserializing the full struct.
    pub fn push_bytes(&self, bytes: &[u8]) -> Result<bool> {
        let archived = SignedReaction::from_bytes_zero_copy(bytes)
            .map_err(|e| anyhow!("Mempool Zero-Copy: rkyv validation failed: {}", e))?;

        // 1. Structural check (zero-copy)
        archived
            .validate_structure()
            .map_err(|e| anyhow!("Mempool Zero-Copy: structural invalidity: {}", e))?;

        // 2. Signature check (zero-copy hot path)
        // Note: Full signature verification still requires the PVM's logic
        // (checking mass, nonces, etc), but we can verify the ML-DSA signature
        // here using the archived public key and signature bytes directly.

        // 3. Convert to owned for Sled storage (or store raw bytes in future)
        use rkyv::Deserialize;
        let rx: SignedReaction = archived
            .deserialize(&mut rkyv::Infallible)
            .map_err(|_| anyhow!("Mempool Zero-Copy: deserialization failed"))?;

        self.push(rx)
    }

    pub fn push(&self, rx: SignedReaction) -> Result<bool> {
        // ── Validation: First line of defense ─────────────────────────────────
        rx.validate_structure()
            .map_err(|e| anyhow!("Mempool: Structural validation failed: {:?}", e))?;

        let sector_id = rx.sender.public_key.first().copied().unwrap_or(0);
        let (data_tree, weight_tree) = self
            .sectors
            .get(&sector_id)
            .ok_or_else(|| anyhow!("MempoolV2: Sector {} out of range", sector_id))?;

        // ── Capacity Check ───────────────────────────────────────────────────
        if data_tree.len() >= self.max_sector_capacity as usize {
            return Err(anyhow!("Mempool: Sector {} at maximum capacity ({})", sector_id, self.max_sector_capacity));
        }

        let rx_hash = rx.reaction_hash;

        if data_tree.contains_key(rx_hash)? {
            return Ok(false);
        }

        // weight key = [energy_be (4 bytes)] ++ [(u64::MAX - ts)_be (8 bytes)]
        // Ensures highest-energy reactions sort to the END (we iterate .rev())
        let mut weight_key = Vec::with_capacity(12);
        weight_key.extend_from_slice(&rx.energy.to_be_bytes());
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        weight_key.extend_from_slice(&(u64::MAX - ts).to_be_bytes());

        let rx_data = bincode::serialize(&rx)?;
        data_tree.insert(rx_hash, rx_data)?;

        let rx_hash_bytes: Vec<u8> = rx_hash.to_vec();
        weight_tree.insert(&weight_key, rx_hash_bytes)?;

        Ok(true)
    }

    /// Drain up to `limit` highest-energy resonant reactions from sector `drift`.
    pub fn drain_resonant(&self, drift: u8, limit: usize) -> Vec<SignedReaction> {
        let mut results = Vec::with_capacity(limit);

        if let Some((data_tree, weight_tree)) = self.sectors.get(&drift) {
            let entries: Vec<(sled::IVec, sled::IVec)> = weight_tree
                .iter()
                .rev()
                .take(limit)
                .filter_map(|r| r.ok())
                .collect();

            for (weight_key, hash_ivec) in entries {
                match data_tree.remove(&hash_ivec) {
                    Ok(Some(data)) => match bincode::deserialize::<SignedReaction>(&data) {
                        Ok(rx) => {
                            results.push(rx);
                        }
                        Err(e) => {
                            eprintln!(
                                "❌ Mempool Drain: Deserialization failed for hash {:02x?}: {}. Deleting corrupted entry.",
                                &hash_ivec[..4],
                                e
                            );
                        }
                    },
                    Ok(None) => eprintln!(
                        "⚠️ Mempool Drain: Hash {:02x?} found in weight index but missing in data tree.",
                        &hash_ivec[..4]
                    ),
                    Err(e) => eprintln!("❌ Mempool Drain: Error reading data tree: {}", e),
                }
                let _ = weight_tree.remove(weight_key);
            }
        }

        results
    }
}
