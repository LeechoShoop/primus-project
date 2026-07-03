use primus_net_opt::dht::PrimusDHT;
use primus_types::PrimusNR;

#[tokio::test]
async fn find_node_empty_table_terminates() {
    let nr = PrimusNR {
        public_key: vec![0; 32],
        addr_ip: 0,
        addr_port: 0,
        signature: vec![],
        timestamp: 0,
    };
    let dht = PrimusDHT::new(&nr);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        dht.find_closest(&[1u8; 32], 20),
    )
    .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}
