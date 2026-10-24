//! PostgreSQL cognitive persistence used by Core.
//!
//! Only literal loopback TCP endpoints are supported until verified TLS is added.
//! The synchronous driver is owned and operated on dedicated blocking threads so
//! callers may open and drop the store from a Tokio runtime safely.
mod correction;
mod execution;
mod inference;
use archie_memory::{
    cognitive::{reference, CognitiveEvent, CognitivePayload, FocusRecord},
    graph::GraphSnapshot,
    shared_thought::SharedThoughtState,
    storage::{EventStore, GraphStore, RecordedCognitiveEvent, ReplayStore, SharedThoughtStore},
};
pub use correction::CorrectionReceipt;
use postgres::{config::Host, Client, Config, GenericClient, NoTls, Row, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{net::IpAddr, sync::Mutex, time::Duration};

const SCHEMA: &str = include_str!("schema.sql");
const INFERENCE_V2: &str = include_str!("inference-v2.sql");
// One database-wide writer lock also serializes migration with normal writes.
// This preserves single-writer ordering, including replay watermarks.
const WRITER_LOCK: i64 = 0x4152_4348_4945_5047;
const EVENT_COLUMNS: &str =
    "sequence,event_id,recorded_at,focus_id,focus_revision,payload_json,payload_sha256";
const GRAPH_COLUMNS: &str = "snapshot_id,captured_at,payload_json,payload_sha256";
const THOUGHT_COLUMNS: &str = "thought_id,revision,recorded_at,focus_id,payload,hash";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    // Do not expose server error text: it may contain credentials or row payloads.
    #[error("PostgreSQL operation failed (SQLSTATE {0})")]
    Database(String),
    #[error("invalid local PostgreSQL configuration")]
    Configuration,
    #[error("unsupported cognitive PostgreSQL schema")]
    Schema,
    #[error("cognitive storage invariant failed: {0}")]
    Invariant(&'static str),
    #[error("cognitive storage connection lock was poisoned")]
    Poisoned,
}
impl From<postgres::Error> for StoreError {
    fn from(error: postgres::Error) -> Self {
        Self::Database(error.code().map_or("unavailable", |c| c.code()).to_owned())
    }
}
pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    pub sequence: i64,
    pub topic: String,
    pub object_id: String,
    pub payload_sha256: String,
    pub recorded_at_ms: i64,
}

/// No Debug implementation: connection configuration must never be logged.
pub struct PostgresStore {
    client: Mutex<Option<Client>>,
}

pub fn validate_local_configuration(config: &Config, schema: &str) -> Result<()> {
    if !schema.starts_with("archie_")
        || !(8..=55).contains(&schema.len())
        || !schema
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        || config.get_hosts().len() != 1
        || !matches!(&config.get_hosts()[0], Host::Tcp(host) if host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()))
        || config.get_hostaddrs().iter().any(|ip| !ip.is_loopback())
        || config.get_ssl_mode() == postgres::config::SslMode::Require
        || config.get_dbname().is_none()
        || config.get_user().is_none()
    {
        return Err(StoreError::Configuration);
    }
    Ok(())
}

impl PostgresStore {
    /// Provision/open a dedicated schema on PostgreSQL 18.
    /// The role must own this schema; PUBLIC gets no schema access.
    pub fn open_local(config: Config, schema: &str) -> Result<Self> {
        validate_local_configuration(&config, schema)?;
        let schema = schema.to_owned();
        std::thread::spawn(move || Self::connect_local(config, &schema))
            .join()
            .map_err(|_| StoreError::Poisoned)?
    }
    fn connect_local(mut config: Config, schema: &str) -> Result<Self> {
        config.ssl_mode(postgres::config::SslMode::Disable);
        config.connect_timeout(Duration::from_secs(3));
        config.application_name("archie-cognitive-store");
        config.options("-c statement_timeout=5000 -c lock_timeout=2000 -c idle_in_transaction_session_timeout=10000 -c synchronous_commit=on");
        let mut client = config.connect(NoTls)?;
        let version: i32 = client
            .query_one("SELECT current_setting('server_version_num')::integer", &[])?
            .try_get(0)?;
        if version / 10000 != 18 {
            return Err(StoreError::Configuration);
        }
        let durable: bool = client.query_one("SELECT current_setting('fsync')::boolean AND current_setting('full_page_writes')::boolean AND NOT pg_is_in_recovery()", &[])?.try_get(0)?;
        if !durable {
            return Err(StoreError::Configuration);
        }
        let mut tx = client.transaction()?;
        writer_lock(&mut tx)?;
        // Schema identifier was restricted to lowercase ASCII, digits and '_'.
        tx.batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}; REVOKE ALL ON SCHEMA {schema} FROM PUBLIC; SET search_path TO {schema}, pg_catalog;"))?;
        tx.batch_execute("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, schema_sha256 TEXT NOT NULL)")?;
        let versions = tx.query(
            "SELECT version,schema_sha256 FROM schema_migrations ORDER BY version LIMIT 3",
            &[],
        )?;
        if versions.is_empty() {
            tx.batch_execute(SCHEMA)?;
            tx.execute(
                "INSERT INTO schema_migrations VALUES (1,$1)",
                &[&schema_digest()],
            )?;
        } else if versions.len() > 2
            || versions[0].try_get::<_, i32>(0)? != 1
            || versions[0].try_get::<_, String>(1)? != schema_digest()
        {
            return Err(StoreError::Schema);
        }
        if versions.len() == 2 {
            if versions[1].try_get::<_, i32>(0)? != 2
                || versions[1].try_get::<_, String>(1)?
                    != digest(&INFERENCE_V2.replace("\r\n", "\n"))
            {
                return Err(StoreError::Schema);
            }
        } else {
            tx.batch_execute(INFERENCE_V2)?;
            tx.execute(
                "INSERT INTO schema_migrations VALUES (2,$1)",
                &[&digest(&INFERENCE_V2.replace("\r\n", "\n"))],
            )?;
        }
        tx.commit()?;
        Ok(Self {
            client: Mutex::new(Some(client)),
        })
    }

    pub fn with_client<T: Send, E: From<StoreError> + Send>(
        &self,
        f: impl FnOnce(&mut Client) -> std::result::Result<T, E> + Send,
    ) -> std::result::Result<T, E> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let mut guard = self.client.lock().map_err(|_| StoreError::Poisoned)?;
                    f(guard.as_mut().ok_or(StoreError::Poisoned)?)
                })
                .join()
                .map_err(|_| E::from(StoreError::Poisoned))?
        })
    }
    fn read<T: Send>(&self, f: impl FnOnce(&mut Client) -> Result<T> + Send) -> Result<T> {
        self.with_client(f)
    }
    pub fn with_transaction<T: Send, E: From<StoreError> + Send>(
        &self,
        f: impl FnOnce(&mut Transaction<'_>) -> std::result::Result<T, E> + Send,
    ) -> std::result::Result<T, E> {
        self.with_client(|client| {
            let mut tx = client.transaction().map_err(StoreError::from)?;
            writer_lock(&mut tx)?;
            let result = f(&mut tx)?;
            tx.commit().map_err(StoreError::from)?;
            Ok(result)
        })
    }
    fn write<T: Send>(
        &self,
        f: impl FnOnce(&mut Transaction<'_>) -> Result<T> + Send,
    ) -> Result<T> {
        self.with_transaction(f)
    }
    /// A stricter caller-owned deadline, never a grant of storage/model authority.
    /// The monotonic guard includes connection/writer-lock waits. Both checks run
    /// inside the existing transaction so expiry rolls back all secondary writes.
    pub(crate) fn write_before<T: Send>(
        &self,
        deadline_ms: i64,
        f: impl FnOnce(&mut Transaction<'_>) -> Result<T> + Send,
    ) -> Result<T> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| StoreError::Invariant("inference commit clock invalid"))?
            .as_millis();
        let remaining = i128::from(deadline_ms) - now_ms as i128;
        if !(1..=600000).contains(&remaining) {
            return Err(StoreError::Invariant(
                "inference commit deadline expired or unbounded",
            ));
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(remaining as u64);
        self.with_transaction(|tx| {
            let check = || {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| StoreError::Invariant("inference commit clock invalid"))?
                    .as_millis();
                if std::time::Instant::now() >= deadline
                    || now_ms as i128 >= i128::from(deadline_ms)
                {
                    Err(StoreError::Invariant("inference commit deadline expired"))
                } else {
                    Ok(())
                }
            };
            check()?;
            let milliseconds = deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_millis()
                .max(1);
            tx.query_one(
                "SELECT set_config('statement_timeout',$1,true)",
                &[&format!("{milliseconds}ms")],
            )?;
            let result = f(tx)?;
            check()?;
            Ok(result)
        })
    }
    pub fn pending_outbox(&self, limit: u32) -> Result<Vec<OutboxEntry>> {
        bounded_limit(limit)?;
        self.read(|client| client.query("SELECT sequence,topic,object_id,payload_sha256,recorded_at FROM cognitive_outbox WHERE NOT acknowledged ORDER BY sequence LIMIT $1", &[&i64::from(limit)])?
            .into_iter().map(|row| Ok(OutboxEntry {
                sequence: row.try_get(0)?, topic: row.try_get(1)?, object_id: row.try_get(2)?,
                payload_sha256: row.try_get(3)?, recorded_at_ms: row.try_get(4)?,
            })).collect())
    }
    pub fn acknowledge_outbox(&self, sequence: i64, hash: &str) -> Result<()> {
        if sequence <= 0 || hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StoreError::Invariant("invalid outbox acknowledgement"));
        }
        self.write(|tx| {
            if tx.execute("UPDATE cognitive_outbox SET acknowledged=TRUE WHERE sequence=$1 AND payload_sha256=$2", &[&sequence,&hash])? != 1 {
                return Err(StoreError::Invariant("outbox identity or digest mismatch"));
            }
            Ok(())
        })
    }
}

impl Drop for PostgresStore {
    fn drop(&mut self) {
        let client = self
            .client
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        // postgres owns a Tokio runtime; its creation, calls and destruction must
        // happen outside the daemon's async runtime.
        let _ = std::thread::spawn(move || drop(client)).join();
    }
}

fn writer_lock(tx: &mut Transaction<'_>) -> Result<()> {
    tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&WRITER_LOCK])?;
    Ok(())
}
fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn schema_digest() -> String {
    // Git may check out SQL as CRLF on Windows; this is one schema on both hosts.
    digest(&SCHEMA.replace("\r\n", "\n"))
}
fn encode<T: Serialize>(value: &T, limit: usize) -> Result<String> {
    let json =
        serde_json::to_string(value).map_err(|_| StoreError::Invariant("invalid serialization"))?;
    if json.len() > limit {
        return Err(StoreError::Invariant("payload exceeds bound"));
    }
    Ok(json)
}
fn decode<T: DeserializeOwned>(json: &str, hash: &str, limit: usize) -> Result<T> {
    if json.len() > limit || digest(json) != hash {
        return Err(StoreError::Invariant("payload digest or size mismatch"));
    }
    serde_json::from_str(json).map_err(|_| StoreError::Invariant("invalid payload JSON"))
}
fn bounded_limit(limit: u32) -> Result<()> {
    if limit == 0 || limit > 1000 {
        return Err(StoreError::Invariant("invalid read limit"));
    }
    Ok(())
}
fn query_key(id: &str, time: i64) -> Result<()> {
    if !reference(id) || time <= 0 {
        return Err(StoreError::Invariant("invalid historical query"));
    }
    Ok(())
}
fn enqueue(tx: &mut Transaction<'_>, topic: &str, id: &str, json: &str, time: i64) -> Result<()> {
    tx.execute("INSERT INTO cognitive_outbox(topic,object_id,payload_sha256,recorded_at) VALUES ($1,$2,$3,$4)", &[&topic,&id,&digest(json),&time])?;
    Ok(())
}
fn event_index(event: &CognitiveEvent) -> (Option<&str>, Option<i64>) {
    match &event.payload {
        CognitivePayload::FocusUpdated(f) => (Some(f.focus_id.as_str()), Some(f.revision as i64)),
        _ => (None, None),
    }
}
fn decode_event(row: Row) -> Result<RecordedCognitiveEvent> {
    let event: CognitiveEvent = decode(row.try_get(5)?, row.try_get(6)?, 65536)?;
    event
        .validate()
        .map_err(|_| StoreError::Invariant("invalid cognitive event"))?;
    let (focus, revision) = event_index(&event);
    let sequence: i64 = row.try_get(0)?;
    if sequence <= 0
        || event.event_id != row.try_get::<_, String>(1)?
        || event.recorded_at_ms != row.try_get::<_, i64>(2)?
        || focus != row.try_get::<_, Option<&str>>(3)?
        || revision != row.try_get::<_, Option<i64>>(4)?
    {
        return Err(StoreError::Invariant("cognitive event index mismatch"));
    }
    Ok(RecordedCognitiveEvent { sequence, event })
}
fn focus_at(client: &mut impl GenericClient, id: &str, time: i64) -> Result<Option<FocusRecord>> {
    client.query_opt(&format!("SELECT {EVENT_COLUMNS} FROM cognitive_events WHERE focus_id=$1 AND recorded_at<=$2 ORDER BY recorded_at DESC,sequence DESC LIMIT 1"), &[&id,&time])?
        .map(|row| match decode_event(row)?.event.payload {
            CognitivePayload::FocusUpdated(f) => Ok(f),
            _ => Err(StoreError::Invariant("Focus row is not a Focus")),
        }).transpose()
}
impl EventStore for PostgresStore {
    type Error = StoreError;
    fn append_event(&self, event: &CognitiveEvent) -> Result<i64> {
        event
            .validate()
            .map_err(|_| StoreError::Invariant("invalid cognitive event"))?;
        let json = encode(event, 65536)?;
        self.write(|tx| {
            if let Some(row) = tx.query_opt(&format!("SELECT {EVENT_COLUMNS} FROM cognitive_events WHERE event_id=$1"), &[&event.event_id])? {
                let stored = decode_event(row)?;
                return if stored.event == *event { Ok(stored.sequence) } else { Err(StoreError::Invariant("event ID collision")) };
            }
            if let CognitivePayload::FocusUpdated(f) = &event.payload {
                match focus_at(tx,&f.focus_id,i64::MAX)? {
                    Some(old) if old.revision.checked_add(1) == Some(f.revision) && f.updated_at_ms >= old.updated_at_ms && f.parent_focus_id == old.parent_focus_id => (),
                    None if f.revision == 1 => (),
                    _ => return Err(StoreError::Invariant("stale Focus revision, time or changed parent")),
                }
                if let Some(parent) = &f.parent_focus_id {
                    if focus_at(tx,parent,f.updated_at_ms)?.is_none() { return Err(StoreError::Invariant("Focus parent missing at event time")); }
                }
            }
            let (focus,revision) = event_index(event);
            let row = tx.query_one("INSERT INTO cognitive_events(event_id,recorded_at,focus_id,focus_revision,payload_json,payload_sha256) VALUES ($1,$2,$3,$4,$5,$6) RETURNING sequence", &[&event.event_id,&event.recorded_at_ms,&focus,&revision,&json,&digest(&json)])?;
            enqueue(tx,"event",&event.event_id,&json,event.recorded_at_ms)?;
            Ok(row.try_get(0)?)
        })
    }
    fn event_watermark(&self) -> Result<i64> {
        self.read(|client| {
            Ok(client
                .query_one(
                    "SELECT COALESCE(MAX(sequence),0) FROM cognitive_events",
                    &[],
                )?
                .try_get(0)?)
        })
    }
    fn focus_at(&self, id: &str, time: i64) -> Result<Option<FocusRecord>> {
        query_key(id, time)?;
        self.read(|client| focus_at(client, id, time))
    }
}
impl ReplayStore for PostgresStore {
    fn replay_events(
        &self,
        after: i64,
        through: i64,
        time: i64,
        limit: u32,
    ) -> Result<Vec<RecordedCognitiveEvent>> {
        bounded_limit(limit)?;
        if after < 0 || through < after || time <= 0 {
            return Err(StoreError::Invariant("invalid bounded replay cursor"));
        }
        self.read(|client| client.query(&format!("SELECT {EVENT_COLUMNS} FROM cognitive_events WHERE sequence>$1 AND sequence<=$2 AND recorded_at<=$3 ORDER BY sequence LIMIT $4"), &[&after,&through,&time,&i64::from(limit)])?
            .into_iter().map(decode_event).collect())
    }
}
fn decode_graph(row: Row) -> Result<GraphSnapshot> {
    let graph: GraphSnapshot = decode(row.try_get(2)?, row.try_get(3)?, 64 * 1024 * 1024)?;
    graph
        .validate()
        .map_err(|_| StoreError::Invariant("invalid graph"))?;
    if graph.snapshot_id != row.try_get::<_, String>(0)?
        || graph.captured_at_ms != row.try_get::<_, i64>(1)?
    {
        return Err(StoreError::Invariant("graph index mismatch"));
    }
    Ok(graph)
}
fn graph_at(client: &mut impl GenericClient, time: i64) -> Result<Option<GraphSnapshot>> {
    client.query_opt(&format!("SELECT {GRAPH_COLUMNS} FROM cortex_graph_snapshots WHERE captured_at<=$1 ORDER BY captured_at DESC,sequence DESC LIMIT 1"), &[&time])?.map(decode_graph).transpose()
}
impl GraphStore for PostgresStore {
    type Error = StoreError;
    fn graph_at(&self, time: i64) -> Result<Option<GraphSnapshot>> {
        if time <= 0 {
            return Err(StoreError::Invariant("invalid graph replay time"));
        }
        self.read(|client| graph_at(client, time))
    }
    fn publish_graph(&self, graph: &GraphSnapshot, expected: Option<&str>) -> Result<()> {
        self.write(|tx| publish_graph_tx(tx, graph, expected))
    }
}
fn publish_graph_tx(
    tx: &mut Transaction<'_>,
    graph: &GraphSnapshot,
    expected: Option<&str>,
) -> Result<()> {
    graph
        .validate()
        .map_err(|_| StoreError::Invariant("invalid graph"))?;
    let json = encode(graph, 64 * 1024 * 1024)?;
    let current = graph_at(tx, i64::MAX)?;
    if current.as_ref().map(|g| g.snapshot_id.as_str()) != expected
        || current
            .as_ref()
            .is_some_and(|g| g.captured_at_ms >= graph.captured_at_ms)
    {
        return Err(StoreError::Invariant(
            "stale graph parent or non-increasing time",
        ));
    }
    tx.execute("INSERT INTO cortex_graph_snapshots(snapshot_id,captured_at,payload_json,payload_sha256) VALUES ($1,$2,$3,$4)", &[&graph.snapshot_id,&graph.captured_at_ms,&json,&digest(&json)])?;
    enqueue(tx, "graph", &graph.snapshot_id, &json, graph.captured_at_ms)
}
fn decode_thought(row: Row) -> Result<SharedThoughtState> {
    let thought: SharedThoughtState = decode(row.try_get(4)?, row.try_get(5)?, 65536)?;
    thought
        .validate()
        .map_err(|_| StoreError::Invariant("invalid SharedThought"))?;
    if thought.thought_id != row.try_get::<_, String>(0)?
        || thought.revision as i64 != row.try_get::<_, i64>(1)?
        || thought.updated_at_ms != row.try_get::<_, i64>(2)?
        || thought.focus_id != row.try_get::<_, String>(3)?
    {
        return Err(StoreError::Invariant("SharedThought index mismatch"));
    }
    Ok(thought)
}
fn thought_at(
    client: &mut impl GenericClient,
    id: &str,
    time: i64,
) -> Result<Option<SharedThoughtState>> {
    client.query_opt(&format!("SELECT {THOUGHT_COLUMNS} FROM shared_thought_snapshots WHERE thought_id=$1 AND recorded_at<=$2 ORDER BY recorded_at DESC,revision DESC LIMIT 1"), &[&id,&time])?.map(decode_thought).transpose()
}
impl SharedThoughtStore for PostgresStore {
    type Error = StoreError;
    fn thought_at(&self, id: &str, time: i64) -> Result<Option<SharedThoughtState>> {
        query_key(id, time)?;
        self.read(|client| thought_at(client, id, time))
    }
    fn publish_thought(&self, thought: &SharedThoughtState, expected: Option<u64>) -> Result<()> {
        thought
            .validate()
            .map_err(|_| StoreError::Invariant("invalid SharedThought"))?;
        let json = encode(thought, 65536)?;
        self.write(|tx| {
            let previous = thought_at(tx,&thought.thought_id,i64::MAX)?;
            if previous.as_ref() == Some(thought) { return Ok(()); }
            if previous.as_ref().map(|t| t.revision) != expected { return Err(StoreError::Invariant("thought compare-and-swap conflict")); }
            match previous {
                Some(ref old) => thought.validate_successor(old).map_err(|_| StoreError::Invariant("invalid thought successor"))?,
                None if thought.revision == 1 => (),
                None => return Err(StoreError::Invariant("initial thought revision must be one")),
            }
            let focus = focus_at(tx,&thought.focus_id,thought.updated_at_ms)?.ok_or(StoreError::Invariant("thought requires existing Focus"))?;
            if focus.objective_ref != thought.objective_ref { return Err(StoreError::Invariant("thought objective differs from Focus")); }
            tx.execute("INSERT INTO shared_thought_snapshots(thought_id,revision,recorded_at,focus_id,payload,hash) VALUES ($1,$2,$3,$4,$5,$6)", &[&thought.thought_id,&(thought.revision as i64),&thought.updated_at_ms,&thought.focus_id,&json,&digest(&json)])?;
            enqueue(tx,"shared_thought",&format!("{}:{}",thought.thought_id,thought.revision),&json,thought.updated_at_ms)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_remote_dns_fallback_and_schema_injection_before_connecting() {
        for host in ["example.com", "localhost", "10.0.0.1", "0.0.0.0", "::"] {
            let mut config = Config::new();
            config.host(host).user("fixture").dbname("fixture");
            assert!(matches!(
                PostgresStore::open_local(config, "archie_fixture"),
                Err(StoreError::Configuration)
            ));
        }
        let mut config = Config::new();
        config.host("127.0.0.1").user("fixture").dbname("fixture");
        assert!(validate_local_configuration(&config, "archie_fixture").is_ok());
        for schema in [
            "public",
            "archie_foo;DROP SCHEMA public",
            "archie_x\"",
            "archie_",
        ] {
            assert!(validate_local_configuration(&config, schema).is_err());
        }
        config.hostaddr("10.0.0.2".parse().unwrap());
        assert!(validate_local_configuration(&config, "archie_fixture").is_err());
    }
}
