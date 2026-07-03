use primus_net_opt::gossip::GossipService;
use primus_net_opt::network::{PrimusMessage, PrimusNetwork};
use primus_types::PrimusNR;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

mockall::mock! {
    pub Core {}
    #[async_trait::async_trait]
    impl primus_net_opt::network::CoreHandle for Core {
        async fn on_reaction(&self, rx: primus_types::reaction::SignedReaction) -> anyhow::Result<()>;
        async fn on_crystal(&self, crystal_bytes: Vec<u8>) -> anyhow::Result<()>;
        async fn get_crystal_bytes(&self, index: u64) -> Option<Vec<u8>>;
        async fn local_state(&self) -> (u64, f32, f32);
        async fn is_syncing(&self) -> bool;
        async fn set_sync_target(&self, height: u64);
        async fn finish_sync(&self);
        async fn get_atom_state(&self, addr: Vec<u8>) -> anyhow::Result<(u64, u64, [u8; 32], String)>;
        async fn push_bytes(&self, bytes: &[u8]) -> anyhow::Result<()>;
        async fn on_get_proof(&self, addr: Vec<u8>) -> anyhow::Result<primus_types::MerkleProof>;
    }
}

#[tokio::test]
async fn gossip_dedup_spreads_once() {
    let mut mock_core = MockCore::new();

    // We expect exactly 1 call to push_bytes (local ingress).
    // The second spread() call will be filtered out by the deduplication set.
    mock_core.expect_push_bytes().times(1).returning(|_| Ok(()));

    // Wrap the mock in an Arc to satisfy the CoreHandle trait bounds
    let core = Arc::new(mock_core);

    // Create a dummy Network Record (PrimusNR) needed to initialize the DHT routing table
    let nr = PrimusNR {
        public_key: vec![0; 32],
        addr_ip: 0,
        addr_port: 0,
        signature: vec![],
        timestamp: 0,
    };

    // Initialize the real PrimusDHT and the atomic frame drop counter
    let dht = Arc::new(primus_net_opt::dht::PrimusDHT::new(&nr));
    let frame_drops = Arc::new(AtomicU64::new(0));

    // Construct the network layer with the exact signature required by network.rs
    let mut net = PrimusNetwork::new(8000, core.clone(), dht, frame_drops);

    // Initialize gossip and inject it back into the network stack
    let gossip = GossipService::new(net.clone());
    net.set_gossip(Arc::new(gossip.clone()));

    let msg = PrimusMessage::NewReaction(vec![1, 2, 3], 10);

    // Trigger the spread twice with the identical message
    gossip.spread(msg.clone(), None).await;
    gossip.spread(msg, None).await;

    // Yield execution slightly to allow tokio::spawn tasks inside spread() to complete
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Forcefully drop the network and gossip services.
    // This removes all Arc clones of our mock_core scattered in the background tasks.
    drop(gossip);
    drop(net);

    // Unwrap the mock from the Arc. If mock expectations (e.g., times(1)) were not met,
    // this will panic here, causing a clean and descriptive test failure.
    let _final_mock = Arc::into_inner(core)
        .expect("All other Arc references to CoreHandle should be dropped by now");
}