//! The Tavily client, against a real socket.
//!
//! Every test runs a loopback listener and speaks actual HTTP to it. The assertions
//! about what Archie sends are made on the bytes the server received, because a test
//! that inspects its own request builder proves only that the builder was called.

use archie_search::{
    AuthorizedSearch, SearchAuthority, SearchError, SearchGate, SearchOptions, SearchProposal,
    SearchProviderManifest, SearchRegistry, SearchTopic, TavilyClient, TimeRange,
};
use archie_types::capability::well_known;
use archie_types::PrivacyGrade;
use archie_vault::{EntryClass, SealedCredential, Vault, VaultKey, VaultRef};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

const T0: i64 = 1_760_000_000_000;
const KEY: &str = "tvly-test-0123456789abcdef";
const QUERY: &str = "ArchieOS capability-based scheduling";

/// What the provider answers.
struct Reply {
    status: &'static str,
    headers: Vec<String>,
    body: String,
    declare_length: bool,
}

impl Reply {
    fn json(status: &'static str, body: Value) -> Self {
        Self {
            status,
            headers: vec!["Content-Type: application/json".into()],
            body: body.to_string(),
            declare_length: true,
        }
    }

    fn text(status: &'static str, body: &str) -> Self {
        Self {
            status,
            headers: vec!["Content-Type: text/plain".into()],
            body: body.into(),
            declare_length: true,
        }
    }
}

/// One request, as the server received it.
struct Seen {
    head: String,
    body: String,
}

async fn read_request(socket: &mut TcpStream) -> Seen {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        let read = socket.read(&mut chunk).await.unwrap();
        if read == 0 {
            break buffer.len();
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let read = socket.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let end = (header_end + length).min(buffer.len());
    Seen {
        head,
        body: String::from_utf8_lossy(&buffer[header_end..end]).to_string(),
    }
}

/// A provider that answers each connection in turn and reports what it was sent.
async fn serve(replies: Vec<Reply>) -> (String, oneshot::Receiver<Vec<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let mut seen = Vec::new();
        for reply in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            seen.push(read_request(&mut socket).await);
            let mut response = format!("HTTP/1.1 {}\r\nConnection: close\r\n", reply.status);
            for header in &reply.headers {
                response.push_str(header);
                response.push_str("\r\n");
            }
            if reply.declare_length {
                response.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
            }
            response.push_str("\r\n");
            response.push_str(&reply.body);
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
        let _ = sender.send(seen);
    });
    (format!("http://{address}"), receiver)
}

/// A listener that only records whether anybody connected within half a second.
async fn listener_expecting_nobody() -> (String, tokio::task::JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let connected = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_millis(500), listener.accept())
            .await
            .is_ok()
    });
    (url, connected)
}

fn manifest(endpoint: &str) -> SearchProviderManifest {
    let mut manifest =
        SearchProviderManifest::tavily(VaultRef::parse("vault:tavily/main").unwrap());
    manifest.endpoint = endpoint.into();
    manifest.timeout_ms = 5_000;
    manifest.max_response_bytes = 65_536;
    manifest
}

fn authorize(manifest: &SearchProviderManifest, options: SearchOptions) -> AuthorizedSearch {
    let mut registry = SearchRegistry::new();
    registry.register(manifest.clone()).unwrap();
    let authority = SearchAuthority::new([well_known::network_internet()].into_iter().collect());
    SearchGate::authorize(
        &registry,
        &authority,
        &SearchProposal::new(
            &manifest.id,
            QUERY,
            PrivacyGrade::Public,
            "task-1",
            "trace-1",
        )
        .with_options(options),
    )
    .allowed()
    .expect("the gate should admit a public query")
    .clone()
}

fn credential(reference: &str) -> SealedCredential {
    let mut vault = Vault::open(&VaultKey::from_bytes([7_u8; 32])).unwrap();
    let reference = VaultRef::parse(reference).unwrap();
    vault
        .store(
            "test",
            reference.clone(),
            EntryClass::Credential,
            PrivacyGrade::Secret,
            KEY,
            T0,
            None,
        )
        .unwrap();
    vault.broker_credential("test", &reference, T0).unwrap()
}

fn one_result() -> Value {
    json!({
        "query": QUERY,
        "results": [
            {"title": "Capability scheduling", "url": "https://a.example/paper", "content": "alpha", "score": 0.912, "published_date": "2026-09-01"}
        ],
        "response_time": 1.2,
        "request_id": "req-1"
    })
}

#[tokio::test]
async fn a_search_sends_the_key_as_a_bearer_header_and_only_the_declared_options() {
    let (url, seen) = serve(vec![Reply::json("200 OK", one_result())]).await;
    let manifest = manifest(&url);
    let search = authorize(
        &manifest,
        SearchOptions {
            topic: SearchTopic::News,
            max_results: 3,
            time_range: Some(TimeRange::Week),
            include_domains: vec!["arxiv.org".into()],
            ..SearchOptions::default()
        },
    );

    let client = TavilyClient::connect(&manifest).unwrap();
    let results = client
        .search(&search, credential("vault:tavily/main"))
        .await
        .unwrap();

    let requests = seen.await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert!(
        request.head.starts_with("POST /search HTTP/1.1"),
        "{}",
        request.head
    );
    assert!(
        request
            .head
            .contains(&format!("authorization: Bearer {KEY}")),
        "{}",
        request.head
    );

    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(body["query"], QUERY);
    assert_eq!(body["search_depth"], "basic");
    assert_eq!(body["topic"], "news");
    assert_eq!(body["max_results"], 3);
    assert_eq!(body["time_range"], "week");
    assert_eq!(body["include_domains"], json!(["arxiv.org"]));
    assert!(
        body.get("exclude_domains").is_none(),
        "empty lists are not sent"
    );
    // Everything Archie does not use is switched off explicitly.
    for off in [
        "include_answer",
        "include_raw_content",
        "include_images",
        "include_image_descriptions",
        "include_favicon",
        "auto_parameters",
    ] {
        assert_eq!(body[off], false, "{off} must be sent as false");
    }
    // The key travels in the header and nowhere else; nothing locates the operator.
    assert!(!request.body.contains(KEY));
    for absent in ["api_key", "country", "language"] {
        assert!(body.get(absent).is_none(), "{absent} must not be sent");
    }

    assert_eq!(results.provider_id, "tavily");
    assert_eq!(results.results.len(), 1);
    let hit = &results.results[0];
    assert_eq!(hit.title, "Capability scheduling");
    assert_eq!(hit.url, "https://a.example/paper");
    assert_eq!(hit.score_permille, 912);
    assert_eq!(hit.published.as_deref(), Some("2026-09-01"));
    assert!(hit.reference.starts_with("web:"));
    assert_eq!(results.response_time_ms, Some(1200));
}

#[tokio::test]
async fn results_are_sanitised_and_capped_at_the_number_asked_for() {
    let body = json!({
        "results": [
            {"title": "run\u{1b}[2J this", "url": "https://one.example/", "content": "one", "score": 0.9},
            {"title": "script", "url": "javascript:alert(1)", "content": "bad", "score": 0.8},
            {"title": "one again", "url": "https://one.example/", "content": "dup", "score": 0.7},
            {"title": "two", "url": "https://two.example/", "content": "two", "score": 0.6},
            {"title": "three", "url": "https://three.example/", "content": "three", "score": 0.5}
        ]
    });
    let (url, _seen) = serve(vec![Reply::json("200 OK", body)]).await;
    let manifest = manifest(&url);
    let search = authorize(
        &manifest,
        SearchOptions {
            max_results: 2,
            ..SearchOptions::default()
        },
    );
    let results = TavilyClient::connect(&manifest)
        .unwrap()
        .search(&search, credential("vault:tavily/main"))
        .await
        .unwrap();

    let titles: Vec<&str> = results.results.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, vec!["run [2J this", "two"]);
    assert_eq!(results.discarded, 3);
}

#[tokio::test]
async fn a_redirect_is_refused_and_the_query_goes_nowhere_else() {
    let (elsewhere, connected) = listener_expecting_nobody().await;
    let redirect = Reply {
        status: "307 Temporary Redirect",
        headers: vec![format!("Location: {elsewhere}/search")],
        body: String::new(),
        declare_length: true,
    };
    let (url, _seen) = serve(vec![redirect]).await;
    let manifest = manifest(&url);
    let search = authorize(&manifest, SearchOptions::default());

    let error = TavilyClient::connect(&manifest)
        .unwrap()
        .search(&search, credential("vault:tavily/main"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, SearchError::Redirected { status: 307 }),
        "{error:?}"
    );
    assert!(
        !connected.await.unwrap(),
        "the redirect target must never receive the query"
    );
}

#[tokio::test]
async fn an_oversized_response_is_refused_whether_or_not_it_declares_its_length() {
    let big = format!(r#"{{"results":[],"padding":"{}"}}"#, "x".repeat(4_096));
    let declared = Reply::text("200 OK", &big);
    let undeclared = Reply {
        declare_length: false,
        ..Reply::text("200 OK", &big)
    };
    let (url, _seen) = serve(vec![declared, undeclared]).await;
    let mut manifest = manifest(&url);
    manifest.max_response_bytes = 1_024;
    let client = TavilyClient::connect(&manifest).unwrap();

    for _ in 0..2 {
        let search = authorize(&manifest, SearchOptions::default());
        let error = client
            .search(&search, credential("vault:tavily/main"))
            .await
            .unwrap_err();
        assert!(
            matches!(error, SearchError::ResponseTooLarge { budget: 1_024 }),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn provider_failures_become_stable_codes_that_leak_neither_query_nor_key() {
    let echo = format!("bad request for '{QUERY}' with key {KEY}");
    let (url, _seen) = serve(vec![
        Reply::text("400 Bad Request", &echo),
        Reply::text("401 Unauthorized", &echo),
        Reply::text("429 Too Many Requests", ""),
        Reply::text("432 Plan Limit", ""),
        Reply::text("433 Spend Limit", ""),
        Reply::text("500 Internal Server Error", &echo),
    ])
    .await;
    let manifest = manifest(&url);
    let client = TavilyClient::connect(&manifest).unwrap();

    let mut codes = Vec::new();
    for _ in 0..6 {
        let search = authorize(&manifest, SearchOptions::default());
        let error = client
            .search(&search, credential("vault:tavily/main"))
            .await
            .unwrap_err();
        for rendered in [error.to_string(), format!("{error:?}")] {
            assert!(!rendered.contains(KEY), "{rendered}");
            assert!(!rendered.contains(QUERY), "{rendered}");
        }
        codes.push(error.code());
    }
    assert_eq!(
        codes,
        vec![
            "search_request_rejected",
            "search_key_refused",
            "search_rate_limited",
            "search_plan_limit",
            "search_spend_limit",
            "search_provider_error",
        ]
    );
}

#[tokio::test]
async fn a_key_brokered_under_another_name_is_never_sent() {
    let (url, connected) = listener_expecting_nobody().await;
    let manifest = manifest(&url);
    let search = authorize(&manifest, SearchOptions::default());

    let error = TavilyClient::connect(&manifest)
        .unwrap()
        .search(&search, credential("vault:github/main"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, SearchError::CredentialMismatch),
        "{error:?}"
    );
    assert!(
        !connected.await.unwrap(),
        "nothing may be sent with the wrong key"
    );
}

#[tokio::test]
async fn a_search_authorised_for_another_provider_is_not_sent_here() {
    let (url, connected) = listener_expecting_nobody().await;
    let here = manifest(&url);
    let mut there = manifest("http://127.0.0.1:9");
    there.id = "tavily-staging".into();
    let search = authorize(&there, SearchOptions::default());

    let error = TavilyClient::connect(&here)
        .unwrap()
        .search(&search, credential("vault:tavily/main"))
        .await
        .unwrap_err();
    assert!(matches!(error, SearchError::ProviderMismatch), "{error:?}");
    assert!(!connected.await.unwrap());
}

#[tokio::test]
async fn a_client_cannot_be_opened_on_a_plaintext_remote_endpoint() {
    let error = TavilyClient::connect(&manifest("http://api.tavily.com")).unwrap_err();
    assert!(matches!(error, SearchError::Manifest(_)), "{error:?}");
    let client = TavilyClient::connect(&manifest("https://api.tavily.com")).unwrap();
    assert!(!format!("{client:?}").contains("tvly"));
}
