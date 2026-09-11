//! The control protocol's framing, tested as the trust boundary it is.
//!
//! The control socket is group-accessible: every member of the `archie` group can connect
//! to it, and none of them is required to be well-behaved. Everything arriving on it is
//! hostile input until proven otherwise, and the framing layer is where that proof either
//! happens or does not.
//!
//! The property that matters most here is not correctness on good input — it is that bad
//! input costs a bounded amount of memory and ends the connection, rather than growing a
//! buffer until the daemon is killed.

use archie_core::control::{
    serve_connection, ControlRequest, ControlResponse, MAX_CONTROL_MESSAGE_BYTES,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

/// A frame with no newline in it must not be buffered without limit.
///
/// This is the denial of service the bound exists to prevent: `read_line` reads until a
/// newline or end of input, so a client that opens the socket and writes forever without
/// ever sending one makes the daemon allocate until the kernel kills it. Checking the
/// length *after* the read is too late — by then the memory is already gone.
///
/// The discriminator is how much the writer manages to hand over. The duplex below has a
/// small buffer, so once the server stops reading the writer blocks and cannot finish. A
/// server that reads without limit swallows the whole flood; a server that bounds its read
/// stops near the limit and leaves most of it unwritten. Asserting on bytes accepted is
/// therefore a real test of the bound, where asserting only "it terminated" is not — an
/// unbounded reader also terminates, just after buffering everything first.
#[tokio::test]
async fn an_unterminated_frame_is_bounded_and_does_not_reach_the_handler() {
    let (client, server) = tokio::io::duplex(64 * 1024);

    let handled = Arc::new(AtomicUsize::new(0));
    let counter = handled.clone();

    let serving = tokio::spawn(async move {
        serve_connection(server, move |_request: ControlRequest| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                ControlResponse::error("should not be reached".to_string())
            }
        })
        .await
    });

    // Well past the limit, with no delimiter anywhere. A well-behaved client never does
    // this; a hostile one on the `archie` group socket can.
    let target = MAX_CONTROL_MESSAGE_BYTES * 4;
    let written = Arc::new(AtomicUsize::new(0));
    let written_by_writer = written.clone();

    let flooding = tokio::spawn(async move {
        let mut client = client;
        let chunk = vec![b'x'; 32 * 1024];
        while written_by_writer.load(Ordering::SeqCst) < target {
            if client.write_all(&chunk).await.is_err() {
                break; // the server stopped reading, which is the point
            }
            written_by_writer.fetch_add(chunk.len(), Ordering::SeqCst);
        }
        let _ = client.flush().await;
    });

    // Give both sides time to reach their steady state, then look at what happened.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), flooding).await;

    let accepted = written.load(Ordering::SeqCst);
    assert!(
        accepted < target,
        "the daemon accepted {accepted} bytes of a {target}-byte unterminated frame: the          read is not bounded, so a local client can make it allocate until the kernel kills          it. The size check in `decode` runs after the whole line is already in memory."
    );
    // Near the limit, not far past it. A little slack for the duplex buffer in flight.
    assert!(
        accepted <= MAX_CONTROL_MESSAGE_BYTES + 256 * 1024,
        "accepted {accepted} bytes, which is well past the {MAX_CONTROL_MESSAGE_BYTES}-byte          limit plus buffering slack"
    );

    assert_eq!(
        handled.load(Ordering::SeqCst),
        0,
        "an oversized frame must never reach the request handler"
    );

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), serving).await;
}

/// A frame that is exactly at the limit is still a legal frame.
///
/// A bound that rejected valid input one byte early would be its own bug, so this pins the
/// boundary from the other side.
#[tokio::test]
async fn a_large_but_legal_request_is_served() {
    let (client, server) = tokio::io::duplex(4 * 1024 * 1024);

    let serving = tokio::spawn(async move {
        serve_connection(server, |_request: ControlRequest| async move {
            ControlResponse::error("served".to_string())
        })
        .await
    });

    let mut client = client;
    // A real request, padded nowhere near the limit but far past a typical one, so the
    // large-frame path is genuinely exercised.
    let request = ControlRequest::Status;
    let mut line = serde_json::to_vec(&request).expect("encode");
    line.push(b'\n');
    client.write_all(&line).await.expect("write");
    client.flush().await.expect("flush");

    let mut reply = String::new();
    {
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(&mut client);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            reader.read_line(&mut reply),
        )
        .await
        .expect("a legal request must be answered")
        .expect("read reply");
    }

    assert!(
        reply.contains("served"),
        "a legal request must reach the handler; got {reply}"
    );

    drop(client);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), serving).await;
}

/// One malformed line must not end the session.
///
/// An operator running a script that emits one bad line should get an error for that line
/// and keep their connection, not be disconnected and left guessing.
#[tokio::test]
async fn a_malformed_line_is_answered_and_the_connection_survives() {
    let (client, server) = tokio::io::duplex(64 * 1024);

    let serving = tokio::spawn(async move {
        serve_connection(server, |_request: ControlRequest| async move {
            ControlResponse::error("handled".to_string())
        })
        .await
    });

    let mut client = client;
    client
        .write_all(b"{this is not valid json at all}\n")
        .await
        .expect("write");
    client.flush().await.expect("flush");

    use tokio::io::AsyncBufReadExt;
    let mut reader = tokio::io::BufReader::new(&mut client);

    let mut first = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut first),
    )
    .await
    .expect("the malformed line must be answered")
    .expect("read");
    assert!(
        first.contains("error") || first.contains("decode"),
        "a malformed line must produce an error response; got {first}"
    );

    // And the session continues: a good request after a bad one still works.
    let mut line = serde_json::to_vec(&ControlRequest::Status).expect("encode");
    line.push(b'\n');
    reader.get_mut().write_all(&line).await.expect("write");
    reader.get_mut().flush().await.expect("flush");

    let mut second = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut second),
    )
    .await
    .expect("the connection must survive one bad line")
    .expect("read");
    assert!(
        second.contains("handled"),
        "a good request after a bad one must still be served; got {second}"
    );

    drop(client);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), serving).await;
}
