use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

#[tokio::test]
async fn frame_size_limit_enforcement() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut framed = FramedWrite::new(
            &mut socket,
            LengthDelimitedCodec::builder()
                .max_frame_length(20 * 1024 * 1024)
                .new_codec(),
        );
        let large_payload = vec![0u8; 17 * 1024 * 1024];
        let _ = framed.send(large_payload.into()).await;
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut framed_read = FramedRead::new(
        stream,
        LengthDelimitedCodec::builder()
            .max_frame_length(16 * 1024 * 1024)
            .new_codec(),
    );

    let result = framed_read.next().await;
    assert!(result.is_some());
    let err = result.unwrap().unwrap_err();
    assert!(
        err.to_string().contains("frame size too big")
            || err.to_string().contains("frame size limit exceeded")
    );
}
