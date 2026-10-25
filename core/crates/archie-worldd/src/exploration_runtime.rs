//! Native Aleph-style capture execution, owned by worldd, independent of UI lifetime.
//! Upstream reference: TheLoki2020/aleph ad68fc4 (MIT, Belluxx), see third_party/aleph.
//! Provider limits are application admission controls, never OS packet shaping.
use crate::exploration::*;
use crate::geo_formats::{self, DecodeLimits, GeoAnchor, GeoBounds};
use archie_geofabric::exploration::{self, ExplorationDomain, ExplorationLayer, ExplorationPlan};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

const MAX_JOBS: usize = 32;
const MAX_ACTIVE: usize = 4;
const MAX_RESPONSE: u64 = 16 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;

pub fn now_ms() -> i64 {
    archie_types::Timestamp::now().as_millis()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Ready,
    WaitUntil(i64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLedger {
    requests_per_minute: u32,
    max_in_flight: u32,
    in_flight: u32,
    window_start_ms: Option<i64>,
    window_requests: u32,
    requests_sent: u64,
    bytes_received: u64,
    retry_at_ms: Option<i64>,
    remaining_requests: Option<u64>,
}
impl RateLedger {
    pub fn new(requests_per_minute: u32, max_in_flight: u32) -> Self {
        Self {
            requests_per_minute: requests_per_minute.max(1),
            max_in_flight: max_in_flight.max(1),
            in_flight: 0,
            window_start_ms: None,
            window_requests: 0,
            requests_sent: 0,
            bytes_received: 0,
            retry_at_ms: None,
            remaining_requests: None,
        }
    }
    pub fn recover_after_restart(&mut self) {
        self.in_flight = 0;
    }
    pub fn admit(&mut self, now: i64) -> Admission {
        if let Some(until) = self.retry_at_ms.filter(|t| *t > now) {
            return Admission::WaitUntil(until);
        }
        self.retry_at_ms = None;
        let start = *self.window_start_ms.get_or_insert(now);
        if now >= start.saturating_add(60_000) {
            self.window_start_ms = Some(now);
            self.window_requests = 0;
        }
        if self.window_requests >= self.requests_per_minute {
            return Admission::WaitUntil(self.window_start_ms.unwrap().saturating_add(60_000));
        }
        if self.in_flight >= self.max_in_flight {
            return Admission::WaitUntil(now.saturating_add(100));
        }
        self.window_requests += 1;
        self.requests_sent = self.requests_sent.saturating_add(1);
        self.in_flight += 1;
        Admission::Ready
    }
    pub fn finish(&mut self, bytes: u64, remaining: Option<u64>, retry: Option<i64>) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.bytes_received = self.bytes_received.saturating_add(bytes);
        if let Some(n) = remaining {
            self.remaining_requests = Some(n);
            if n == 0 && retry.is_none() {
                self.retry_at_ms = Some(
                    self.window_start_ms
                        .unwrap_or_else(now_ms)
                        .saturating_add(60_000)
                        .max(now_ms() + 1000),
                );
            }
        }
        if let Some(t) = retry {
            self.retry_at_ms = Some(self.retry_at_ms.map_or(t, |old| old.max(t)));
        }
    }
    pub fn snapshot(&self, id: &str) -> ProviderUsage {
        ProviderUsage {
            id: id.into(),
            state: if self.retry_at_ms.is_some_and(|t| t > now_ms()) {
                "waiting_rate_limit"
            } else if self.in_flight > 0 {
                "fetching"
            } else {
                "ready"
            }
            .into(),
            requests_sent: self.requests_sent,
            bytes_received: self.bytes_received,
            requests_per_minute: self.requests_per_minute,
            max_in_flight: self.max_in_flight,
            in_flight: self.in_flight,
            remaining_requests: self.remaining_requests,
            retry_at_ms: self.retry_at_ms,
            quota_source: if self.remaining_requests.is_some() {
                "provider"
            } else {
                "local"
            }
            .into(),
        }
    }
}

#[derive(Clone)]
pub struct ExplorationConfig {
    pub cache_dir: PathBuf,
    pub domain: ExplorationDomain,
}

#[derive(Clone)]
struct Endpoints {
    terrain: String,
    osm: String,
    geocoder: String,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            terrain: "https://elevation-tiles-prod.s3.amazonaws.com/geotiff".into(),
            osm: "https://overpass-api.de/api/interpreter".into(),
            geocoder: "https://photon.komoot.io/api/".into(),
        }
    }
}

struct State {
    jobs: Vec<ExplorationJob>,
    rates: BTreeMap<String, RateLedger>,
    workers: BTreeSet<String>,
}

// An execute timeout or explicit job cancellation must release concurrency, while
// retaining the already-recorded request and provider cooldown.
struct RequestPermit {
    state: Arc<Mutex<State>>,
    provider: String,
    armed: bool,
}
impl Drop for RequestPermit {
    fn drop(&mut self) {
        if self.armed {
            let state = self.state.clone();
            let provider = self.provider.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Some(rate) = state.lock().await.rates.get_mut(&provider) {
                        rate.finish(0, None, None);
                    }
                });
            }
        }
    }
}
#[derive(Clone)]
pub struct ExplorationService {
    config: ExplorationConfig,
    client: reqwest::Client,
    state: Arc<Mutex<State>>,
    endpoints: Endpoints,
    cache_locks: Arc<Mutex<BTreeMap<String, std::sync::Weak<Mutex<()>>>>>,
    cache_writer: Arc<Mutex<()>>,
}

fn domain_name(domain: ExplorationDomain) -> &'static str {
    match domain {
        ExplorationDomain::Personal => "personal",
        ExplorationDomain::Work => "work",
    }
}
fn default_rates() -> BTreeMap<String, RateLedger> {
    [
        ("aws_terrain_tiles".into(), RateLedger::new(120, 4)),
        ("openstreetmap".into(), RateLedger::new(6, 1)),
        ("photon".into(), RateLedger::new(30, 1)),
    ]
    .into()
}

pub fn private_directory(path: &Path) -> Result<(), String> {
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("refusing symlink directory".into());
    }
    std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension(format!("tmp-{:016x}", rand::random::<u64>()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}

impl ExplorationService {
    pub fn new(config: ExplorationConfig) -> Result<Self, String> {
        private_directory(&config.cache_dir)?;
        let root = config.cache_dir.join(domain_name(config.domain));
        private_directory(&root)?;
        private_directory(&root.join("cache"))?;
        let mut jobs = Vec::new();
        for item in std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .flatten()
            .take(MAX_JOBS + 16)
        {
            if item.file_name().to_string_lossy().starts_with("job-")
                && item.file_type().is_ok_and(|t| t.is_dir())
            {
                let p = item.path().join("manifest.json");
                if std::fs::metadata(&p).is_ok_and(|m| m.len() < 2 * 1024 * 1024) {
                    if let Ok(bytes) = std::fs::read(p) {
                        if let Ok(mut j) = serde_json::from_slice::<ExplorationJob>(&bytes) {
                            if j.domain != config.domain
                                || !valid_id(&j.id)
                                || exploration::plan(&j.request).is_err()
                            {
                                continue;
                            }
                            if matches!(
                                j.state,
                                JobState::Queued | JobState::Running | JobState::WaitingRateLimit
                            ) {
                                j.state = JobState::Paused;
                                j.error = Some("daemon restarted; explicit resume required".into());
                            }
                            jobs.push(j);
                            if jobs.len() >= MAX_JOBS {
                                break;
                            }
                        }
                    }
                }
            }
        }
        jobs.sort_by_key(|j| j.created_at_ms);
        let rates_path = root.join("usage.json");
        let mut rates = if std::fs::metadata(&rates_path).is_ok_and(|m| m.len() < 64 * 1024) {
            std::fs::read(rates_path)
                .ok()
                .and_then(|b| serde_json::from_slice::<BTreeMap<String, RateLedger>>(&b).ok())
                .unwrap_or_else(default_rates)
        } else {
            default_rates()
        };
        for rate in rates.values_mut() {
            rate.recover_after_restart();
        }
        for (id, rate) in default_rates() {
            rates.entry(id).or_insert(rate);
        }
        Ok(Self {
            config,
            client: reqwest::Client::builder()
                .user_agent("ArchieOS/0.1 (+https://github.com/PentaVM/ArchieOS-Nvidia)")
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| e.to_string())?,
            state: Arc::new(Mutex::new(State {
                jobs,
                rates,
                workers: BTreeSet::new(),
            })),
            endpoints: Endpoints::default(),
            cache_locks: Arc::new(Mutex::new(BTreeMap::new())),
            cache_writer: Arc::new(Mutex::new(())),
        })
    }
    fn root(&self) -> PathBuf {
        self.config.cache_dir.join(domain_name(self.config.domain))
    }
    fn save_job(&self, job: &ExplorationJob) -> Result<(), String> {
        let dir = self.root().join(format!("job-{}", job.id));
        private_directory(&dir)?;
        let bytes = serde_json::to_vec(job).map_err(|e| e.to_string())?;
        if bytes.len() >= 2 * 1024 * 1024 {
            return Err("capture manifest exceeds its 2MiB bound".into());
        }
        atomic_write(&dir.join("manifest.json"), &bytes)
    }
    fn save_rates(&self, state: &State) -> Result<(), String> {
        atomic_write(
            &self.root().join("usage.json"),
            &serde_json::to_vec(&state.rates).map_err(|e| e.to_string())?,
        )
    }
    pub async fn snapshot(&self) -> ExplorationSnapshot {
        let state = self.state.lock().await;
        let mut jobs = state.jobs.clone();
        let last = jobs.len().saturating_sub(1);
        for (index, job) in jobs.iter_mut().enumerate() {
            job.projection_truncated = !job.plan.tiles.is_empty()
                || job.artifacts.len() > 16
                || index != last && job.preview.is_some();
            job.plan.tiles.clear();
            job.artifacts.truncate(16);
            if index != last {
                job.preview = None;
            }
        }
        ExplorationSnapshot {
            schema_version: 1,
            source: "live".into(),
            generated_at_ms: now_ms(),
            domain: self.config.domain,
            jobs,
            providers: state.rates.iter().map(|(id, r)| r.snapshot(id)).collect(),
            error: None,
        }
    }
    fn check_domain(&self, domain: ExplorationDomain) -> Result<(), String> {
        if domain == self.config.domain {
            Ok(())
        } else {
            Err("domain differs from this world daemon".into())
        }
    }
    fn checked_plan(
        &self,
        request: &exploration::ExplorationRequest,
    ) -> Result<ExplorationPlan, String> {
        self.check_domain(request.domain)?;
        let mut plan = exploration::plan(request).map_err(|e| e.to_string())?;
        if request.layers.contains(&ExplorationLayer::Terrain) && request.zoom > 14 {
            return Err("AWS terrain is available at zoom 0..14; choose a supported detail".into());
        }
        if plan.bounds.west > plan.bounds.east {
            return Err("split antimeridian captures into two areas before downloading".into());
        }
        for layer in &request.layers {
            if !matches!(
                layer,
                ExplorationLayer::Terrain | ExplorationLayer::Buildings
            ) {
                return Err(format!("{layer:?}: source capture/storage not permitted by configured GeoFabric providers; imagery, Street View and Google meshes require a compatible licensed adapter"));
            }
            let source = match layer {
                ExplorationLayer::Terrain => archie_geofabric::GeoProviderId::AwsTerrainTiles,
                _ => archie_geofabric::GeoProviderId::OpenStreetMap,
            };
            let provenance = archie_geofabric::TileProvenance::from_provider(source, None);
            provenance.may_be_cached().map_err(|e| e.to_string())?;
            provenance
                .may_be_reasoned_about()
                .map_err(|e| e.to_string())?;
        }
        plan.estimated_requests = plan.tiles.len() as u64
            * u64::from(request.layers.contains(&ExplorationLayer::Terrain))
            + u64::from(request.layers.contains(&ExplorationLayer::Buildings));
        Ok(plan)
    }
    async fn command(&self, command: ExploreCommand) -> Result<Value, String> {
        match command {
            ExploreCommand::Plan { request } => Ok(json!({"plan":self.checked_plan(&request)?})),
            ExploreCommand::Status { domain } => {
                self.check_domain(domain)?;
                Ok(json!(self.snapshot().await))
            }
            ExploreCommand::Resolve { query, domain } => {
                self.check_domain(domain)?;
                self.resolve(&query).await
            }
            ExploreCommand::Explore { request } => {
                let plan = self.checked_plan(&request)?;
                let mut state = self.state.lock().await;
                if state.jobs.len() >= MAX_JOBS {
                    return Err("capture history limit reached; archive completed captures before starting more".into());
                }
                if state
                    .jobs
                    .iter()
                    .filter(|j| {
                        matches!(
                            j.state,
                            JobState::Queued | JobState::Running | JobState::WaitingRateLimit
                        )
                    })
                    .count()
                    >= MAX_ACTIVE
                {
                    return Err("four exploration jobs already active".into());
                }
                let total = plan.tiles.len() as u64
                    * u64::from(request.layers.contains(&ExplorationLayer::Terrain))
                    + u64::from(request.layers.contains(&ExplorationLayer::Buildings));
                let job = ExplorationJob {
                    id: format!("{:032x}", rand::random::<u128>()),
                    domain: request.domain,
                    state: JobState::Queued,
                    camera: plan.camera,
                    request,
                    tiles_total: total,
                    tiles_done: 0,
                    bytes_received: 0,
                    stored_bytes: 0,
                    cache_hits: 0,
                    error: None,
                    artifacts: vec![],
                    preview: None,
                    created_at_ms: now_ms(),
                    updated_at_ms: now_ms(),
                    plan,
                    projection_truncated: false,
                };
                self.save_job(&job)?;
                let id = job.id.clone();
                state.jobs.push(job.clone());
                drop(state);
                self.spawn(id);
                Ok(json!({"job":job}))
            }
            ExploreCommand::Pause { id, domain }
            | ExploreCommand::Resume { id, domain }
            | ExploreCommand::Cancel { id, domain } => {
                // Kept below as separate action classification before borrowing the job.
                let _ = (id, domain);
                Err("job control must use execute".into())
            }
        }
    }
    pub async fn execute(&self, command: ExploreCommand) -> Result<Value, String> {
        let (id, domain, desired) = match &command {
            ExploreCommand::Pause { id, domain } => (id.clone(), *domain, JobState::Paused),
            ExploreCommand::Resume { id, domain } => (id.clone(), *domain, JobState::Queued),
            ExploreCommand::Cancel { id, domain } => (id.clone(), *domain, JobState::Cancelled),
            _ => return self.command(command).await,
        };
        self.check_domain(domain)?;
        if !valid_id(&id) {
            return Err("invalid exploration id".into());
        }
        let mut state = self.state.lock().await;
        if desired == JobState::Queued && state.workers.contains(&id) {
            return Err("previous worker is stopping; retry resume shortly".into());
        }
        if desired == JobState::Queued
            && state
                .jobs
                .iter()
                .filter(|j| {
                    matches!(
                        j.state,
                        JobState::Queued | JobState::Running | JobState::WaitingRateLimit
                    )
                })
                .count()
                >= MAX_ACTIVE
        {
            return Err("four exploration jobs already active".into());
        }
        let job = state
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("unknown exploration id")?;
        if desired == JobState::Queued && !matches!(job.state, JobState::Paused | JobState::Failed)
        {
            return Err("only paused or failed captures can resume".into());
        }
        if desired == JobState::Paused
            && matches!(job.state, JobState::Complete | JobState::Cancelled)
        {
            return Err("capture already terminal".into());
        }
        job.state = desired;
        job.updated_at_ms = now_ms();
        job.error = None;
        self.save_job(job)?;
        let answer = json!({"job":job});
        drop(state);
        if desired == JobState::Queued {
            self.spawn(id);
        }
        Ok(answer)
    }
    fn spawn(&self, id: String) {
        let service = self.clone();
        tokio::spawn(async move {
            {
                let mut state = service.state.lock().await;
                if !state.workers.insert(id.clone()) {
                    return;
                }
            }
            if let Err(error) = service.run(&id).await {
                let mut state = service.state.lock().await;
                if let Some(job) = state.jobs.iter_mut().find(|j| j.id == id) {
                    if !matches!(job.state, JobState::Paused | JobState::Cancelled) {
                        job.state = JobState::Failed;
                        job.error = Some(error);
                        job.updated_at_ms = now_ms();
                        let _ = service.save_job(job);
                    }
                }
            }
            service.state.lock().await.workers.remove(&id);
        });
    }
    async fn active(&self, id: &str) -> Result<(), String> {
        let state = self.state.lock().await;
        match state.jobs.iter().find(|j| j.id == id).map(|j| j.state) {
            Some(JobState::Paused | JobState::Cancelled | JobState::Complete) | None => {
                Err("capture stopped by explicit command".into())
            }
            _ => Ok(()),
        }
    }
    async fn stopped(&self, id: Option<&str>) {
        let Some(id) = id else {
            std::future::pending::<()>().await;
            return;
        };
        loop {
            if self.active(id).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    async fn mark(&self, id: &str, next: JobState) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let j = state
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("unknown job")?;
        if matches!(j.state, JobState::Paused | JobState::Cancelled) {
            return Err("capture explicitly stopped".into());
        }
        if j.state == next {
            return Ok(());
        }
        j.state = next;
        j.updated_at_ms = now_ms();
        self.save_job(j)
    }
    async fn fetch(
        &self,
        id: Option<&str>,
        provider: &str,
        url: &str,
        budget: u64,
    ) -> Result<(Vec<u8>, bool), String> {
        let hash = format!("{:x}", Sha256::digest(url.as_bytes()));
        let key_lock = {
            let mut locks = self.cache_locks.lock().await;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&hash).and_then(std::sync::Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(hash.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        let _same_resource = tokio::select! {guard=key_lock.lock_owned()=>guard,_=self.stopped(id)=>return Err("capture explicitly stopped".into())};
        let cache = self.root().join("cache").join(hash);
        if let Ok(metadata) = std::fs::metadata(&cache) {
            if metadata.is_file()
                && metadata.len() <= MAX_RESPONSE
                && metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age < Duration::from_secs(7 * 86400))
            {
                if let Ok(bytes) = std::fs::read(&cache) {
                    return Ok((bytes, true));
                }
                // Another resource's bounded-cache eviction may race metadata.
                // An evicted object is a miss, not a failed capture.
            }
        }
        if budget == 0 {
            return Err("capture network byte budget exhausted".into());
        }
        for attempt in 0..4 {
            loop {
                if let Some(id) = id {
                    self.active(id).await?;
                }
                let decision = {
                    let mut state = self.state.lock().await;
                    let decision = state
                        .rates
                        .get_mut(provider)
                        .ok_or("unknown provider")?
                        .admit(now_ms());
                    if decision == Admission::Ready {
                        if let Err(error) = self.save_rates(&state) {
                            state.rates.get_mut(provider).unwrap().finish(0, None, None);
                            return Err(error);
                        }
                    }
                    decision
                };
                match decision {
                    Admission::Ready => break,
                    Admission::WaitUntil(until) => {
                        if let Some(id) = id {
                            self.mark(id, JobState::WaitingRateLimit).await?;
                        }
                        tokio::time::sleep(Duration::from_millis(
                            (until - now_ms()).clamp(1, 250) as u64
                        ))
                        .await;
                    }
                }
            }
            let mut permit = RequestPermit {
                state: self.state.clone(),
                provider: provider.into(),
                armed: true,
            };
            if let Some(id) = id {
                self.mark(id, JobState::Running).await?;
            }
            let outcome = async {
                let mut response=tokio::select! {
                    response=self.client.get(url).send()=>response.map_err(|_|"provider request failed".to_string())?,
                    _=self.stopped(id)=>return Err("capture explicitly stopped".into()),
                };
                let status = response.status();
                let remaining = ["ratelimit-remaining", "x-ratelimit-remaining"]
                    .iter()
                    .find_map(|h| {
                        response
                            .headers()
                            .get(*h)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                    });
                let retry = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| retry_after(v, now_ms()));
                if status.as_u16() == 429 || status.is_server_error() {
                    return Ok((
                        Vec::new(),
                        remaining,
                        Some(retry.unwrap_or_else(|| now_ms() + 1000 * (1i64 << attempt))),
                        true,
                    ));
                }
                if !status.is_success() {
                    return Err(format!("provider HTTP {}", status.as_u16()));
                }
                let limit = budget.min(MAX_RESPONSE);
                if response.content_length().is_some_and(|n| n > limit) {
                    return Err("provider response exceeds remaining byte budget".into());
                }
                let mut bytes = Vec::new();
                loop {
                    let chunk=tokio::select!{
                        chunk=response.chunk()=>chunk.map_err(|_|"provider stream failed".to_string())?,
                        _=self.stopped(id)=>return Err("capture explicitly stopped".into()),
                    };
                    let Some(chunk)=chunk else {break;};
                    if let Some(id) = id {
                        self.active(id).await?;
                    }
                    {
                        let mut state = self.state.lock().await;
                        state.rates.get_mut(provider).unwrap().bytes_received = state.rates
                            [provider]
                            .bytes_received
                            .saturating_add(chunk.len() as u64);
                        if let Some(id) = id {
                            let job = state
                                .jobs
                                .iter_mut()
                                .find(|j| j.id == id)
                                .ok_or("unknown job")?;
                            job.bytes_received =
                                job.bytes_received.saturating_add(chunk.len() as u64);
                            self.save_job(job)?;
                        }
                        self.save_rates(&state)?;
                    }
                    if bytes.len().saturating_add(chunk.len()) as u64 > limit {
                        return Err("capture byte budget exceeded".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok::<_, String>((bytes, remaining, retry, false))
            }
            .await;
            {
                let mut state = self.state.lock().await;
                let (remaining, retry) = outcome
                    .as_ref()
                    .map(|(_, r, t, _)| (*r, *t))
                    .unwrap_or((None, None));
                state
                    .rates
                    .get_mut(provider)
                    .unwrap()
                    .finish(0, remaining, retry);
                permit.armed = false;
                self.save_rates(&state)?;
            }
            match outcome {
                Ok((_, _, _, true)) if attempt < 3 => continue,
                Ok((_, _, _, true)) => {
                    return Err("provider retries exhausted; cooldown retained".into())
                }
                Ok((bytes, _, _, false)) => {
                    validate_payload(provider, &bytes)?;
                    let _writer = self.cache_writer.lock().await;
                    trim_cache(cache.parent().unwrap(), bytes.len() as u64, MAX_CACHE_BYTES)?;
                    atomic_write(&cache, &bytes)?;
                    return Ok((bytes, false));
                }
                Err(e) => return Err(e),
            }
        }
        Err("provider retries exhausted".into())
    }
    async fn resolve(&self, query: &str) -> Result<Value, String> {
        if query.trim().is_empty() || query.chars().count() > 256 {
            return Err("place query must contain 1..256 characters".into());
        }
        let mut url = reqwest::Url::parse(&self.endpoints.geocoder).map_err(|e| e.to_string())?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("limit", "5");
        let (bytes, cached) = self
            .fetch(None, "photon", url.as_str(), 1024 * 1024)
            .await?;
        let raw: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid geocoder response")?;
        let places=raw["features"].as_array().ok_or("invalid geocoder features")?.iter().take(5).filter_map(|f|{
            let c=f["geometry"]["coordinates"].as_array()?;let lon=c.first()?.as_f64()?;let lat=c.get(1)?.as_f64()?;
            if !lat.is_finite()|| !lon.is_finite()||lat.abs()>90.||lon.abs()>180.{return None;}
            Some(json!({"latitude":lat,"longitude":lon,"name":f["properties"]["name"].as_str().unwrap_or("").chars().take(200).collect::<String>(),"source":"OpenStreetMap / Photon"}))
        }).collect::<Vec<_>>();
        Ok(json!({"places":places,"cache_hit":cached}))
    }
    async fn run(&self, id: &str) -> Result<(), String> {
        self.mark(id, JobState::Running).await?;
        let job = self
            .state
            .lock()
            .await
            .jobs
            .iter()
            .find(|j| j.id == id)
            .cloned()
            .ok_or("unknown job")?;
        let bounds = GeoBounds {
            south: job.plan.bounds.south,
            west: job.plan.bounds.west,
            north: job.plan.bounds.north,
            east: job.plan.bounds.east,
        };
        let limits = DecodeLimits::default();
        let mut buildings = vec![];
        let mut terrain = geo_formats::TerrainPreview {
            width: 33,
            height: 33,
            elevations_m: vec![None; 33 * 33],
        };
        if job.request.layers.contains(&ExplorationLayer::Buildings) {
            if bounds.west > bounds.east {
                return Err(
                    "building query crossing antimeridian requires two separate captures".into(),
                );
            }
            let query = format!(
                "[out:json][timeout:25];way[building]({},{},{},{});out geom;",
                bounds.south, bounds.west, bounds.north, bounds.east
            );
            let mut url = reqwest::Url::parse(&self.endpoints.osm).map_err(|e| e.to_string())?;
            url.query_pairs_mut().append_pair("data", &query);
            let (bytes, cache) = self
                .source_bytes(id, "buildings.json", "openstreetmap", url.as_str())
                .await?;
            buildings = geo_formats::parse_overpass_buildings(&bytes, &limits)?;
            self.artifact(
                id,
                "buildings",
                "osm_json",
                "buildings.json",
                &bytes,
                "© OpenStreetMap contributors · ODbL",
                cache,
            )
            .await?;
            self.preview(id, &bounds, None, &buildings).await?;
        }
        if job.request.layers.contains(&ExplorationLayer::Terrain) {
            for tile in &job.plan.tiles {
                self.active(id).await?;
                let filename = format!("terrain-{}-{}-{}.tif", tile.zoom, tile.x, tile.y);
                let url = format!(
                    "{}/{}/{}/{}.tif",
                    self.endpoints.terrain, tile.zoom, tile.x, tile.y
                );
                let (bytes, cache) = self
                    .source_bytes(id, &filename, "aws_terrain_tiles", &url)
                    .await?;
                let preview = geo_formats::decode_terrain_tiff(&bytes, &limits)?;
                let b = tile_bounds(tile.zoom, tile.x, tile.y);
                self.artifact(
                    id,
                    "terrain",
                    "geotiff",
                    &filename,
                    &bytes,
                    "AWS Terrain Tiles · source attribution: tilezen/joerd",
                    cache,
                )
                .await?;
                merge_terrain(&mut terrain, &bounds, &preview, &b);
                self.preview(id, &bounds, Some(&terrain), &buildings)
                    .await?;
            }
        }
        let export_bounds = bounds;
        let dem = job
            .request
            .layers
            .contains(&ExplorationLayer::Terrain)
            .then_some(&terrain);
        let geojson = geo_formats::export_geojson(export_bounds, dem, &buildings)?;
        self.export(id, "capture.geojson", "geojson", &geojson)
            .await?;
        let glb = geo_formats::export_glb(
            GeoAnchor {
                lon: job.request.longitude,
                lat: job.request.latitude,
            },
            export_bounds,
            dem,
            &buildings,
            None,
        );
        match glb {
            Ok(bytes) => self.export(id, "capture.glb", "glb", &bytes).await?,
            Err(error) if error == "no triangulatable geometry for GLB" => {}
            Err(error) => return Err(error),
        }
        self.mark(id, JobState::Complete).await
    }
    async fn remaining(&self, id: &str) -> Result<u64, String> {
        let state = self.state.lock().await;
        let job = state
            .jobs
            .iter()
            .find(|j| j.id == id)
            .ok_or("unknown job")?;
        job.request
            .max_bytes
            .checked_sub(job.bytes_received)
            .ok_or("capture byte budget exhausted".into())
    }
    async fn source_bytes(
        &self,
        id: &str,
        name: &str,
        provider: &str,
        url: &str,
    ) -> Result<(Vec<u8>, bool), String> {
        let saved = {
            let state = self.state.lock().await;
            state
                .jobs
                .iter()
                .find(|j| j.id == id)
                .is_some_and(|j| j.artifacts.iter().any(|a| a.relative_path == name))
        };
        if saved {
            let path = self.root().join(format!("job-{id}")).join(name);
            if path
                .symlink_metadata()
                .is_ok_and(|m| m.is_file() && m.len() <= MAX_RESPONSE)
            {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                validate_payload(provider, &bytes)?;
                return Ok((bytes, true));
            }
        }
        self.fetch(Some(id), provider, url, self.remaining(id).await?)
            .await
    }
    async fn preview(
        &self,
        id: &str,
        bounds: &GeoBounds,
        terrain: Option<&geo_formats::TerrainPreview>,
        buildings: &[geo_formats::BuildingFeature],
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let job = state
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("unknown job")?;
        let mut vertices = 0usize;
        let shown = buildings
            .iter()
            .filter(|b| {
                if b.ring.len() > 512 || vertices + b.ring.len() > 4096 {
                    return false;
                }
                vertices += b.ring.len();
                true
            })
            .take(128)
            .collect::<Vec<_>>();
        job.preview = Some(
            json!({"bounds":bounds,"terrain":terrain,"buildings":shown,"omitted_buildings":buildings.len()-shown.len(),"row_projection":"web_mercator"}),
        );
        job.camera.elevation_m = terrain
            .and_then(|t| t.elevations_m.iter().flatten().copied().reduce(f32::max))
            .map(f64::from);
        self.save_job(job)
    }
    #[allow(clippy::too_many_arguments)]
    async fn artifact(
        &self,
        id: &str,
        layer: &str,
        kind: &str,
        name: &str,
        bytes: &[u8],
        source: &str,
        cached: bool,
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let job = state
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("unknown job")?;
        self.store_artifact_bytes(job, name, bytes)?;
        if !job.artifacts.iter().any(|a| a.relative_path == name) {
            job.tiles_done += 1;
            job.artifacts.push(Artifact {
                layer: layer.into(),
                kind: kind.into(),
                relative_path: name.into(),
                source: source.into(),
                retrieved_at_ms: now_ms(),
            });
        }
        if cached {
            job.cache_hits += 1;
        }
        job.updated_at_ms = now_ms();
        self.save_job(job)
    }
    async fn export(&self, id: &str, name: &str, kind: &str, bytes: &[u8]) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let job = state
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("unknown job")?;
        self.store_artifact_bytes(job, name, bytes)?;
        job.artifacts.retain(|a| a.relative_path != name);
        job.artifacts.push(Artifact {
            layer: "export".into(),
            kind: kind.into(),
            relative_path: name.into(),
            source: "ArchieOS normalized capture".into(),
            retrieved_at_ms: now_ms(),
        });
        self.save_job(job)
    }
    fn store_artifact_bytes(
        &self,
        job: &mut ExplorationJob,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let path = self.root().join(format!("job-{}", job.id)).join(name);
        let old = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file())
            .map_or(0, |m| m.len());
        let stored = job
            .stored_bytes
            .saturating_sub(old)
            .saturating_add(bytes.len() as u64);
        if stored > job.request.max_bytes {
            return Err(
                "capture storage byte budget exceeded (includes cached files and exports)".into(),
            );
        }
        atomic_write(&path, bytes)?;
        job.stored_bytes = stored;
        Ok(())
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn trim_cache(directory: &Path, incoming: u64, capacity: u64) -> Result<(), String> {
    if incoming > capacity {
        return Err("cache object exceeds cache capacity".into());
    }
    let mut files = std::fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let metadata = entry.path().symlink_metadata().ok()?;
            metadata
                .is_file()
                .then_some((metadata.modified().ok()?, entry.path(), metadata.len()))
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|(modified, path, _)| (*modified, path.clone()));
    let mut size = files.iter().map(|(_, _, size)| size).sum::<u64>();
    let mut count = files.len();
    for (_, path, bytes) in files {
        if size.saturating_add(incoming) <= capacity && count < 2048 {
            break;
        }
        // Only generated SHA256 filenames in this task-owned cache, no recursion.
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
        size = size.saturating_sub(bytes);
        count -= 1;
    }
    Ok(())
}
pub fn retry_after(value: &str, now: i64) -> Option<i64> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(now.saturating_add(i64::try_from(seconds).ok()?.saturating_mul(1000)));
    }
    let date = httpdate::parse_http_date(value)
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(i64::try_from(date.as_millis()).ok()?.max(now))
}
fn tile_bounds(z: u8, x: u32, y: u32) -> GeoBounds {
    let n = 2f64.powi(i32::from(z));
    let lat = |y: f64| ((std::f64::consts::PI * (1. - 2. * y / n)).sinh().atan()).to_degrees();
    GeoBounds {
        west: f64::from(x) / n * 360. - 180.,
        east: (f64::from(x) + 1.) / n * 360. - 180.,
        north: lat(f64::from(y)),
        south: lat(f64::from(y) + 1.),
    }
}

fn merge_terrain(
    target: &mut geo_formats::TerrainPreview,
    bounds: &GeoBounds,
    source: &geo_formats::TerrainPreview,
    tile: &GeoBounds,
) {
    let span = if bounds.east >= bounds.west {
        bounds.east - bounds.west
    } else {
        bounds.east + 360. - bounds.west
    };
    let merc = |lat: f64| {
        (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.)
            .tan()
            .ln()
    };
    for y in 0..target.height {
        let normalized = merc(bounds.north)
            - (merc(bounds.north) - merc(bounds.south)) * y as f64 / (target.height - 1) as f64;
        let lat = (2. * normalized.exp().atan() - std::f64::consts::FRAC_PI_2)
            .to_degrees()
            .clamp(bounds.south, bounds.north);
        if lat < tile.south || lat > tile.north {
            continue;
        }
        let sy = ((merc(tile.north) - merc(lat)) / (merc(tile.north) - merc(tile.south))
            * (source.height - 1) as f64)
            .round()
            .clamp(0., (source.height - 1) as f64) as usize;
        for x in 0..target.width {
            let mut lon = (bounds.west + span * x as f64 / (target.width - 1) as f64 + 180.)
                .rem_euclid(360.)
                - 180.;
            if lon == -180. && tile.east == 180. {
                lon = 180.;
            }
            if lon < tile.west || lon > tile.east {
                continue;
            }
            let sx = ((lon - tile.west) / (tile.east - tile.west) * (source.width - 1) as f64)
                .round()
                .clamp(0., (source.width - 1) as f64) as usize;
            target.elevations_m[y * target.width + x] = source.elevations_m[sy * source.width + sx];
        }
    }
}

fn validate_payload(provider: &str, bytes: &[u8]) -> Result<(), String> {
    match provider {
        "aws_terrain_tiles" => {
            geo_formats::decode_terrain_tiff(
                bytes,
                &DecodeLimits {
                    max_preview_side: 33,
                    ..DecodeLimits::default()
                },
            )?;
        }
        "openstreetmap" => {
            geo_formats::parse_overpass_buildings(bytes, &DecodeLimits::default())?;
        }
        "photon" => {
            let value: Value =
                serde_json::from_slice(bytes).map_err(|_| "invalid geocoder JSON")?;
            if value["features"].as_array().is_none_or(|a| a.len() > 50) {
                return Err("invalid geocoder features".into());
            }
        }
        _ => return Err("unknown provider".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn service() -> ExplorationService {
        ExplorationService::new(ExplorationConfig {
            cache_dir: std::env::temp_dir()
                .join(format!("archie-geo-test-{:032x}", rand::random::<u128>())),
            domain: ExplorationDomain::Personal,
        })
        .unwrap()
    }
    async fn completed(service: &ExplorationService, id: &str) -> ExplorationJob {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let job = service
                    .snapshot()
                    .await
                    .jobs
                    .into_iter()
                    .find(|j| j.id == id)
                    .unwrap();
                if matches!(
                    job.state,
                    JobState::Complete | JobState::Failed | JobState::Cancelled
                ) {
                    return job;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    const BUILDING: &str = r#"{"elements":[{"type":"way","id":1,"tags":{"building":"yes","height":"12"},"geometry":[{"lon":0.0,"lat":0.0},{"lon":0.001,"lat":0.0},{"lon":0.001,"lat":0.001},{"lon":0.0,"lat":0.0}]}]}"#;
    #[tokio::test]
    async fn native_http_job_exports_real_geometry_while_views_are_read_independently() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut c, _) = listener.accept().await.unwrap();
            let mut b = [0; 4096];
            let _ = c.read(&mut b).await;
            c.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    BUILDING.len(),
                    BUILDING
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        });
        let mut service = service();
        service.endpoints.osm = format!("http://{address}/osm");
        let request = exploration::ExplorationRequest {
            layers: vec![ExplorationLayer::Buildings],
            ..Default::default()
        };
        let response = service
            .execute(ExploreCommand::Explore {
                request: request.clone(),
            })
            .await
            .unwrap();
        let id = response["job"]["id"].as_str().unwrap();
        for _ in 0..20 {
            let _ = service.snapshot().await;
            tokio::task::yield_now().await;
        }
        let job = completed(&service, id).await;
        assert_eq!(job.state, JobState::Complete, "{:?}", job.error);
        assert_eq!(job.tiles_done, 1);
        assert_eq!(
            job.preview.as_ref().unwrap()["buildings"][0]["height_m"],
            12.0
        );
        let glb =
            std::fs::read(service.root().join(format!("job-{id}")).join("capture.glb")).unwrap();
        assert_eq!(&glb[..4], b"glTF");
        server.await.unwrap();
        let again = service
            .execute(ExploreCommand::Explore { request })
            .await
            .unwrap();
        let second = completed(&service, again["job"]["id"].as_str().unwrap()).await;
        assert_eq!(second.state, JobState::Complete);
        assert_eq!(second.cache_hits, 1);
        assert_eq!(
            service
                .snapshot()
                .await
                .providers
                .iter()
                .find(|p| p.id == "openstreetmap")
                .unwrap()
                .requests_sent,
            1
        );
    }
    #[tokio::test]
    async fn only_explicit_cancel_stops_a_stalled_download_and_releases_admission() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (sent, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut c, _) = listener.accept().await.unwrap();
            let mut b = [0; 4096];
            let _ = c.read(&mut b).await;
            c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999\r\n\r\n")
                .await
                .unwrap();
            let _ = sent.send(());
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let mut service = service();
        service.endpoints.osm = format!("http://{addr}/osm");
        let request = exploration::ExplorationRequest {
            layers: vec![ExplorationLayer::Buildings],
            ..Default::default()
        };
        let accepted = service
            .execute(ExploreCommand::Explore { request })
            .await
            .unwrap();
        let id = accepted["job"]["id"].as_str().unwrap().to_string();
        ready.await.unwrap();
        service
            .execute(ExploreCommand::Cancel {
                id: id.clone(),
                domain: ExplorationDomain::Personal,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if service.state.lock().await.workers.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(completed(&service, &id).await.state, JobState::Cancelled);
        assert_eq!(
            service
                .snapshot()
                .await
                .providers
                .iter()
                .find(|p| p.id == "openstreetmap")
                .unwrap()
                .in_flight,
            0
        );
        server.abort();
    }
    #[test]
    fn mosaic_covers_all_tiles_preserving_nodata() {
        let bounds = GeoBounds {
            west: 0.,
            south: 0.,
            east: 2.,
            north: 1.,
        };
        let mut target = geo_formats::TerrainPreview {
            width: 3,
            height: 2,
            elevations_m: vec![None; 6],
        };
        let source = geo_formats::TerrainPreview {
            width: 2,
            height: 2,
            elevations_m: vec![Some(1.); 4],
        };
        merge_terrain(
            &mut target,
            &bounds,
            &source,
            &GeoBounds {
                west: 0.,
                south: 0.,
                east: 1.,
                north: 1.,
            },
        );
        let source = geo_formats::TerrainPreview {
            width: 2,
            height: 2,
            elevations_m: vec![Some(2.), None, Some(2.), Some(2.)],
        };
        merge_terrain(
            &mut target,
            &bounds,
            &source,
            &GeoBounds {
                west: 1.,
                south: 0.,
                east: 2.,
                north: 1.,
            },
        );
        assert_eq!(
            target.elevations_m,
            vec![Some(1.), Some(2.), None, Some(1.), Some(2.), Some(2.)]
        );
    }
    #[test]
    fn http_retry_dates_and_disabled_capture_sources_are_explicit() {
        assert_eq!(retry_after("2", 1000), Some(3000));
        assert_eq!(retry_after("Thu, 01 Jan 1970 00:01:00 GMT", 0), Some(60000));
        assert!(retry_after("bad", 0).is_none());
        let service = service();
        let mut request = exploration::ExplorationRequest {
            layers: vec![ExplorationLayer::Mesh],
            ..Default::default()
        };
        assert!(service.checked_plan(&request).is_err());
        request.layers = vec![ExplorationLayer::Terrain];
        request.zoom = 15;
        assert!(service.checked_plan(&request).is_err());
    }

    fn seed(_service: &ExplorationService) -> ExplorationJob {
        let request = exploration::ExplorationRequest::default();
        let plan = exploration::plan(&request).unwrap();
        ExplorationJob {
            id: "1".repeat(32),
            domain: request.domain,
            state: JobState::Paused,
            request,
            camera: plan.camera,
            tiles_total: plan.tiles.len() as u64,
            tiles_done: 0,
            bytes_received: 0,
            stored_bytes: 0,
            cache_hits: 0,
            error: None,
            artifacts: vec![],
            preview: None,
            created_at_ms: now_ms(),
            updated_at_ms: now_ms(),
            plan,
            projection_truncated: false,
        }
    }
    #[tokio::test]
    async fn history_projection_and_manifest_remain_bounded_without_silently_hiding_current_counts()
    {
        let service = service();
        let mut job = seed(&service);
        service.save_job(&job).unwrap();
        service.state.lock().await.jobs.push(job.clone());
        let ring = (0..512)
            .map(|n| {
                let a = n as f64 / 511. * std::f64::consts::TAU;
                [a.cos() * 0.001, a.sin() * 0.001]
            })
            .collect::<Vec<_>>();
        let buildings = (0..128)
            .map(|n| geo_formats::BuildingFeature {
                id: n.to_string(),
                ring: ring.clone(),
                height_m: None,
            })
            .collect::<Vec<_>>();
        let bounds = GeoBounds {
            south: -0.01,
            west: -0.01,
            north: 0.01,
            east: 0.01,
        };
        service
            .preview(&job.id, &bounds, None, &buildings)
            .await
            .unwrap();
        job = service.state.lock().await.jobs[0].clone();
        assert!(
            job.preview.as_ref().unwrap()["omitted_buildings"]
                .as_u64()
                .unwrap()
                > 0
        );
        {
            let mut state = service.state.lock().await;
            for index in 2..=32 {
                let mut other = job.clone();
                other.id = format!("{index:032x}");
                state.jobs.push(other);
            }
        }
        let view = service.snapshot().await;
        assert_eq!(view.jobs.len(), 32);
        assert_eq!(view.jobs.iter().filter(|j| j.preview.is_some()).count(), 1);
        assert!(serde_json::to_vec(&view).unwrap().len() < 1024 * 1024);
        assert!(
            std::fs::metadata(
                service
                    .root()
                    .join(format!("job-{}", job.id))
                    .join("manifest.json")
            )
            .unwrap()
            .len()
                < 2 * 1024 * 1024
        );
    }
    #[tokio::test]
    async fn exhausted_network_budget_can_resume_from_its_preserved_source_artifact() {
        let service = service();
        let mut job = seed(&service);
        job.request.layers = vec![ExplorationLayer::Buildings];
        job.plan = exploration::plan(&job.request).unwrap();
        job.bytes_received = job.request.max_bytes;
        job.tiles_total = 1;
        service.save_job(&job).unwrap();
        service.state.lock().await.jobs.push(job.clone());
        service
            .artifact(
                &job.id,
                "buildings",
                "osm_json",
                "buildings.json",
                BUILDING.as_bytes(),
                "OSM",
                false,
            )
            .await
            .unwrap();
        let mut restarted = ExplorationService::new(service.config.clone()).unwrap();
        restarted.endpoints.osm = "http://127.0.0.1:9/should-not-fetch".into();
        restarted
            .execute(ExploreCommand::Resume {
                id: job.id.clone(),
                domain: ExplorationDomain::Personal,
            })
            .await
            .unwrap();
        let final_job = completed(&restarted, &job.id).await;
        assert_eq!(final_job.state, JobState::Complete, "{:?}", final_job.error);
        assert_eq!(
            restarted
                .snapshot()
                .await
                .providers
                .iter()
                .find(|p| p.id == "openstreetmap")
                .unwrap()
                .requests_sent,
            0
        );
    }
    #[tokio::test]
    async fn empty_building_coverage_completes_without_a_fabricated_glb() {
        let service = service();
        let mut job = seed(&service);
        job.request.layers = vec![ExplorationLayer::Buildings];
        job.plan = exploration::plan(&job.request).unwrap();
        job.tiles_total = 1;
        service.save_job(&job).unwrap();
        service.state.lock().await.jobs.push(job.clone());
        service
            .artifact(
                &job.id,
                "buildings",
                "osm_json",
                "buildings.json",
                br#"{"elements":[]}"#,
                "OSM",
                false,
            )
            .await
            .unwrap();
        service
            .execute(ExploreCommand::Resume {
                id: job.id.clone(),
                domain: ExplorationDomain::Personal,
            })
            .await
            .unwrap();
        let final_job = completed(&service, &job.id).await;
        assert_eq!(final_job.state, JobState::Complete, "{:?}", final_job.error);
        assert!(final_job.artifacts.iter().any(|a| a.kind == "geojson"));
        assert!(!final_job.artifacts.iter().any(|a| a.kind == "glb"));
    }
    #[tokio::test]
    async fn failed_quota_persistence_does_not_leave_an_in_flight_slot_reserved() {
        let service = service();
        std::fs::create_dir(service.root().join("usage.json")).unwrap();
        assert!(service
            .fetch(None, "photon", "http://127.0.0.1:9/never-sent", 1000)
            .await
            .is_err());
        assert_eq!(
            service
                .snapshot()
                .await
                .providers
                .iter()
                .find(|p| p.id == "photon")
                .unwrap()
                .in_flight,
            0
        );
    }
    #[test]
    fn high_latitude_mosaic_uses_the_same_mercator_rows_as_exports() {
        let bounds = GeoBounds {
            west: 0.,
            east: 1.,
            south: 70.,
            north: 85.,
        };
        let source = geo_formats::TerrainPreview {
            width: 2,
            height: 9,
            elevations_m: (0..9).flat_map(|v| [Some(v as f32); 2]).collect(),
        };
        let mut target = geo_formats::TerrainPreview {
            width: 2,
            height: 5,
            elevations_m: vec![None; 10],
        };
        merge_terrain(&mut target, &bounds, &source, &bounds);
        assert_eq!(
            target.elevations_m,
            (0..5)
                .flat_map(|v| [Some((v * 2) as f32); 2])
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn cache_capacity_evicts_only_its_generated_objects() {
        let service = service();
        let directory = service.root().join("cache");
        std::fs::write(directory.join("a".repeat(64)), [1u8; 600]).unwrap();
        std::fs::write(directory.join("b".repeat(64)), [2u8; 600]).unwrap();
        std::fs::write(directory.join("owner-note"), b"keep").unwrap();
        trim_cache(&directory, 500, 1000).unwrap();
        assert!(directory.join("owner-note").exists());
        assert!(!directory.join("a".repeat(64)).exists());
        assert!(!directory.join("b".repeat(64)).exists());
    }
}
