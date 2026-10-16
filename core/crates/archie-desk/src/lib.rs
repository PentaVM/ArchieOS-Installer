//! # archie-desk
//!
//! The person's end of Archie's delivery stream. It runs on the workstation, listens on
//! loopback only, and takes the files a node delivers through the ssh tunnel the
//! workstation opened:
//!
//! ```text
//! ~/Archie/<project>/stages/<stage>/<file>   every stage, as delivered, never overwritten
//! ~/Archie/<project>/latest/<file>           the newest file of each name, for an app
//!                                            that keeps a file open
//! ~/Archie/<project>/index.html              the stages with their previews, reloading
//! ~/Archie/index.html                        every project
//! ```
//!
//! A delivery is accepted only when its HMAC verifies with the paired key, its timestamp
//! is within five minutes, it was not seen before, and the bytes that arrived are the
//! bytes it declared. A file is opened in the person's application only when their
//! `desk.toml` names an application for that kind of file.

#![forbid(unsafe_code)]

pub mod gallery;
pub mod open;

use archie_bridges::delivery::{is_name, now_ms, DeliveryKey, Envelope, MAX_FILE, MAX_SKEW_MS};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::Router;
use futures_util::StreamExt;
use gallery::{FileEntry, Project};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub struct Desk {
    root: PathBuf,
    key: DeliveryKey,
    rules: open::Rules,
    /// Signatures seen within the skew window: a replayed request is refused.
    seen: Mutex<HashMap<String, u64>>,
    /// One delivery is written at a time, so a project's record is never raced.
    writing: Mutex<()>,
}

impl Desk {
    pub fn new(root: PathBuf, key: DeliveryKey, rules: open::Rules) -> std::io::Result<Arc<Self>> {
        std::fs::create_dir_all(&root)?;
        Ok(Arc::new(Self {
            root: root.canonicalize()?,
            key,
            rules,
            seen: Mutex::new(HashMap::new()),
            writing: Mutex::new(()),
        }))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/v1/health", get(health))
            .route("/v1/deliver/:project/:stage/:file", put(deliver))
            .layer(DefaultBodyLimit::disable())
            .with_state(self)
    }

    fn project_dir(&self, project: &str) -> PathBuf {
        self.root.join(project)
    }

    fn load(&self, project: &str) -> Project {
        std::fs::read_to_string(self.project_dir(project).join("stages.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| Project {
                name: project.to_string(),
                stages: Vec::new(),
            })
    }

    fn save(&self, project: &Project) -> std::io::Result<()> {
        let dir = self.project_dir(&project.name);
        write_atomic(
            &dir.join("stages.json"),
            serde_json::to_string_pretty(project)?.as_bytes(),
        )?;
        write_atomic(
            &dir.join("index.html"),
            gallery::project_page(project).as_bytes(),
        )?;
        let mut projects = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_name(&name) && entry.path().join("stages.json").is_file() {
                    projects.push(self.load(&name));
                }
            }
        }
        write_atomic(
            &self.root.join("index.html"),
            gallery::home_page(&projects).as_bytes(),
        )
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let staged = path.with_extension("archie-new");
    std::fs::write(&staged, bytes)?;
    std::fs::rename(&staged, path)
}

async fn health() -> impl IntoResponse {
    format!("archie-desk {}", env!("CARGO_PKG_VERSION"))
}

fn refuse(status: StatusCode, why: impl Into<String>) -> axum::response::Response {
    (status, why.into()).into_response()
}

async fn deliver(
    State(desk): State<Arc<Desk>>,
    UrlPath((project, stage, file)): UrlPath<(String, String, String)>,
    headers: HeaderMap,
    body: Body,
) -> axum::response::Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let Some(bytes) = header("content-length").and_then(|v| v.parse::<u64>().ok()) else {
        return refuse(
            StatusCode::LENGTH_REQUIRED,
            "a delivery declares its length",
        );
    };
    if bytes > MAX_FILE {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "larger than a delivery may be",
        );
    }
    let envelope = match Envelope::from_request(&project, &stage, &file, bytes, header) {
        Ok(envelope) => envelope,
        Err(why) => return refuse(StatusCode::BAD_REQUEST, why),
    };
    let now = now_ms();
    if envelope.stamp_ms.abs_diff(now) > MAX_SKEW_MS {
        return refuse(
            StatusCode::UNAUTHORIZED,
            "the timestamp is too far from this computer's clock",
        );
    }
    let Some(signature) = header("x-archie-signature") else {
        return refuse(StatusCode::UNAUTHORIZED, "unsigned");
    };
    if !envelope.verify(&desk.key, &signature) {
        return refuse(
            StatusCode::UNAUTHORIZED,
            "the signature does not verify with the paired key",
        );
    }
    {
        let mut seen = desk.seen.lock().await;
        seen.retain(|_, stamp| stamp.abs_diff(now) <= 2 * MAX_SKEW_MS);
        if seen.contains_key(&signature) {
            return refuse(StatusCode::CONFLICT, "this delivery was already received");
        }
    }

    // The bytes go to a hidden file first and become the delivery only if they are
    // exactly what was signed.
    let project_dir = desk.project_dir(&envelope.project);
    let incoming = project_dir.join(".incoming");
    if let Err(e) = tokio::fs::create_dir_all(&incoming).await {
        return refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    static ARRIVALS: AtomicU64 = AtomicU64::new(0);
    let temporary = incoming.join(format!(
        "{}-{}.part",
        &signature[..16],
        ARRIVALS.fetch_add(1, Ordering::Relaxed)
    ));
    let received = receive(body, &temporary, bytes).await;
    let sha256 = match received {
        Ok(sha256) => sha256,
        Err(why) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return refuse(StatusCode::BAD_REQUEST, why);
        }
    };
    if sha256 != envelope.sha256 {
        let _ = tokio::fs::remove_file(&temporary).await;
        return refuse(
            StatusCode::BAD_REQUEST,
            "the bytes are not the ones that were signed",
        );
    }
    // Only a delivery that arrived whole counts as seen: a cut connection can be retried.
    if desk
        .seen
        .lock()
        .await
        .insert(signature.clone(), envelope.stamp_ms)
        .is_some()
    {
        let _ = tokio::fs::remove_file(&temporary).await;
        return refuse(StatusCode::CONFLICT, "this delivery was already received");
    }

    let _writing = desk.writing.lock().await;
    // Moving, linking and rewriting the pages is file work: off the async threads.
    let placed = {
        let (desk, envelope, temporary) = (desk.clone(), envelope.clone(), temporary.clone());
        tokio::task::spawn_blocking(move || place(&desk, &envelope, &temporary))
            .await
            .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())))
    };
    let _ = std::fs::remove_file(&temporary);
    let (path, name) = match placed {
        Ok(placed) => placed,
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    eprintln!(
        "archie-desk: {}/{}/{} ({} bytes){}",
        envelope.project,
        envelope.stage,
        name,
        bytes,
        if envelope.note.is_empty() {
            String::new()
        } else {
            format!(" — {}", envelope.note)
        }
    );
    let opened = envelope.open && desk.rules.open(&path);
    (
        StatusCode::CREATED,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "path": format!("{}/stages/{}/{}", envelope.project, envelope.stage, name),
            "opened": opened,
        })
        .to_string(),
    )
        .into_response()
}

/// Streams the body into `path`, hashing it, and refuses more or fewer bytes than declared.
async fn receive(body: Body, path: &Path, declared: u64) -> Result<String, String> {
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut total = 0u64;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        total += chunk.len() as u64;
        if total > declared {
            return Err("more bytes than declared".into());
        }
        context.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    if total != declared {
        return Err(format!("{total} bytes arrived of {declared}"));
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(hex::encode(context.finish().as_ref()))
}

/// Moves a verified file into its stage, never over a different file, copies it to
/// `latest/`, and updates the project's record and pages. Returns where it went.
fn place(desk: &Desk, envelope: &Envelope, temporary: &Path) -> std::io::Result<(PathBuf, String)> {
    let project_dir = desk.project_dir(&envelope.project);
    let stage_dir = project_dir.join("stages").join(&envelope.stage);
    std::fs::create_dir_all(&stage_dir)?;
    let (stem, extension) = match envelope.file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (envelope.file.clone(), String::new()),
    };
    let mut name = envelope.file.clone();
    let mut n = 1;
    loop {
        let candidate = stage_dir.join(&name);
        if !candidate.exists() {
            break;
        }
        // The same bytes again: already here.
        if archie_bridges::delivery::digest_file(&candidate)?.0 == envelope.sha256 {
            break;
        }
        n += 1;
        name = format!("{stem}-{n}{extension}");
    }
    let target = stage_dir.join(&name);
    if !target.exists() && std::fs::rename(temporary, &target).is_err() {
        std::fs::copy(temporary, &target)?;
    }
    let latest = project_dir.join("latest");
    std::fs::create_dir_all(&latest)?;
    let latest_file = latest.join(&envelope.file);
    let staged = latest.join(format!(".{}.archie-new", envelope.file));
    let _ = std::fs::remove_file(&staged);
    // A copy, not a link: an application that saves over latest/ must not change the stage.
    std::fs::copy(&target, &staged)?;
    std::fs::rename(&staged, &latest_file)?;

    let mut project = desk.load(&envelope.project);
    project.record(
        &envelope.stage,
        &envelope.note,
        FileEntry {
            name: name.clone(),
            bytes: envelope.bytes,
            sha256: envelope.sha256.clone(),
        },
        now_ms(),
    );
    desk.save(&project)?;
    Ok((target, name))
}
