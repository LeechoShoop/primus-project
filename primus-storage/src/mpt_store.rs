use crate::mpt::{Hash32, MptNode, MptStore};
use sled::Tree;

pub struct SledMptStore {
    tree: Tree, // sled tree named "mpt_nodes"
}

impl SledMptStore {
    pub fn new(db: &sled::Db) -> anyhow::Result<Self> {
        Ok(Self { tree: db.open_tree("mpt_nodes")? })
    }
}

impl MptStore for SledMptStore {
    fn get_node(&self, hash: &Hash32) -> anyhow::Result<Option<MptNode>> {
        match self.tree.get(hash)? {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    fn put_node(&self, node: &MptNode) -> anyhow::Result<Hash32> {
        let hash  = node.hash();
        let bytes = bincode::serialize(node)?;
        self.tree.insert(hash, bytes)?;
        Ok(hash)
    }

    fn delete_node(&self, hash: &Hash32) -> anyhow::Result<()> {
        self.tree.remove(hash)?;
        Ok(())
    }
}
