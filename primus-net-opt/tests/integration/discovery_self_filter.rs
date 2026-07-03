use primus_net_opt::discovery::PrimusDiscovery;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn self_beacon_is_ignored() {
    let port = 8123;
    let called = Arc::new(AtomicBool::new(false));
    let called_clone = called.clone();

    let discovery = PrimusDiscovery::new(port, None);
    tokio::spawn(async move {
        let _ = discovery
            .start(move |_| {
                called_clone.store(true, Ordering::SeqCst);
                async move {}
            })
            .await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let msg = format!("PRIMUS_PEER:{}", port);
    socket
        .send_to(msg.as_bytes(), format!("127.0.0.1:{}", port))
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert!(
        !called.load(Ordering::SeqCst),
        "Self-beacon should be ignored"
    );
}
