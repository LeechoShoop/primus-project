use primus_net_opt::dht::{K, NodePinger, RoutingTable};
use primus_types::PrimusNR;

struct AlwaysDeadPinger;

#[async_trait::async_trait]
impl NodePinger for AlwaysDeadPinger {
    async fn ping(&self, _nr: &PrimusNR) -> bool {
        false
    }
}

#[tokio::test]
#[cfg(feature = "test-helpers")]
async fn kbucket_eviction_with_dead_pinger() {
    let local = [0u8; 32];
    let table = RoutingTable::new(local);
    let pinger = AlwaysDeadPinger;

    // Create K+1 mock NRs that all map to bucket 0
    for i in 0..=K {
        let mut id = [0u8; 32];
        id[0] = 0b1000_0000;
        id[31] = i as u8;
        let nr = PrimusNR::mock(id);
        table.insert(nr, &pinger).await;
    }

    assert_eq!(table.len().await, K);
}
