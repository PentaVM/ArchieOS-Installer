//! A node delivering to a desk over loopback, end to end: stages, latest/, the pages, and
//! the requests a desk must refuse.

use archie_bridges::delivery::{Deliverer, DeliveryKey, Envelope, Outbox};
use archie_bridges::Workspace;
use archie_desk::{open::Rules, Desk};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archie-desk-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn desk(key: &DeliveryKey, root: PathBuf) -> SocketAddr {
    let desk = Desk::new(root, key.clone(), Rules::default()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, desk.router()).await });
    address
}

fn raw(address: SocketAddr, request: &[u8]) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(request).unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stages_land_on_the_desk_and_the_queue_drains() {
    let key = DeliveryKey::generate();
    let root = scratch("root");
    let node = scratch("node");
    std::fs::create_dir_all(node.join("exports")).unwrap();
    std::fs::write(node.join("exports/bracket.step"), b"ISO-10303-21; one").unwrap();
    std::fs::write(node.join("exports/bracket-iso.png"), b"\x89PNG fake").unwrap();
    let address = desk(&key, root.clone()).await;

    let (key2, node2) = (key.clone(), node.clone());
    let report = tokio::task::spawn_blocking(move || {
        let workspace = Workspace::open(&node2).unwrap();
        let deliverer = Deliverer::new(address, key2).unwrap();
        assert!(deliverer.health().unwrap().starts_with("archie-desk"));
        let first = deliverer
            .deliver(
                &workspace,
                &[
                    "exports/bracket.step".into(),
                    "exports/bracket-iso.png".into(),
                ],
                "Bracket v2",
                "iter-0001",
                "mass 41.2 g",
                false,
            )
            .unwrap();
        std::fs::write(node2.join("exports/bracket.step"), b"ISO-10303-21; two").unwrap();
        let second = deliverer
            .deliver(
                &workspace,
                &["exports/bracket.step".into()],
                "Bracket v2",
                "iter-0001",
                "",
                false,
            )
            .unwrap();
        (first, second)
    })
    .await
    .unwrap();
    assert_eq!(report.0.delivered.len(), 2, "{:?}", report.0);
    assert!(report.1.queued.is_empty());

    let project = root.join("Bracket_v2");
    let stage = project.join("stages/iter-0001");
    assert_eq!(
        std::fs::read(stage.join("bracket.step")).unwrap(),
        b"ISO-10303-21; one"
    );
    assert_eq!(
        std::fs::read(stage.join("bracket-2.step")).unwrap(),
        b"ISO-10303-21; two",
        "a different file of the same name never replaces the first"
    );
    assert_eq!(
        std::fs::read(project.join("latest/bracket.step")).unwrap(),
        b"ISO-10303-21; two"
    );
    let page = std::fs::read_to_string(project.join("index.html")).unwrap();
    assert!(page.contains("mass 41.2 g") && page.contains("bracket-iso.png"));
    assert!(std::fs::read_to_string(root.join("index.html"))
        .unwrap()
        .contains("Bracket_v2"));

    // What waited in the outbox goes when the desk is back.
    let workspace = Workspace::open(&node).unwrap();
    std::fs::write(
        workspace.scratch("outbox").unwrap().join("pending.jsonl"),
        "{\"file\":\"exports/bracket-iso.png\",\"project\":\"late\",\"stage\":\"final\",\"note\":\"\",\"open\":false}\n",
    )
    .unwrap();
    let flushed = tokio::task::spawn_blocking(move || {
        let workspace = Workspace::open(&node).unwrap();
        Deliverer::new(address, key)
            .unwrap()
            .flush(&workspace)
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(flushed.delivered, ["late/final/bracket-iso.png"]);
    assert!(root.join("late/stages/final/bracket-iso.png").is_file());
    assert!(Outbox::new(&workspace).unwrap().pending().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forged_tampered_and_replayed_deliveries_are_refused() {
    let key = DeliveryKey::generate();
    let root = scratch("refuse");
    let address = desk(&key, root.clone()).await;
    let body = b"payload";
    let sha = archie_bridges::delivery::digest_file(&{
        let f = root
            .join("..")
            .join(format!("archie-desk-body-{}", std::process::id()));
        std::fs::write(&f, body).unwrap();
        f
    })
    .unwrap()
    .0;
    let envelope = Envelope {
        project: "p".into(),
        stage: "s".into(),
        file: "f.txt".into(),
        sha256: sha,
        bytes: body.len() as u64,
        stamp_ms: archie_bridges::delivery::now_ms(),
        note: String::new(),
        open: false,
    };
    let request = move |signing: &DeliveryKey, payload: &[u8]| {
        let mut head = format!(
            "PUT {} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n",
            envelope.path(),
            payload.len()
        );
        for (name, value) in envelope.headers(signing) {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(payload);
        bytes
    };
    let answers = tokio::task::spawn_blocking(move || {
        let forged = raw(address, &request(&DeliveryKey::generate(), body));
        let tampered = raw(address, &request(&key, b"PAYLOAD"));
        let good = raw(address, &request(&key, body));
        let replayed = raw(address, &request(&key, body));
        (forged, tampered, good, replayed)
    })
    .await
    .unwrap();
    assert!(answers.0.starts_with("HTTP/1.1 401"), "{}", answers.0);
    assert!(answers.1.starts_with("HTTP/1.1 400"), "{}", answers.1);
    assert!(answers.2.starts_with("HTTP/1.1 201"), "{}", answers.2);
    assert!(answers.3.starts_with("HTTP/1.1 409"), "{}", answers.3);
    assert!(
        !root.join("p/.incoming").read_dir().unwrap().any(|_| true),
        "nothing half-written is left"
    );
}
