//! Durable PostgreSQL state. No embedded or ephemeral backend fallback.
use crate::query::{sql_params, QueryExt};
use archie_memory::CorrectionTrajectory;
use archie_observability::{AuditError, AuditEvent, AuditSink};
use archie_postgres::{PostgresStore, StoreError};
use archie_types::{
    Capability, CapabilitySet, DeviceAdapterManifest, DeviceAdapterOrigin, NodeDescriptor, NodeId,
    Task, TaskGraph, TaskId, TaskStatus, Timestamp, TraceId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum StateError {
    #[error("{0}")]
    Database(#[from] StoreError),
    #[error("invalid serialized state")]
    Serialization(#[from] serde_json::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid stored data: {0}")]
    Corrupt(String),
    #[error("invalid device record: {0}")]
    InvalidDevice(String),
    #[error("invalid Cortex correction trajectory: {0}")]
    InvalidCortexTrajectory(String),
}
impl From<postgres::Error> for StateError {
    fn from(e: postgres::Error) -> Self {
        Self::Database(StoreError::from(e))
    }
}
pub type Result<T> = std::result::Result<T, StateError>;
pub const SCHEMA_VERSION: i64 = 1;
/// Read from the actual connection/server, not an independent client's defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Durability {
    pub synchronous: String,
    pub fsync: bool,
    pub full_page_writes: bool,
    pub data_checksums: bool,
    pub foreign_keys: bool,
    pub logged_tables: bool,
}
impl Durability {
    /// Required settings; this does not certify the host's storage hardware.
    pub fn commits_are_durable(&self) -> bool {
        self.synchronous == "on" && self.fsync && self.full_page_writes && self.logged_tables
    }
}
/// What changed on this node between two boots.
///
/// Answers the spec question "what appeared / what disappeared since last boot" —
/// which in practice is how a dead GPU or an unplugged sensor gets noticed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityDelta {
    pub appeared: Vec<Capability>,
    pub disappeared: Vec<Capability>,
    /// False on the very first boot, where there is nothing to compare against.
    pub had_previous_boot: bool,
}

impl CapabilityDelta {
    pub fn is_unchanged(&self) -> bool {
        self.appeared.is_empty() && self.disappeared.is_empty()
    }
}

/// How much a peer is trusted. Held by the *receiver*, never self-asserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustState {
    /// Seen, key not pinned. May exchange manifests; may not be scheduled work.
    Unverified,
    /// Key pinned on first contact and unchanged since.
    Pinned,
    /// Explicitly approved by the operator. Eligible for sensitive workloads.
    Trusted,
    /// Explicitly rejected. Ignored entirely.
    Revoked,
}

impl TrustState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Pinned => "pinned",
            Self::Trusted => "trusted",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "unverified" => Self::Unverified,
            "pinned" => Self::Pinned,
            "trusted" => Self::Trusted,
            "revoked" => Self::Revoked,
            _ => return None,
        })
    }

    /// Whether work may be scheduled onto a peer in this state.
    pub fn schedulable(&self) -> bool {
        matches!(self, Self::Pinned | Self::Trusted)
    }

    /// Whether data above `Internal` may be sent to a peer in this state.
    pub fn may_receive_sensitive_data(&self) -> bool {
        matches!(self, Self::Trusted)
    }
}

/// A peer as this node knows it.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerRecord {
    pub node_id: NodeId,
    /// Hex Ed25519 public key, pinned on first contact.
    pub public_key: String,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    pub trust: TrustState,
    /// Last descriptor received. `None` if we have only seen an announcement.
    pub descriptor: Option<NodeDescriptor>,
}

/// The fixed vocabulary of device families the local inventory can recognise.
///
/// The inventory is deliberately smaller than a general network scanner. An adapter is
/// added only after its pairing and recovery behaviour are specified; an arbitrary
/// network service cannot name itself into an authority-bearing record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceClass {
    Television,
    Router,
    Camera,
    Display,
    Mobile,
    Host,
    Printer,
    HomeAssistant,
}

impl DeviceClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Television => "television",
            Self::Router => "router",
            Self::Camera => "camera",
            Self::Display => "display",
            Self::Mobile => "mobile",
            Self::Host => "host",
            Self::Printer => "printer",
            Self::HomeAssistant => "home_assistant",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "television" => Self::Television,
            "router" => Self::Router,
            "camera" => Self::Camera,
            "display" => Self::Display,
            "mobile" => Self::Mobile,
            "host" => Self::Host,
            "printer" => Self::Printer,
            "home_assistant" => Self::HomeAssistant,
            _ => return None,
        })
    }
}

/// Local operator-controlled pairing state for a device public key.
///
/// A proposal only pins the presented key. It becomes usable only after an explicit
/// local pairing confirmation. Revocation is sticky: a later sighting with the same key
/// cannot silently restore device authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePairingState {
    Proposed,
    Paired,
    Revoked,
}

impl DevicePairingState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Paired => "paired",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "proposed" => Self::Proposed,
            "paired" => Self::Paired,
            "revoked" => Self::Revoked,
            _ => return None,
        })
    }

    /// Only a locally confirmed record may receive a future typed operation. This does
    /// not grant authority itself; policy and a per-operation lease remain mandatory.
    pub fn may_receive_commands(&self) -> bool {
        matches!(self, Self::Paired)
    }
}

/// A durable device key-pinning and pairing record.
///
/// This is inventory only: it has no endpoint, credential, protocol selection, or
/// adapter capability. That prevents storing a record from accidentally enabling a
/// network path to a television, router, camera, or display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRecord {
    pub device_reference: String,
    /// Lower-case hex encoding of an Ed25519 public key, pinned when proposed.
    pub public_key: String,
    pub class: DeviceClass,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    pub pairing: DevicePairingState,
}

/// The durable lifecycle of a prepared physical operation.
///
/// This is intentionally shorter than an execution lifecycle. The state store can
/// preserve what an operator is reviewing or cancelling, but it never records a live
/// permission to actuate after a restart. A future dispatcher must obtain and validate
/// a fresh lease at the physical boundary every time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceOperationState {
    Prepared,
    Cancelled,
}

impl DeviceOperationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "prepared" => Self::Prepared,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// An immutable operation digest associated with one paired device.
///
/// The payload itself stays with the future typed adapter; durable control/audit state
/// retains only this digest, the trace and the allocated operation id. That makes a
/// later approval bind to the reviewed operation without copying voice, screen or
/// configuration content into the general state database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceOperationRecord {
    pub operation_id: u64,
    pub device_reference: String,
    pub trace_id: TraceId,
    pub operation_sha256: String,
    pub prepared_at: Timestamp,
    pub state: DeviceOperationState,
}

/// An auditable, non-executable native-adapter proposal generated from fixed types.
///
/// This is deliberately proposal history, not an installation or approval record. The
/// stored JSON cannot include an endpoint, credential, payload or executable path by
/// construction of [`DeviceAdapterManifest`], and its digest detects a corrupt or
/// mismatched database row on read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAdapterProposalRecord {
    pub proposal_id: u64,
    pub manifest: DeviceAdapterManifest,
    pub manifest_sha256: String,
    pub proposed_at: Timestamp,
}

/// The durable, hash-verified record that joins one authoritative graph correction to
/// one asynchronous training trajectory. It contains only opaque vault/provenance
/// references and digests, never message or voice content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CortexCorrectionTrajectoryRecord {
    pub trajectory: CorrectionTrajectory,
    pub payload_sha256: String,
}

/// Idempotent result of attempting to persist a correction trajectory. Replaying the
/// exact same graph-writer receipt after a crash is safe; reusing an id for different
/// data is refused before it can contaminate the training set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CortexCorrectionTrajectoryWrite {
    Recorded(CortexCorrectionTrajectoryRecord),
    AlreadyRecorded(CortexCorrectionTrajectoryRecord),
}

pub struct StateStore {
    pub(crate) cognitive: PostgresStore,
    cleanup_schema: Option<String>,
}
impl std::fmt::Debug for StateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StateStore(PostgreSQL)")
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseConfig {
    connection: String,
    schema: String,
}
impl StateStore {
    /// Uses Core's durable connection, without a memory fallback.
    pub fn sentry_journal(
        &self,
    ) -> std::result::Result<
        archie_sentry::journal::PostgresJournal<'_>,
        archie_sentry::journal::JournalError,
    > {
        archie_sentry::journal::PostgresJournal::open(&self.cognitive)
    }
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        #[cfg(feature = "test-support")]
        if !path.exists() {
            if let Ok(url) = std::env::var("ARCHIE_TEST_DATABASE_URL") {
                if !path.starts_with(std::env::temp_dir()) {
                    return Err(StateError::Corrupt(
                        "test configuration must stay in temporary directory".into(),
                    ));
                }
                let schema = format!(
                    "archie_test_{}",
                    &sha256_hex(path.to_string_lossy().as_bytes())[..24]
                );
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut options = std::fs::OpenOptions::new();
                options.create_new(true).write(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(path)?;
                use std::io::Write;
                file.write_all(
                    serde_json::to_string(&serde_json::json!({"connection":url,"schema":schema}))?
                        .as_bytes(),
                )?;
                return Self::connect(&url, &schema);
            }
        }
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() {
            return Err(StateError::Corrupt(
                "database configuration is not a regular file".into(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o027 != 0 {
                return Err(StateError::Corrupt(
                    "database configuration must be private and not group-writable".into(),
                ));
            }
        }
        if metadata.len() > 16384 {
            return Err(StateError::Corrupt(
                "database configuration exceeds bound".into(),
            ));
        }
        let config: DatabaseConfig = serde_json::from_slice(&std::fs::read(path)?)?;
        Self::connect(&config.connection, &config.schema)
    }
    pub fn connect(connection: &str, schema: &str) -> Result<Self> {
        let config: postgres::Config = connection
            .parse()
            .map_err(|_| StateError::Corrupt("invalid PostgreSQL configuration".into()))?;
        let store = Self {
            cognitive: PostgresStore::open_local(config, schema)?,
            cleanup_schema: None,
        };
        store.migrate()?;
        let durability = store.durability()?;
        if !durability.commits_are_durable() || !durability.foreign_keys {
            return Err(StateError::Corrupt(
                "unsafe PostgreSQL durability configuration".into(),
            ));
        }
        Ok(store)
    }
    pub fn durability(&self) -> Result<Durability> {
        self.with_connection(|tx| {
            let row=tx.query_one("SELECT current_setting('synchronous_commit'),current_setting('fsync')::boolean,current_setting('full_page_writes')::boolean,current_setting('data_checksums')::boolean,current_setting('session_replication_role')='origin',NOT EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=current_schema() AND c.relkind IN ('r','p') AND c.relpersistence<>'p')",&[])?;
            Ok(Durability { synchronous:row.try_get(0)?,fsync:row.try_get(1)?,full_page_writes:row.try_get(2)?,data_checksums:row.try_get(3)?,foreign_keys:row.try_get(4)?,logged_tables:row.try_get(5)? })
        })
    }
    #[cfg(feature = "test-support")]
    pub fn open_test() -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let url = std::env::var("ARCHIE_TEST_DATABASE_URL")
            .map_err(|_| StateError::Corrupt("explicit PostgreSQL test fixture required".into()))?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| StateError::Corrupt("invalid test fixture time".into()))?
            .as_nanos();
        let schema = format!(
            "archie_fixture_{}_{}_{}",
            std::process::id(),
            nonce,
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let mut store = Self::connect(&url, &schema)?;
        store.cleanup_schema = Some(schema);
        Ok(store)
    }
    pub(crate) fn with_connection<T: Send>(
        &self,
        f: impl FnOnce(&mut postgres::Transaction<'_>) -> Result<T> + Send,
    ) -> Result<T> {
        self.cognitive.with_transaction(f)
    }
    #[cfg(feature = "test-support")]
    pub fn test_sql(&self, sql: &str) -> Result<()> {
        self.with_connection(|tx| {
            tx.batch_execute(sql)?;
            Ok(())
        })
    }
    fn migrate(&self) -> Result<()> {
        self.with_connection(|tx| {
            for (name,version) in [("timescaledb","2.29.2"),("vector","0.8.6")] {
                let row=tx.query_opt("SELECT extversion FROM pg_extension WHERE extname=$1",&[&name])?.ok_or_else(|| StateError::Corrupt("required database extension unavailable".into()))?;
                if row.try_get::<_,String>(0)?!=version { return Err(StateError::Corrupt("unreviewed database extension version".into())); }
            }
            tx.batch_execute("CREATE TABLE IF NOT EXISTS control_schema_migrations(version BIGINT PRIMARY KEY,hash TEXT NOT NULL)")?;
            let versions=tx.query("SELECT version,hash FROM control_schema_migrations ORDER BY version LIMIT 2",&[])?;
            let schema=include_str!("schema.sql"); let hash=sha256_hex(schema.replace("\r\n","\n").as_bytes());
            if versions.is_empty() { tx.batch_execute(schema)?;tx.execute("INSERT INTO control_schema_migrations VALUES (1,$1)",&[&hash])?; }
            else if versions.len()!=1 || versions[0].try_get::<_,i64>(0)?!=SCHEMA_VERSION || versions[0].try_get::<_,String>(1)?!=hash { return Err(StateError::Corrupt("unsupported control schema".into())); }
            Ok(())
        })
    }
    pub fn schema_version(&self) -> Result<i64> {
        self.with_connection(|tx| {
            Ok(tx
                .query_one("SELECT version FROM control_schema_migrations", &[])?
                .try_get(0)?)
        })
    }
    // ---------------------------------------------------------------- boots

    /// Record this boot and report what changed since the previous one.
    pub fn record_boot(&self, descriptor: &NodeDescriptor) -> Result<CapabilityDelta> {
        let capabilities_json = serde_json::to_string(&descriptor.capabilities)?;
        let descriptor_json = serde_json::to_string(descriptor)?;

        self.with_connection(|conn| {
            let previous: Option<String> = conn.select_optional(
                "SELECT capabilities FROM boots WHERE node_id = $1
                     ORDER BY id DESC LIMIT 1",
                sql_params![descriptor.node_id.to_string()],
                |row| row.try_get(0),
            )?;

            conn.execute(
                "INSERT INTO boots (node_id, started_at, archieos_version, capabilities, descriptor)
                 VALUES ($1, $2, $3, $4, $5)",
                sql_params![
                    descriptor.node_id.to_string(),
                    Timestamp::now().as_millis(),
                    descriptor.archieos_version,
                    capabilities_json,
                    descriptor_json,
                ],
            )?;

            let Some(previous) = previous else {
                return Ok(CapabilityDelta {
                    had_previous_boot: false,
                    ..Default::default()
                });
            };

            let before: CapabilitySet = serde_json::from_str(&previous)?;
            Ok(CapabilityDelta {
                appeared: descriptor.capabilities.difference(&before),
                disappeared: before.difference(&descriptor.capabilities),
                had_previous_boot: true,
            })
        })
    }

    pub fn boot_count(&self, node_id: &NodeId) -> Result<u64> {
        self.with_connection(|conn| {
            let count: i64 = conn.select_one(
                "SELECT COUNT(*) FROM boots WHERE node_id = $1",
                sql_params![node_id.to_string()],
                |row| row.try_get(0),
            )?;
            Ok(count as u64)
        })
    }

    // ---------------------------------------------------------------- tasks

    /// Insert or replace a task. Validated first: an invalid task must never reach disk.
    pub fn put_task(&self, task: &Task) -> Result<()> {
        task.validate()
            .map_err(|e| StateError::Corrupt(e.to_string()))?;
        let payload = serde_json::to_string(task)?;
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO tasks
                   (task_id, trace_id, status, queue_class, created_at, updated_at, payload)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT(task_id) DO UPDATE SET
                   status = excluded.status,
                   queue_class = excluded.queue_class,
                   updated_at = excluded.updated_at,
                   payload = excluded.payload",
                sql_params![
                    task.task_id.to_string(),
                    task.trace_id.to_string(),
                    task.status.as_str(),
                    task.queue_class.as_str(),
                    task.created_at.as_millis(),
                    task.updated_at.as_millis(),
                    payload,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_task(&self, id: &TaskId) -> Result<Option<Task>> {
        self.with_connection(|conn| {
            let payload: Option<String> = conn.select_optional(
                "SELECT payload FROM tasks WHERE task_id = $1",
                sql_params![id.to_string()],
                |row| row.try_get(0),
            )?;
            match payload {
                Some(json) => Ok(Some(serde_json::from_str(&json)?)),
                None => Ok(None),
            }
        })
    }

    /// Every task in a given status, oldest first.
    pub fn tasks_with_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
        self.with_connection(|conn| {
            let rows = conn.select_many(
                "SELECT payload FROM tasks WHERE status = $1 ORDER BY created_at ASC, task_id ASC",
                sql_params![status.as_str()],
                |row| row.try_get::<_, String>(0),
            )?;
            let mut tasks = Vec::new();
            for row in rows {
                tasks.push(serde_json::from_str(&row?)?);
            }
            Ok(tasks)
        })
    }

    /// Load every non-terminal task into a graph for scheduling.
    ///
    /// Terminal tasks are included when they are a dependency of a live task, because
    /// dependency resolution needs their final status to decide whether a blocked task
    /// can proceed.
    pub fn load_active_graph(&self) -> Result<TaskGraph> {
        let mut graph = TaskGraph::new();
        let live = self.with_connection(|conn| {
            let rows = conn.select_many(
                "SELECT payload FROM tasks
                 WHERE status NOT IN ('succeeded', 'failed', 'cancelled')
                 ORDER BY created_at ASC",
                &[],
                |row| row.try_get::<_, String>(0),
            )?;
            let mut tasks: Vec<Task> = Vec::new();
            for row in rows {
                tasks.push(serde_json::from_str(&row?)?);
            }
            Ok(tasks)
        })?;

        let mut needed: Vec<TaskId> = Vec::new();
        for task in &live {
            for dependency in &task.depends_on {
                needed.push(dependency.task_id);
            }
        }
        for task in live {
            graph
                .insert(task)
                .map_err(|e| StateError::Corrupt(e.to_string()))?;
        }
        for id in needed {
            if graph.get(&id).is_none() {
                if let Some(task) = self.get_task(&id)? {
                    graph
                        .insert(task)
                        .map_err(|e| StateError::Corrupt(e.to_string()))?;
                }
            }
        }
        Ok(graph)
    }

    pub fn task_count(&self) -> Result<u64> {
        self.with_connection(|conn| {
            let count: i64 =
                conn.select_one("SELECT COUNT(*) FROM tasks", &[], |row| row.try_get(0))?;
            Ok(count as u64)
        })
    }

    // ---------------------------------------------------------------- peers

    /// Record a peer sighting.
    ///
    /// # Key pinning
    ///
    /// If the peer is already known with a *different* public key, this refuses the
    /// update and returns an error. That is trust-on-first-use: a node id is derived
    /// from its key, so a changed key for the same id means either a reinstall or an
    /// impersonation attempt, and the operator must decide which.
    pub fn observe_peer(
        &self,
        node_id: &NodeId,
        public_key: &str,
        descriptor: Option<&NodeDescriptor>,
    ) -> Result<PeerRecord> {
        let now = Timestamp::now();
        let descriptor_json = match descriptor {
            Some(d) => Some(serde_json::to_string(d)?),
            None => None,
        };

        self.with_connection(|conn| {
            let existing: Option<(String, i64, String)> = conn.select_optional(
                "SELECT public_key, first_seen, trust FROM peers WHERE node_id = $1",
                sql_params![node_id.to_string()],
                |row| Ok((row.try_get(0)?, row.try_get(1)?, row.try_get(2)?)),
            )?;

            let (first_seen, trust) = match existing {
                Some((known_key, first_seen, trust)) => {
                    if known_key != public_key {
                        return Err(StateError::Corrupt(format!(
                            "peer {node_id} presented a public key different from the pinned one; \
                             refusing to re-pin automatically"
                        )));
                    }
                    (
                        Timestamp::from_millis(first_seen),
                        TrustState::parse(&trust).unwrap_or(TrustState::Unverified),
                    )
                }
                None => (now, TrustState::Pinned),
            };

            conn.execute(
                "INSERT INTO peers
                   (node_id, public_key, first_seen, last_seen, trust, descriptor)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(node_id) DO UPDATE SET
                   last_seen = excluded.last_seen,
                   descriptor = COALESCE(excluded.descriptor, peers.descriptor)",
                sql_params![
                    node_id.to_string(),
                    public_key,
                    first_seen.as_millis(),
                    now.as_millis(),
                    trust.as_str(),
                    descriptor_json,
                ],
            )?;

            Ok(PeerRecord {
                node_id: *node_id,
                public_key: public_key.to_string(),
                first_seen,
                last_seen: now,
                trust,
                descriptor: descriptor.cloned(),
            })
        })
    }

    /// Change a peer's trust. Operator action; never automatic.
    pub fn set_peer_trust(&self, node_id: &NodeId, trust: TrustState) -> Result<()> {
        self.with_connection(|conn| {
            let changed = conn.execute(
                "UPDATE peers SET trust = $1 WHERE node_id = $2",
                sql_params![trust.as_str(), node_id.to_string()],
            )?;
            if changed == 0 {
                return Err(StateError::Corrupt(format!("unknown peer {node_id}")));
            }
            Ok(())
        })
    }

    pub fn list_peers(&self) -> Result<Vec<PeerRecord>> {
        self.with_connection(|conn| {
            let rows = conn.select_many(
                "SELECT node_id, public_key, first_seen, last_seen, trust, descriptor
                 FROM peers ORDER BY first_seen ASC",
                &[],
                |row| {
                    Ok((
                        row.try_get::<_, String>(0)?,
                        row.try_get::<_, String>(1)?,
                        row.try_get::<_, i64>(2)?,
                        row.try_get::<_, i64>(3)?,
                        row.try_get::<_, String>(4)?,
                        row.try_get::<_, Option<String>>(5)?,
                    ))
                },
            )?;

            let mut peers = Vec::new();
            for row in rows {
                let (id, key, first, last, trust, descriptor) = row?;
                let node_id = id
                    .parse::<NodeId>()
                    .map_err(|e| StateError::Corrupt(e.to_string()))?;
                let descriptor = match descriptor {
                    Some(json) => Some(serde_json::from_str(&json)?),
                    None => None,
                };
                peers.push(PeerRecord {
                    node_id,
                    public_key: key,
                    first_seen: Timestamp::from_millis(first),
                    last_seen: Timestamp::from_millis(last),
                    trust: TrustState::parse(&trust).unwrap_or(TrustState::Unverified),
                    descriptor,
                });
            }
            Ok(peers)
        })
    }

    // ---------------------------------------------------------------- devices

    /// Pin a device key as a local pairing proposal.
    ///
    /// This method is fed only by a future explicit pairing exchange; it does not
    /// perform discovery or networking. A record begins `Proposed`, and the first key
    /// is immutable. The caller must use [`Self::confirm_device_pairing`] after an
    /// authenticated operator decision before a dispatcher may consider the device.
    pub fn propose_device_pairing(
        &self,
        device_reference: &str,
        public_key: &str,
        class: DeviceClass,
    ) -> Result<DeviceRecord> {
        validate_device_reference(device_reference)?;
        validate_device_public_key(public_key)?;
        let now = Timestamp::now();

        self.with_connection(|conn| {
            let existing: Option<(String, String, i64, String)> = conn.select_optional(
                "SELECT public_key, class, first_seen, pairing_state
                     FROM devices WHERE device_reference = $1",
                sql_params![device_reference],
                |row| {
                    Ok((
                        row.try_get(0)?,
                        row.try_get(1)?,
                        row.try_get(2)?,
                        row.try_get(3)?,
                    ))
                },
            )?;

            let (first_seen, pairing) = match existing {
                Some((known_key, known_class, first_seen, pairing)) => {
                    if known_key != public_key {
                        return Err(StateError::InvalidDevice(format!(
                            "device {device_reference} presented a different pinned public key"
                        )));
                    }
                    if DeviceClass::parse(&known_class) != Some(class) {
                        return Err(StateError::InvalidDevice(format!(
                            "device {device_reference} changed its recorded class"
                        )));
                    }
                    let pairing = DevicePairingState::parse(&pairing).ok_or_else(|| {
                        StateError::Corrupt(format!(
                            "device {device_reference} has an unknown pairing state {pairing}"
                        ))
                    })?;
                    (Timestamp::from_millis(first_seen), pairing)
                }
                None => (now, DevicePairingState::Proposed),
            };

            conn.execute(
                "INSERT INTO devices
                   (device_reference, public_key, class, first_seen, last_seen, pairing_state)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT(device_reference) DO UPDATE SET
                   last_seen = excluded.last_seen",
                sql_params![
                    device_reference,
                    public_key,
                    class.as_str(),
                    first_seen.as_millis(),
                    now.as_millis(),
                    pairing.as_str(),
                ],
            )?;

            Ok(DeviceRecord {
                device_reference: device_reference.to_string(),
                public_key: public_key.to_string(),
                class,
                first_seen,
                last_seen: now,
                pairing,
            })
        })
    }

    /// Locally confirm a previously proposed device pairing.
    ///
    /// Confirming an already paired device is intentionally idempotent. A revoked
    /// record is never revived by this method; a new pairing flow must use a different
    /// device reference after the operator has reviewed the old one.
    pub fn confirm_device_pairing(&self, device_reference: &str) -> Result<()> {
        validate_device_reference(device_reference)?;
        self.with_connection(|conn| {
            let pairing: Option<String> = conn.select_optional(
                "SELECT pairing_state FROM devices WHERE device_reference = $1",
                sql_params![device_reference],
                |row| row.try_get(0),
            )?;
            let pairing = pairing.ok_or_else(|| {
                StateError::InvalidDevice(format!("unknown device {device_reference}"))
            })?;
            match DevicePairingState::parse(&pairing) {
                Some(DevicePairingState::Proposed) => {
                    conn.execute(
                        "UPDATE devices SET pairing_state = $1, last_seen = $2
                         WHERE device_reference = $3",
                        sql_params![
                            DevicePairingState::Paired.as_str(),
                            Timestamp::now().as_millis(),
                            device_reference,
                        ],
                    )?;
                    Ok(())
                }
                Some(DevicePairingState::Paired) => Ok(()),
                Some(DevicePairingState::Revoked) => Err(StateError::InvalidDevice(format!(
                    "device {device_reference} is revoked and cannot be re-paired"
                ))),
                None => Err(StateError::Corrupt(format!(
                    "device {device_reference} has an unknown pairing state {pairing}"
                ))),
            }
        })
    }

    /// Revoke a device locally. The public-key pin stays on disk for audit and to make
    /// a later unsolicited sighting fail closed rather than becoming a new proposal.
    pub fn revoke_device(&self, device_reference: &str) -> Result<()> {
        validate_device_reference(device_reference)?;
        self.with_connection(|conn| {
            let changed = conn.execute(
                "UPDATE devices SET pairing_state = $1, last_seen = $2
                 WHERE device_reference = $3",
                sql_params![
                    DevicePairingState::Revoked.as_str(),
                    Timestamp::now().as_millis(),
                    device_reference,
                ],
            )?;
            if changed == 0 {
                return Err(StateError::InvalidDevice(format!(
                    "unknown device {device_reference}"
                )));
            }
            Ok(())
        })
    }

    pub fn get_device(&self, device_reference: &str) -> Result<Option<DeviceRecord>> {
        validate_device_reference(device_reference)?;
        self.with_connection(|conn| {
            let values: Option<(String, String, String, i64, i64, String)> = conn.select_optional(
                "SELECT device_reference, public_key, class, first_seen, last_seen, pairing_state
                     FROM devices WHERE device_reference = $1",
                sql_params![device_reference],
                |row| {
                    Ok((
                        row.try_get(0)?,
                        row.try_get(1)?,
                        row.try_get(2)?,
                        row.try_get(3)?,
                        row.try_get(4)?,
                        row.try_get(5)?,
                    ))
                },
            )?;
            values.map(device_record_from_values).transpose()
        })
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceRecord>> {
        self.with_connection(|conn| {
            let rows = conn.select_many(
                "SELECT device_reference, public_key, class, first_seen, last_seen, pairing_state
                 FROM devices ORDER BY first_seen ASC, device_reference ASC",
                &[],
                |row| {
                    Ok((
                        row.try_get::<_, String>(0)?,
                        row.try_get::<_, String>(1)?,
                        row.try_get::<_, String>(2)?,
                        row.try_get::<_, i64>(3)?,
                        row.try_get::<_, i64>(4)?,
                        row.try_get::<_, String>(5)?,
                    ))
                },
            )?;
            let mut devices = Vec::new();
            for row in rows {
                devices.push(device_record_from_values(row?)?);
            }
            Ok(devices)
        })
    }

    // --------------------------------------------------------- device operations

    /// Persist a prepared, immutable operation for a currently paired device.
    ///
    /// It allocates the id used by `Scope::OneOperation`, but it does not grant a
    /// lease, choose a transport, or send a command. A device that becomes revoked
    /// after preparation must be refused again by the later dispatch boundary.
    pub fn prepare_device_operation(
        &self,
        device_reference: &str,
        trace_id: TraceId,
        operation_sha256: &str,
    ) -> Result<DeviceOperationRecord> {
        validate_operation_digest(operation_sha256)?;
        self.prepare_bound_device_operation(device_reference, trace_id, |_| {
            Ok(operation_sha256.to_owned())
        })
    }

    /// Bind the typed request to the allocated ID inside the transaction.
    /// `bind` only computes a digest and must not perform external I/O or approval.
    pub fn prepare_bound_device_operation(
        &self,
        device_reference: &str,
        trace_id: TraceId,
        bind: impl FnOnce(u64) -> Result<String> + Send,
    ) -> Result<DeviceOperationRecord> {
        validate_device_reference(device_reference)?;
        let prepared_at = Timestamp::now();

        self.with_connection(|conn| {
            let pairing: Option<String> = conn.select_optional(
                "SELECT pairing_state FROM devices WHERE device_reference = $1",
                sql_params![device_reference],
                |row| row.try_get(0),
            )?;
            let pairing = pairing.ok_or_else(|| {
                StateError::InvalidDevice(format!("unknown device {device_reference}"))
            })?;
            if DevicePairingState::parse(&pairing) != Some(DevicePairingState::Paired) {
                return Err(StateError::InvalidDevice(format!(
                    "device {device_reference} is not locally paired"
                )));
            }

            let operation_id: i64 = conn
                .query_one(
                    "SELECT nextval(pg_get_serial_sequence('device_operations','operation_id'))",
                    &[],
                )?
                .try_get(0)?;
            let allocated = u64::try_from(operation_id)
                .map_err(|_| StateError::Corrupt("invalid allocated identifier".into()))?;
            let operation_sha256 = bind(allocated)?;
            validate_operation_digest(&operation_sha256)?;
            let operation_id: i64 = conn.select_one(
                "INSERT INTO device_operations
                   (device_reference, trace_id, operation_sha256, prepared_at, state, operation_id)
                 OVERRIDING SYSTEM VALUE VALUES ($1, $2, $3, $4, $5, $6) RETURNING operation_id",
                sql_params![
                    device_reference,
                    trace_id.to_string(),
                    &operation_sha256,
                    prepared_at.as_millis(),
                    DeviceOperationState::Prepared.as_str(),
                    operation_id,
                ],
                |row| row.try_get(0),
            )?;
            let operation_id = u64::try_from(operation_id)
                .map_err(|_| StateError::Corrupt("invalid allocated identifier".into()))?;
            Ok(DeviceOperationRecord {
                operation_id,
                device_reference: device_reference.to_string(),
                trace_id,
                operation_sha256: operation_sha256.to_string(),
                prepared_at,
                state: DeviceOperationState::Prepared,
            })
        })
    }

    /// Cancel an operation before any future adapter may consider dispatching it.
    /// Cancellation is idempotent; it never restores a cancelled record to `Prepared`.
    pub fn cancel_device_operation(&self, operation_id: u64) -> Result<()> {
        let operation_id = operation_id_to_sql(operation_id)?;
        self.with_connection(|conn| {
            let state: Option<String> = conn.select_optional(
                "SELECT state FROM device_operations WHERE operation_id = $1",
                sql_params![operation_id],
                |row| row.try_get(0),
            )?;
            let state = state.ok_or_else(|| {
                StateError::InvalidDevice(format!("unknown device operation {operation_id}"))
            })?;
            match DeviceOperationState::parse(&state) {
                Some(DeviceOperationState::Prepared) => {
                    conn.execute(
                        "UPDATE device_operations SET state = $1 WHERE operation_id = $2",
                        sql_params![DeviceOperationState::Cancelled.as_str(), operation_id],
                    )?;
                    Ok(())
                }
                Some(DeviceOperationState::Cancelled) => Ok(()),
                None => Err(StateError::Corrupt(format!(
                    "device operation {operation_id} has an unknown state {state}"
                ))),
            }
        })
    }

    /// Validate pairing and preparation together under the shared writer fence.
    pub fn validate_prepared_device_operation(
        &self,
        operation_id: u64,
        device_reference: &str,
        trace_id: TraceId,
        operation_sha256: &str,
    ) -> Result<()> {
        let operation_id = operation_id_to_sql(operation_id)?;
        validate_device_reference(device_reference)?;
        validate_operation_digest(operation_sha256)?;
        self.with_connection(|tx| {
            let valid:bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM device_operations o JOIN devices d ON d.device_reference=o.device_reference WHERE o.operation_id=$1 AND d.device_reference=$2 AND o.trace_id=$3 AND o.operation_sha256=$4 AND o.state='prepared' AND d.pairing_state='paired')",&[&operation_id,&device_reference,&trace_id.to_string(),&operation_sha256])?.try_get(0)?;
            if !valid { return Err(StateError::InvalidDevice("prepared operation identity or pairing refused".into())); }
            Ok(())
        })
    }

    pub fn get_device_operation(&self, operation_id: u64) -> Result<Option<DeviceOperationRecord>> {
        let operation_id = operation_id_to_sql(operation_id)?;
        self.with_connection(|conn| {
            let values: Option<(i64, String, String, String, i64, String)> = conn
                .select_optional(
                    "SELECT operation_id, device_reference, trace_id, operation_sha256, prepared_at, state
                     FROM device_operations WHERE operation_id = $1",
                    sql_params![operation_id],
                    |row| {
                        Ok((
                            row.try_get(0)?,
                            row.try_get(1)?,
                            row.try_get(2)?,
                            row.try_get(3)?,
                            row.try_get(4)?,
                            row.try_get(5)?,
                        ))
                    },
                )?;
            values.map(device_operation_from_values).transpose()
        })
    }

    /// Most recently prepared records first, regardless of whether they were later
    /// cancelled. A native control view can therefore show that an operator stopped an
    /// action instead of making it disappear from history.
    pub fn recent_device_operations(&self, limit: u32) -> Result<Vec<DeviceOperationRecord>> {
        self.with_connection(|conn| {
            let rows = conn.select_many("SELECT operation_id, device_reference, trace_id, operation_sha256, prepared_at, state
                 FROM device_operations ORDER BY operation_id DESC LIMIT $1", sql_params![i64::from(limit)], |row| {
                Ok((
                    row.try_get::<_, i64>(0)?,
                    row.try_get::<_, String>(1)?,
                    row.try_get::<_, String>(2)?,
                    row.try_get::<_, String>(3)?,
                    row.try_get::<_, i64>(4)?,
                    row.try_get::<_, String>(5)?,
                ))
            })?;
            let mut records = Vec::new();
            for row in rows {
                records.push(device_operation_from_values(row?)?);
            }
            Ok(records)
        })
    }

    // ------------------------------------------------------ adapter proposals

    /// Persist a generated adapter proposal for later operator/native implementation
    /// review. This never registers a module, pairs a device, grants a lease, or opens
    /// a transport. Native contracts must use the separate registry admission path.
    pub fn record_generated_adapter_proposal(
        &self,
        manifest: &DeviceAdapterManifest,
    ) -> Result<DeviceAdapterProposalRecord> {
        manifest
            .validate()
            .map_err(|error| StateError::InvalidDevice(error.to_string()))?;
        if manifest.origin != DeviceAdapterOrigin::GeneratedProposal {
            return Err(StateError::InvalidDevice(
                "only a generated adapter proposal may enter proposal history".into(),
            ));
        }
        let manifest_json = serde_json::to_string(manifest)?;
        let manifest_sha256 = sha256_hex(manifest_json.as_bytes());
        let proposed_at = Timestamp::now();
        self.with_connection(|conn| {
            let proposal_id: i64 = conn.select_one(
                "INSERT INTO device_adapter_proposals
                   (adapter_id, module_id, manifest_json, manifest_sha256, proposed_at)
                 VALUES ($1, $2, $3, $4, $5) RETURNING proposal_id",
                sql_params![
                    manifest.adapter_id,
                    manifest.module_id.to_string(),
                    manifest_json,
                    manifest_sha256,
                    proposed_at.as_millis(),
                ],
                |row| row.try_get(0),
            )?;
            let proposal_id = u64::try_from(proposal_id)
                .map_err(|_| StateError::Corrupt("invalid allocated identifier".into()))?;
            Ok(DeviceAdapterProposalRecord {
                proposal_id,
                manifest: manifest.clone(),
                manifest_sha256,
                proposed_at,
            })
        })
    }

    /// Newest generated proposals first. Reading verifies the immutable JSON hash and
    /// rejects any malformed or non-proposal entry rather than treating it as code.
    pub fn recent_generated_adapter_proposals(
        &self,
        limit: u32,
    ) -> Result<Vec<DeviceAdapterProposalRecord>> {
        self.with_connection(|conn| {
            let rows = conn.select_many("SELECT proposal_id, adapter_id, module_id, manifest_json, manifest_sha256, proposed_at
                 FROM device_adapter_proposals ORDER BY proposal_id DESC LIMIT $1", sql_params![i64::from(limit)], |row| {
                Ok((
                    row.try_get::<_, i64>(0)?,
                    row.try_get::<_, String>(1)?,
                    row.try_get::<_, String>(2)?,
                    row.try_get::<_, String>(3)?,
                    row.try_get::<_, String>(4)?,
                    row.try_get::<_, i64>(5)?,
                ))
            })?;
            let mut proposals = Vec::new();
            for row in rows {
                proposals.push(device_adapter_proposal_from_values(row?)?);
            }
            Ok(proposals)
        })
    }

    // --------------------------------------------------- Cortex corrections

    /// Archive an external graph-writer receipt without publishing a graph version.
    /// Authoritative local graph changes use `apply_cortex_correction`; this
    /// compatibility entry point cannot manufacture a local graph commit.
    pub fn record_cortex_correction_trajectory(
        &self,
        trajectory: &CorrectionTrajectory,
    ) -> Result<CortexCorrectionTrajectoryWrite> {
        trajectory
            .validate()
            .map_err(|e| StateError::InvalidCortexTrajectory(e.to_string()))?;
        let payload = serde_json::to_string(trajectory)?;
        let hash = sha256_hex(payload.as_bytes());
        self.with_connection(|conn| {
            if let Some(row) = conn.query_opt("SELECT trajectory_id,correction_id,payload_json,payload_sha256 FROM (SELECT trajectory_id,correction_id,payload_json,payload_sha256 FROM cortex_receipt_history UNION SELECT trajectory_id,correction_id,payload_json,payload_sha256 FROM cortex_correction_trajectories) AS receipts WHERE correction_id=$1 OR trajectory_id=$2", &[&trajectory.correction.correction_id,&trajectory.trajectory_id])? {
                let existing = cortex_trajectory_from_values((row.try_get(0)?,row.try_get(1)?,row.try_get(2)?,row.try_get(3)?))?;
                if existing.trajectory != *trajectory { return Err(StateError::InvalidCortexTrajectory("receipt identity conflict".into())); }
                return Ok(CortexCorrectionTrajectoryWrite::AlreadyRecorded(existing));
            }
            conn.execute("INSERT INTO cortex_receipt_history VALUES ($1,$2,$3,$4,$5)", &[&trajectory.trajectory_id,&trajectory.correction.correction_id,&trajectory.captured_at_ms,&payload,&hash])?;
            Ok(CortexCorrectionTrajectoryWrite::Recorded(CortexCorrectionTrajectoryRecord { trajectory:trajectory.clone(),payload_sha256:hash }))
        })
    }

    /// Newest correction trajectories first. The payload hash, indexed identifiers,
    /// graph-writer receipt and every cross-reference are revalidated on read so a
    /// corrupt row cannot silently enter replay or training.
    pub fn recent_cortex_correction_trajectories(
        &self,
        limit: u32,
    ) -> Result<Vec<CortexCorrectionTrajectoryRecord>> {
        self.with_connection(|conn| {
            let rows = conn.select_many("SELECT trajectory_id, correction_id, payload_json, payload_sha256
                 FROM (SELECT trajectory_id,correction_id,captured_at,payload_json,payload_sha256 FROM cortex_correction_trajectories UNION SELECT trajectory_id,correction_id,captured_at,payload_json,payload_sha256 FROM cortex_receipt_history) AS receipts ORDER BY captured_at DESC, trajectory_id DESC
                 LIMIT $1", sql_params![i64::from(limit)], |row| {
                Ok((
                    row.try_get::<_, String>(0)?,
                    row.try_get::<_, String>(1)?,
                    row.try_get::<_, String>(2)?,
                    row.try_get::<_, String>(3)?,
                ))
            })?;
            let mut trajectories = Vec::new();
            for row in rows {
                trajectories.push(cortex_trajectory_from_values(row?)?);
            }
            Ok(trajectories)
        })
    }

    // ---------------------------------------------------------------- audit

    pub fn append_audit(&self, event: &AuditEvent) -> Result<()> {
        let payload = serde_json::to_string(event)?;
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO audit (recorded_at, node_id, action, outcome, payload)
                 VALUES ($1, $2, $3, $4, $5)",
                sql_params![
                    event.recorded_at.as_millis(),
                    event.node.to_string(),
                    event.action,
                    event.outcome.as_str(),
                    payload,
                ],
            )?;
            Ok(())
        })
    }

    /// Most recent audit records, newest first.
    pub fn recent_audit(&self, limit: u32) -> Result<Vec<AuditEvent>> {
        self.with_connection(|conn| {
            let rows = conn.select_many(
                "SELECT payload FROM audit ORDER BY id DESC LIMIT $1",
                sql_params![i64::from(limit)],
                |row| row.try_get::<_, String>(0),
            )?;
            let mut events = Vec::new();
            for row in rows {
                events.push(serde_json::from_str(&row?)?);
            }
            Ok(events)
        })
    }
}
fn validate_device_reference(device_reference: &str) -> Result<()> {
    if device_reference.is_empty()
        || device_reference.len() > 128
        || !device_reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(StateError::InvalidDevice(
            "device reference must be 1..=128 ASCII letters, digits, dots, underscores, or hyphens"
                .into(),
        ));
    }
    Ok(())
}

fn validate_device_public_key(public_key: &str) -> Result<()> {
    if public_key.len() != 64
        || !public_key
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(StateError::InvalidDevice(
            "device public key must be a lower-case Ed25519 hex value".into(),
        ));
    }
    Ok(())
}

fn validate_operation_digest(operation_sha256: &str) -> Result<()> {
    if operation_sha256.len() != 64
        || !operation_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(StateError::InvalidDevice(
            "device operation digest must be a lower-case SHA-256 value".into(),
        ));
    }
    Ok(())
}

fn device_record_from_values(
    (device_reference, public_key, class, first_seen, last_seen, pairing): (
        String,
        String,
        String,
        i64,
        i64,
        String,
    ),
) -> Result<DeviceRecord> {
    validate_device_reference(&device_reference).map_err(|error| {
        StateError::Corrupt(format!("stored device reference is invalid: {error}"))
    })?;
    validate_device_public_key(&public_key).map_err(|error| {
        StateError::Corrupt(format!("stored device public key is invalid: {error}"))
    })?;
    let class = DeviceClass::parse(&class)
        .ok_or_else(|| StateError::Corrupt(format!("stored device has unknown class {class}")))?;
    let pairing = DevicePairingState::parse(&pairing).ok_or_else(|| {
        StateError::Corrupt(format!("stored device has unknown pairing state {pairing}"))
    })?;
    Ok(DeviceRecord {
        device_reference,
        public_key,
        class,
        first_seen: Timestamp::from_millis(first_seen),
        last_seen: Timestamp::from_millis(last_seen),
        pairing,
    })
}

fn operation_id_to_sql(operation_id: u64) -> Result<i64> {
    i64::try_from(operation_id).map_err(|_| {
        StateError::InvalidDevice("device operation id exceeds PostgreSQL BIGINT range".into())
    })
}

fn device_operation_from_values(
    (operation_id, device_reference, trace_id, operation_sha256, prepared_at, state): (
        i64,
        String,
        String,
        String,
        i64,
        String,
    ),
) -> Result<DeviceOperationRecord> {
    let operation_id = u64::try_from(operation_id)
        .map_err(|_| StateError::Corrupt("stored device operation id was negative".into()))?;
    validate_device_reference(&device_reference).map_err(|error| {
        StateError::Corrupt(format!(
            "stored device operation reference is invalid: {error}"
        ))
    })?;
    validate_operation_digest(&operation_sha256).map_err(|error| {
        StateError::Corrupt(format!(
            "stored device operation digest is invalid: {error}"
        ))
    })?;
    let trace_id = trace_id.parse::<TraceId>().map_err(|error| {
        StateError::Corrupt(format!("stored device operation trace is invalid: {error}"))
    })?;
    let state = DeviceOperationState::parse(&state).ok_or_else(|| {
        StateError::Corrupt(format!("stored device operation has unknown state {state}"))
    })?;
    Ok(DeviceOperationRecord {
        operation_id,
        device_reference,
        trace_id,
        operation_sha256,
        prepared_at: Timestamp::from_millis(prepared_at),
        state,
    })
}

fn device_adapter_proposal_from_values(
    (proposal_id, adapter_id, module_id, manifest_json, manifest_sha256, proposed_at): (
        i64,
        String,
        String,
        String,
        String,
        i64,
    ),
) -> Result<DeviceAdapterProposalRecord> {
    let proposal_id = u64::try_from(proposal_id)
        .map_err(|_| StateError::Corrupt("stored adapter proposal id was negative".into()))?;
    if sha256_hex(manifest_json.as_bytes()) != manifest_sha256 {
        return Err(StateError::Corrupt(
            "stored adapter proposal digest does not match its manifest".into(),
        ));
    }
    let manifest: DeviceAdapterManifest = serde_json::from_str(&manifest_json)?;
    manifest.validate().map_err(|error| {
        StateError::Corrupt(format!("stored adapter proposal is invalid: {error}"))
    })?;
    if manifest.origin != DeviceAdapterOrigin::GeneratedProposal {
        return Err(StateError::Corrupt(
            "stored adapter proposal is not generated".into(),
        ));
    }
    if manifest.adapter_id != adapter_id || manifest.module_id.to_string() != module_id {
        return Err(StateError::Corrupt(
            "stored adapter proposal index does not match its manifest".into(),
        ));
    }
    Ok(DeviceAdapterProposalRecord {
        proposal_id,
        manifest,
        manifest_sha256,
        proposed_at: Timestamp::from_millis(proposed_at),
    })
}

fn cortex_trajectory_from_values(
    (trajectory_id, correction_id, payload_json, payload_sha256): (String, String, String, String),
) -> Result<CortexCorrectionTrajectoryRecord> {
    if sha256_hex(payload_json.as_bytes()) != payload_sha256 {
        return Err(StateError::Corrupt(
            "stored Cortex trajectory digest does not match its payload".into(),
        ));
    }
    let trajectory: CorrectionTrajectory = serde_json::from_str(&payload_json)?;
    trajectory.validate().map_err(|error| {
        StateError::Corrupt(format!("stored Cortex trajectory is invalid: {error}"))
    })?;
    if trajectory.trajectory_id != trajectory_id
        || trajectory.correction.correction_id != correction_id
    {
        return Err(StateError::Corrupt(
            "stored Cortex trajectory indexes do not match its payload".into(),
        ));
    }
    Ok(CortexCorrectionTrajectoryRecord {
        trajectory,
        payload_sha256,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Implementing the sink here means a policy decision and its audit record commit
/// through the same database, so an allow cannot be persisted without its justification.
impl AuditSink for StateStore {
    fn record(&self, event: AuditEvent) -> std::result::Result<(), AuditError> {
        self.append_audit(&event)
            .map_err(|e| AuditError::Unavailable(e.to_string()))
    }
}

impl Drop for StateStore {
    fn drop(&mut self) {
        if let Some(schema) = &self.cleanup_schema {
            let _: Result<()> = self.cognitive.with_client(|client| {
                client.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))?;
                Ok(())
            });
        }
    }
}
