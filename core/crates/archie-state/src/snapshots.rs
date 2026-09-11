//! Operator-only logical snapshots. Restore always creates a new database; it
//! never replaces the running node. A checksum is not a restore verification.
use crate::{Result, StateError};
use postgres::{config::Host, fallible_iterator::FallibleIterator, Config, GenericClient, NoTls};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BOUND: u64 = 512 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    connection: String,
    schema: String,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TableDigest {
    pub table: String,
    pub rows: u64,
    pub sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotManifest {
    pub version: u32,
    pub source_database: String,
    pub schema: String,
    pub captured_at_ms: u64,
    pub postgres_major: i32,
    pub extensions: Vec<(String, String)>,
    pub archive_bytes: u64,
    pub archive_sha256: String,
    pub tables: Vec<TableDigest>,
}
#[derive(Debug, Serialize)]
pub struct RestoreReceipt {
    pub database: String,
    pub archive_sha256: String,
    pub verified_at_ms: u64,
    pub tables: usize,
}
fn invalid(message: &str) -> StateError {
    StateError::Corrupt(message.into())
}
fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("invalid system time"))?
        .as_millis() as u64)
}
fn identifier(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 63
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(invalid("invalid snapshot identifier"));
    }
    Ok(format!("\"{value}\""))
}
fn settings(path: &Path) -> Result<(Config, String)> {
    if fs::metadata(path)?.len() > 16384 {
        return Err(invalid("database configuration exceeds bound"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)?.permissions().mode() & 0o027 != 0 {
            return Err(invalid(
                "operator configuration must be private and not group-writable",
            ));
        }
    }
    let settings: Settings = serde_json::from_slice(&fs::read(path)?)?;
    let mut config: Config = settings
        .connection
        .parse()
        .map_err(|_| invalid("invalid database configuration"))?;
    // Runtime uses loopback SCRAM. The ISO operator may use PostgreSQL's peer
    // authentication through its dedicated, non-public Unix socket directory.
    #[allow(unused_mut)]
    let mut validation = config.clone();
    #[cfg(unix)]
    if matches!(config.get_hosts(), [Host::Unix(path)] if path == Path::new("/run/archie-postgres"))
        && config.get_user() == Some("postgres")
    {
        validation = Config::new();
        validation.host("127.0.0.1").user("postgres").dbname(
            config
                .get_dbname()
                .ok_or_else(|| invalid("missing operator database"))?,
        );
    }
    archie_postgres::validate_local_configuration(&validation, &settings.schema)?;
    config
        .ssl_mode(postgres::config::SslMode::Disable)
        .connect_timeout(Duration::from_secs(3));
    config.options("-c statement_timeout=120000 -c lock_timeout=5000 -c idle_in_transaction_session_timeout=120000 -c timezone=UTC");
    Ok((config, settings.schema))
}
fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut f = private_file(path)?;
    f.write_all(&serde_json::to_vec_pretty(value)?)?;
    f.sync_all()?;
    Ok(())
}
fn checksum(path: &Path) -> Result<(u64, String)> {
    let mut f = File::open(path)?;
    if f.metadata()?.len() > BOUND {
        return Err(invalid("snapshot archive exceeds operator bound"));
    }
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buffer = [0; 65536];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > BOUND {
            return Err(invalid("snapshot archive exceeds operator bound"));
        }
        hash.update(&buffer[..n]);
    }
    Ok((total, format!("{:x}", hash.finalize())))
}
fn command(config: &Config, executable: &str) -> Result<Command> {
    let bin = std::env::var_os("ARCHIE_POSTGRES_BIN")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut command = Command::new(bin.join(executable));
    let host: std::ffi::OsString = match &config.get_hosts()[0] {
        Host::Tcp(host) => host.into(),
        #[cfg(unix)]
        Host::Unix(path) => path.as_os_str().into(),
    };
    // libpq connection values are environment variables, never arguments/logs.
    for name in [
        "PGSERVICE",
        "PGSERVICEFILE",
        "PGOPTIONS",
        "PGHOSTADDR",
        "PGPASSFILE",
    ] {
        command.env_remove(name);
    }
    command
        .env("PGHOST", host)
        .env(
            "PGPORT",
            config
                .get_ports()
                .first()
                .copied()
                .unwrap_or(5432)
                .to_string(),
        )
        .env(
            "PGUSER",
            config
                .get_user()
                .ok_or_else(|| invalid("missing database user"))?,
        )
        .env(
            "PGDATABASE",
            config
                .get_dbname()
                .ok_or_else(|| invalid("missing database name"))?,
        )
        .env(
            "PGPASSWORD",
            std::str::from_utf8(config.get_password().unwrap_or_default())
                .map_err(|_| invalid("invalid password encoding"))?,
        )
        .env("PGSSLMODE", "disable")
        .env("PGCONNECT_TIMEOUT", "3")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    Ok(command)
}
fn run(mut command: Command, output: Option<&Path>) -> Result<()> {
    let mut child = command.spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(invalid("PostgreSQL snapshot utility failed"))
            };
        }
        let over_budget = output.is_some_and(|path| {
            fs::metadata(path).map_or(true, |m| m.len() > BOUND)
                || fs2::available_space(path.parent().unwrap_or(path))
                    .map_or(true, |free| free < 128 * 1024 * 1024)
        });
        if start.elapsed() > TIMEOUT || over_budget {
            let _ = child.kill();
            let _ = child.wait();
            return Err(invalid("snapshot utility time/disk bound exceeded"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn extensions(client: &mut impl GenericClient) -> Result<Vec<(String, String)>> {
    client.query("SELECT extname,extversion FROM pg_extension WHERE extname IN ('timescaledb','vector') ORDER BY extname", &[])?.iter().map(|r| Ok((r.try_get(0)?,r.try_get(1)?))).collect()
}
fn fingerprints(client: &mut impl GenericClient, schema: &str) -> Result<Vec<TableDigest>> {
    let tables = client.query("SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('r','p') ORDER BY c.relname", &[&schema])?;
    if tables.is_empty() || tables.len() > 128 {
        return Err(invalid("snapshot table count outside bound"));
    }
    let start = Instant::now();
    let mut total = 0;
    let mut result = Vec::new();
    for table in tables {
        let name: String = table.try_get(0)?;
        let sql = format!(
            "SELECT row_to_json(t)::text FROM {}.{} t ORDER BY row_to_json(t)::text COLLATE \"C\"",
            identifier(schema)?,
            identifier(&name)?
        );
        let mut rows = client.query_raw(
            &sql,
            std::iter::empty::<&(dyn postgres::types::ToSql + Sync)>(),
        )?;
        let mut hash = Sha256::new();
        let mut count = 0;
        while let Some(row) = rows.next()? {
            let json: String = row.try_get(0)?;
            count += 1;
            total += json.len() as u64;
            if count > 1_000_000 || total > BOUND || start.elapsed() > TIMEOUT {
                return Err(invalid("snapshot verification exceeds bound"));
            }
            hash.update((json.len() as u64).to_be_bytes());
            hash.update(json.as_bytes());
        }
        result.push(TableDigest {
            table: name,
            rows: count,
            sha256: format!("{:x}", hash.finalize()),
        });
    }
    Ok(result)
}
/// Capture one MVCC snapshot and the full Timescale catalogs. Runs synchronously
/// as an operator operation; call outside an async executor.
pub fn capture(config_path: &Path, directory: &Path) -> Result<SnapshotManifest> {
    let (config, schema) = settings(config_path)?;
    let source = config
        .get_dbname()
        .ok_or_else(|| invalid("missing database"))?
        .to_owned();
    if matches!(source.as_str(), "postgres" | "template0" | "template1") {
        return Err(invalid(
            "snapshot requires a dedicated application database",
        ));
    }
    let mut client = config.connect(NoTls)?;
    let mut tx = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()?;
    let major: i32 = tx
        .query_one(
            "SELECT current_setting('server_version_num')::integer / 10000",
            &[],
        )?
        .try_get(0)?;
    let installed = extensions(&mut tx)?;
    if major != 18
        || installed
            != vec![
                ("timescaledb".into(), "2.29.2".into()),
                ("vector".into(), "0.8.6".into()),
            ]
    {
        return Err(invalid("unsupported snapshot database versions"));
    }
    let snapshot: String = tx
        .query_one("SELECT pg_export_snapshot()", &[])?
        .try_get(0)?;
    let tables = fingerprints(&mut tx, &schema)?;
    #[allow(unused_mut)]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?; // No overwrite, including partial prior attempts.
    if fs2::available_space(directory)? < BOUND + 128 * 1024 * 1024 {
        return Err(invalid("insufficient snapshot disk reserve"));
    }
    let archive = directory.join("database.dump");
    let file = private_file(&archive)?;
    let mut dump = command(&config, "pg_dump")?;
    dump.args([
        "--format=custom",
        "--no-owner",
        "--no-acl",
        "--no-password",
        "--snapshot",
        &snapshot,
    ])
    .stdout(file.try_clone()?);
    run(dump, Some(&archive))?;
    file.sync_all()?;
    tx.commit()?;
    let (archive_bytes, archive_sha256) = checksum(&archive)?;
    let manifest = SnapshotManifest {
        version: 1,
        source_database: source,
        schema,
        captured_at_ms: now()?,
        postgres_major: major,
        extensions: installed,
        archive_bytes,
        archive_sha256,
        tables,
    };
    write_json(&directory.join("manifest.json"), &manifest)?;
    let payload = serde_json::to_string(&manifest)?;
    let hash = format!("{:x}", Sha256::digest(payload.as_bytes()));
    client.execute(&format!("INSERT INTO {}.database_snapshots(snapshot_id,captured_at,manifest_json,manifest_sha256) VALUES($1,$2,$3,$4)", identifier(&manifest.schema)?), &[&manifest.archive_sha256, &(manifest.captured_at_ms as i64), &payload, &hash])?;
    Ok(manifest)
}
/// Restore only to a fresh generated database and verify every application table.
/// The database is retained for inspection/cutover. Failures retain the isolated
/// target but never change the source. Use only operator-created trusted archives:
/// pg_restore executes SQL; checksums detect damage, not a malicious archive.
pub fn restore_verify(config_path: &Path, directory: &Path) -> Result<RestoreReceipt> {
    let (mut config, schema) = settings(config_path)?;
    let manifest_path = directory.join("manifest.json");
    if fs::metadata(&manifest_path)?.len() > 128 * 1024 {
        return Err(invalid("snapshot manifest exceeds bound"));
    }
    let manifest: SnapshotManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    if manifest.version != 1
        || manifest.postgres_major != 18
        || manifest.schema != schema
        || manifest.tables.is_empty()
        || manifest.tables.len() > 128
    {
        return Err(invalid("unsupported snapshot manifest"));
    }
    let archive = directory.join("database.dump");
    if checksum(&archive)? != (manifest.archive_bytes, manifest.archive_sha256.clone()) {
        return Err(invalid("snapshot archive checksum mismatch"));
    }
    // Reserve the receipt before creating anything in PostgreSQL.
    let mut receipt_file = private_file(&directory.join("verification.json"))?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let database = format!(
        "archie_restore_{}_{}_{}",
        now()?,
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    config.dbname("postgres");
    let mut admin = config.connect(NoTls)?;
    admin.batch_execute(&format!(
        "CREATE DATABASE {} TEMPLATE template0",
        identifier(&database)?
    ))?;
    config.dbname(&database);
    let mut target = config.connect(NoTls)?;
    target.batch_execute("CREATE EXTENSION timescaledb VERSION '2.29.2'; CREATE EXTENSION vector VERSION '0.8.6'; SELECT public.timescaledb_pre_restore();")?;
    let mut restore = command(&config, "pg_restore")?;
    restore
        .args([
            "--exit-on-error",
            "--no-owner",
            "--no-acl",
            "--no-password",
            "--dbname",
            &database,
        ])
        .arg(&archive)
        .stdout(Stdio::null());
    let restored = run(restore, None);
    // Always leave restoring mode, even when pg_restore failed.
    let finalized = target.batch_execute("SELECT public.timescaledb_post_restore()");
    restored?;
    finalized?;
    let mut tx = target
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()?;
    if extensions(&mut tx)? != manifest.extensions
        || fingerprints(&mut tx, &schema)? != manifest.tables
    {
        return Err(invalid("restored database does not match snapshot"));
    }
    tx.commit()?;
    let receipt = RestoreReceipt {
        database,
        archive_sha256: manifest.archive_sha256,
        verified_at_ms: now()?,
        tables: manifest.tables.len(),
    };
    receipt_file.write_all(&serde_json::to_vec_pretty(&receipt)?)?;
    receipt_file.sync_all()?;
    Ok(receipt)
}
