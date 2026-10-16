//! Printing (ipp.rs) and driver planning (drivers.rs) against a mock IPP
//! server speaking binary application/ipp over HTTP.
use archie_sentry::{
    adapters::{
        drivers::{analyze, DriverPlan, DriverTier},
        ipp::*,
    },
    contracts::*,
    engine::*,
    registry::*,
};
use archie_types::Timestamp;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// Mock printer state shared between the test and the server task.
#[derive(Default)]
struct Mock {
    formats: Vec<String>,
    state: i32, // IPP printer-state enum: 3 idle, 4 processing, 5 stopped
    make_model: Option<String>,
    jobs: Vec<String>,
    documents: Vec<Vec<u8>>,
}

fn attribute(body: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
    body.push(tag);
    body.extend_from_slice(&(name.len() as u16).to_be_bytes());
    body.extend_from_slice(name.as_bytes());
    body.extend_from_slice(&(value.len() as u16).to_be_bytes());
    body.extend_from_slice(value);
}
fn header(request_id: u32) -> Vec<u8> {
    let mut body = vec![1, 1, 0, 0];
    body.extend_from_slice(&request_id.to_be_bytes());
    body.push(1);
    attribute(&mut body, 0x47, "attributes-charset", b"utf-8");
    attribute(&mut body, 0x48, "attributes-natural-language", b"en");
    body
}
fn attributes_response(request_id: u32, mock: &Mock) -> Vec<u8> {
    let mut body = header(request_id);
    body.push(4);
    attribute(&mut body, 0x42, "printer-name", b"Mock Printer");
    attribute(&mut body, 0x23, "printer-state", &mock.state.to_be_bytes());
    if let Some(make_model) = &mock.make_model {
        attribute(
            &mut body,
            0x41,
            "printer-make-and-model",
            make_model.as_bytes(),
        );
    }
    for (index, format) in mock.formats.iter().enumerate() {
        attribute(
            &mut body,
            0x49,
            if index == 0 {
                "document-format-supported"
            } else {
                ""
            },
            format.as_bytes(),
        );
    }
    body.push(3);
    body
}
fn jobs_response(request_id: u32, jobs: &[String]) -> Vec<u8> {
    let mut body = header(request_id);
    for (index, job) in jobs.iter().enumerate() {
        body.push(2);
        attribute(&mut body, 0x21, "job-id", &(index as i32 + 1).to_be_bytes());
        attribute(&mut body, 0x42, "job-name", job.as_bytes());
    }
    body.push(3);
    body
}
/// Walk a Print-Job request: returns (job-name, document bytes).
fn parse_print_request(body: &[u8]) -> (Option<String>, Vec<u8>) {
    let mut at = 8;
    let mut job_name = None;
    let mut previous = String::new();
    loop {
        let tag = body[at];
        at += 1;
        if tag == 3 {
            break;
        }
        if (1..=15).contains(&tag) {
            previous.clear();
            continue;
        }
        let name_len = u16::from_be_bytes([body[at], body[at + 1]]) as usize;
        at += 2;
        if name_len > 0 {
            previous = String::from_utf8_lossy(&body[at..at + name_len]).into_owned();
        }
        at += name_len;
        let value_len = u16::from_be_bytes([body[at], body[at + 1]]) as usize;
        at += 2;
        if previous == "job-name" {
            job_name = Some(String::from_utf8_lossy(&body[at..at + value_len]).into_owned());
        }
        at += value_len;
    }
    (job_name, body[at..].to_vec())
}
fn handle(mock: &Arc<Mutex<Mock>>, body: &[u8]) -> Vec<u8> {
    let request_id = u32::from_be_bytes(body[4..8].try_into().unwrap());
    match u16::from_be_bytes([body[2], body[3]]) {
        0x000b => attributes_response(request_id, &mock.lock().unwrap()),
        0x0002 => {
            let (job_name, document) = parse_print_request(body);
            let mut mock = mock.lock().unwrap();
            mock.state = 4; // accepted work means the printer is processing
            if let Some(job_name) = job_name {
                mock.jobs.push(job_name);
            }
            mock.documents.push(document);
            // Print-Job accepted: the printer answers with the job it now holds.
            let mut response = header(request_id);
            response.push(2);
            attribute(
                &mut response,
                0x21,
                "job-id",
                &(mock.jobs.len() as i32).to_be_bytes(),
            );
            attribute(&mut response, 0x23, "job-state", &5i32.to_be_bytes());
            response.push(3);
            response
        }
        0x000a => jobs_response(request_id, &mock.lock().unwrap().jobs),
        _ => {
            let mut response = header(request_id);
            response.push(3);
            response
        }
    }
}
/// Mock IPP server: answers `count` requests via `handle`, records every
/// raw request (headers + body), then returns them.
async fn ipp_server(
    count: usize,
    mock: Arc<Mutex<Mock>>,
) -> (String, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut requests = vec![];
        for _ in 0..count {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            let mut chunk = [0; 4096];
            loop {
                let size = stream.read(&mut chunk).await.unwrap();
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..size]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let text = String::from_utf8_lossy(&request[..end]);
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let end = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let response = handle(&mock, &request[end..]);
            requests.push(request);
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(&response).await.unwrap();
        }
        requests
    });
    (format!("http://{address}"), task)
}

fn spool_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archie-print-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn resource() -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "printer".into(),
        edge_id: "edge".into(),
        adapter_id: "ipp".into(),
        paired: true,
        capabilities: vec![CommandKind::PrintSubmit],
    }
}
fn adapter(url: &str, spool: PathBuf) -> IppAdapter {
    IppAdapter::new("edge", "ipp", &format!("{url}/ipp/print"), "printer", spool).unwrap()
}
fn fixture() -> (Vec<u8>, String) {
    let content = b"%PDF-1.4 archie fixture document bytes".to_vec();
    let sha256 = format!("{:x}", Sha256::digest(&content));
    (content, sha256)
}
fn mock(formats: &[&str]) -> Arc<Mutex<Mock>> {
    Arc::new(Mutex::new(Mock {
        formats: formats.iter().map(|f| f.to_string()).collect(),
        state: 3,
        ..Mock::default()
    }))
}

#[tokio::test]
async fn verified_spool_content_is_printed_and_queue_entry_is_the_postcondition() {
    let (content, sha256) = fixture();
    let spool = spool_dir("submit");
    std::fs::write(spool.join(&sha256), &content).unwrap();
    let mock = mock(&["image/pwg-raster", "application/pdf"]);
    let (url, task) = ipp_server(4, mock.clone()).await;
    let mut adapter = adapter(&url, spool);
    let command = Command::PrintSubmit {
        sha256: sha256.clone(),
        format: PrintFormat::Pdf,
        pages: 1,
    };
    assert!(adapter.apply(&resource(), &command).await.unwrap().accepted);
    let observation = adapter.observe(&resource()).await.unwrap();
    let expected = Command::print_job_name(&sha256).unwrap();
    assert_eq!(expected, format!("archie-{}", &sha256[..16]));
    assert_eq!(
        observation.state,
        ResourceState::Printer {
            idle: false, // mock flips to processing (4) on Print-Job
            job_present: Some(expected.clone()),
        }
    );
    assert_eq!(observation.origin, ObservationOrigin::Measurement);
    assert!(command.matches(&observation.state));
    // The mock received exactly the verified document bytes, once.
    {
        let mock = mock.lock().unwrap();
        assert_eq!(mock.documents, vec![content.clone()]);
        assert_eq!(mock.jobs, vec![expected.clone()]);
    }
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 4);
    let print_request = &requests[1];
    assert!(print_request
        .windows(expected.len())
        .any(|w| w == expected.as_bytes()));
    assert!(print_request
        .windows(b"application/pdf".len())
        .any(|w| w == b"application/pdf"));
    assert!(print_request
        .windows(content.len())
        .any(|w| w == content.as_slice()));
}

#[tokio::test]
async fn spool_bytes_that_do_not_match_sha256_are_never_sent() {
    let (_, sha256) = fixture();
    let spool = spool_dir("tampered");
    std::fs::write(spool.join(&sha256), b"tampered content").unwrap();
    let (url, task) = ipp_server(0, mock(&["application/pdf"])).await;
    let mut adapter = adapter(&url, spool);
    assert_eq!(
        adapter
            .apply(
                &resource(),
                &Command::PrintSubmit {
                    sha256,
                    format: PrintFormat::Pdf,
                    pages: 1,
                }
            )
            .await,
        Err(AdapterError::Protocol)
    );
    assert!(task.await.unwrap().is_empty(), "nothing may be sent");
}

#[tokio::test]
async fn oversized_spool_file_is_refused_before_hashing_or_network() {
    let content = vec![b'x'; 16 * 1024 * 1024 + 1];
    let sha256 = format!("{:x}", Sha256::digest(&content));
    let spool = spool_dir("oversized");
    std::fs::write(spool.join(&sha256), &content).unwrap();
    let (url, task) = ipp_server(0, mock(&["application/pdf"])).await;
    let mut adapter = adapter(&url, spool);
    assert_eq!(
        adapter
            .apply(
                &resource(),
                &Command::PrintSubmit {
                    sha256,
                    format: PrintFormat::Pdf,
                    pages: 1,
                }
            )
            .await,
        Err(AdapterError::Protocol)
    );
    assert!(task.await.unwrap().is_empty());
}

#[tokio::test]
async fn undeclared_document_format_is_unsupported_and_nothing_is_sent() {
    let (content, sha256) = fixture();
    let spool = spool_dir("format");
    std::fs::write(spool.join(&sha256), &content).unwrap();
    let mock = mock(&["image/pwg-raster", "image/urf"]);
    let (url, task) = ipp_server(1, mock.clone()).await;
    let mut adapter = adapter(&url, spool);
    assert_eq!(
        adapter
            .apply(
                &resource(),
                &Command::PrintSubmit {
                    sha256,
                    format: PrintFormat::Pdf,
                    pages: 1,
                }
            )
            .await,
        Err(AdapterError::Unsupported)
    );
    // Only the Get-Printer-Attributes probe happened; no Print-Job, no bytes.
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(mock.lock().unwrap().documents.is_empty());
}

#[tokio::test]
async fn non_print_commands_are_unsupported_without_network_traffic() {
    let (url, task) = ipp_server(0, mock(&["application/pdf"])).await;
    let mut adapter = adapter(&url, spool_dir("unsupported"));
    for command in [
        Command::MediaPause,
        Command::SwitchSet { on: true },
        Command::RemoteKey {
            key: RemoteKeyName::PowerOff,
        },
    ] {
        assert_eq!(
            adapter.apply(&resource(), &command).await,
            Err(AdapterError::Unsupported)
        );
    }
    assert!(task.await.unwrap().is_empty());
}

#[tokio::test]
async fn printer_state_maps_to_idle_and_foreign_jobs_are_ignored() {
    for (state, idle) in [(3, true), (4, false), (5, false)] {
        let mock = mock(&["image/pwg-raster"]);
        mock.lock().unwrap().state = state;
        mock.lock().unwrap().jobs = vec!["someone-elses-job".into()];
        let (url, task) = ipp_server(2, mock).await;
        let mut adapter = adapter(&url, spool_dir("state"));
        let observation = adapter.observe(&resource()).await.unwrap();
        assert_eq!(
            observation.state,
            ResourceState::Printer {
                idle,
                job_present: None,
            },
            "printer-state {state}"
        );
        task.await.unwrap();
    }
}

#[tokio::test]
async fn unpaired_or_foreign_resource_is_rejected() {
    let (url, task) = ipp_server(0, mock(&["application/pdf"])).await;
    let mut adapter = adapter(&url, spool_dir("pairing"));
    let mut foreign = resource();
    foreign.paired = false;
    assert_eq!(
        adapter.observe(&foreign).await,
        Err(AdapterError::Unsupported)
    );
    let mut other = resource();
    other.resource_id = "other-printer".into();
    assert_eq!(
        adapter.observe(&other).await,
        Err(AdapterError::Unsupported)
    );
    assert!(task.await.unwrap().is_empty());
}

fn snapshot(formats: &[&str], make_model: Option<&str>) -> PrinterSnapshot {
    PrinterSnapshot {
        name: Some("fixture".into()),
        uuid: None,
        state: PrinterState::Idle,
        accepting_jobs: Some(true),
        reasons: vec![],
        document_formats: formats.iter().map(|f| f.to_string()).collect(),
        make_model: make_model.map(str::to_owned),
        observed_at: Timestamp(0),
    }
}

#[test]
fn raster_formats_mean_driverless_and_no_packages() {
    for formats in [
        &["image/pwg-raster"][..],
        &["image/urf"][..],
        &["application/pdf", "image/urf", "application/postscript"][..],
    ] {
        let plan = analyze(&snapshot(formats, Some("HP Whatever")));
        assert_eq!(plan.tier, DriverTier::Driverless);
        assert_eq!(plan.pacman_command(), None);
        assert!(plan.planned_command_line().contains("no packages required"));
    }
}

#[test]
fn legacy_pdl_on_hp_needs_filters_and_optional_hplip() {
    let plan = analyze(&snapshot(
        &["application/postscript"],
        Some("HP LaserJet 5"),
    ));
    let DriverTier::NeedsPackages { packages, reason } = &plan.tier else {
        panic!("expected NeedsPackages: {:?}", plan.tier);
    };
    for expected in ["cups-filters", "gutenprint", "foomatic-db", "hplip"] {
        assert!(packages.iter().any(|p| p == expected), "missing {expected}");
    }
    assert!(reason.contains("optional") && reason.contains("driverless"));
}

#[test]
fn legacy_pdl_with_scanner_and_without_hp_gets_sane_not_hplip() {
    let plan = analyze(&snapshot(
        &["application/vnd.hp-pcl"],
        Some("Acme Scanner 3000"),
    ));
    let DriverTier::NeedsPackages { packages, reason } = &plan.tier else {
        panic!("expected NeedsPackages: {:?}", plan.tier);
    };
    assert!(packages.iter().any(|p| p == "sane"));
    assert!(!packages.iter().any(|p| p == "hplip"));
    assert!(reason.contains("sane"));
}

#[test]
fn unrecognised_printer_is_unknown_with_the_exact_attributes_considered() {
    let plan = analyze(&snapshot(&["application/octet-stream"], None));
    let DriverTier::Unknown { considered } = &plan.tier else {
        panic!("expected Unknown: {:?}", plan.tier);
    };
    assert_eq!(considered, &["application/octet-stream".to_string()]);
    assert_eq!(plan.pacman_command(), None);
    assert!(plan
        .planned_command_line()
        .contains("application/octet-stream"));
}

#[test]
fn pacman_command_is_data_only_and_never_executed() {
    let plan = analyze(&snapshot(
        &["application/postscript"],
        Some("HP LaserJet 5"),
    ));
    let command = plan.pacman_command().unwrap();
    assert_eq!(&command[..4], ["pacman", "-S", "--needed", "--noconfirm"]);
    assert!(command.len() > 4);
    let line = plan.planned_command_line();
    assert!(line.starts_with("pacman -S --needed --noconfirm "));
    assert!(line.contains("cups-filters"));
    // "Never executed" is structural: drivers.rs contains no process-spawning
    // code path at all, so there is nothing in the module that could run this.
    const SOURCE: &str = include_str!("../src/adapters/drivers.rs");
    for forbidden in ["process::Command", ".spawn(", ".status(", ".output("] {
        assert!(
            !SOURCE.contains(forbidden),
            "drivers.rs must not contain {forbidden}"
        );
    }
}

#[test]
fn invalid_or_unbounded_package_lists_produce_no_command() {
    let bad = DriverPlan {
        tier: DriverTier::NeedsPackages {
            packages: vec!["cups-filters".into(), "bad name!; rm -rf /".into()],
            reason: "fixture".into(),
        },
    };
    assert_eq!(bad.pacman_command(), None);
    let unbounded = DriverPlan {
        tier: DriverTier::NeedsPackages {
            packages: (0..65).map(|i| format!("pkg{i}")).collect(),
            reason: "fixture".into(),
        },
    };
    assert_eq!(unbounded.pacman_command(), None);
}
