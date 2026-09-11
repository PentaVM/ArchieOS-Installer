//! Native verified compressed-map cache. No embedded database or interpreter.
use crate::mapdata::{safe_mirror, safe_source};
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const RESERVE: u64 = 256 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub source: String,
    pub mirror: String,
    pub archive: PathBuf,
    pub format: String,
    pub bytes: u64,
    pub md5: String,
    pub sha256: String,
    pub attribution: String,
    pub license: String,
}
fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("cache symlink refused".into());
    }
    Ok(())
}
fn digest(path: &Path) -> Result<(String, String)> {
    let mut input = File::open(path)?;
    let mut md5 = Md5::new();
    let mut sha = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        md5.update(&buffer[..n]);
        sha.update(&buffer[..n]);
    }
    Ok((hex::encode(md5.finalize()), hex::encode(sha.finalize())))
}
fn cache_hit(source: &str, archive: &Path, manifest: &Path) -> Result<Option<Manifest>> {
    if archive.exists() != manifest.exists() {
        return Err("unmanaged or incomplete existing package; inspect before replacing".into());
    }
    if !archive.exists() {
        return Ok(None);
    }
    if fs::metadata(manifest)?.len() > 16384 {
        return Err("map manifest exceeds bound".into());
    }
    let saved: Manifest = serde_json::from_slice(&fs::read(manifest)?)?;
    // The same filesystem is mounted under /mnt during installation and at its
    // durable path after boot. Bind to source + filename, not the temporary mount.
    if saved.version != 1
        || saved.source != source
        || saved.archive.file_name() != archive.file_name()
    {
        return Err("cache belongs to another package".into());
    }
    let (md5, sha) = digest(archive)?;
    if fs::metadata(archive)?.len() == saved.bytes && sha == saved.sha256 && md5 == saved.md5 {
        Ok(Some(saved))
    } else {
        Ok(None)
    }
}
/// Read-only preflight for a package on an already-mounted, retained map volume.
pub fn inspect_cache(source: &str, directory: &Path) -> Result<Option<Manifest>> {
    if !safe_source(source) || !directory.is_absolute() {
        return Err("invalid cache source or directory".into());
    }
    let filename = source.rsplit('/').next().ok_or("missing filename")?;
    let archive = directory.join(filename);
    let manifest = directory.join(format!("{filename}.json"));
    for path in [directory, &archive, &manifest] {
        reject_symlink(path)?;
    }
    cache_hit(source, &archive, &manifest)
}
fn response_extent(
    status: reqwest::StatusCode,
    range: Option<&str>,
    length: Option<u64>,
    offset: u64,
) -> Result<(u64, bool)> {
    if status == reqwest::StatusCode::OK {
        let total = length.ok_or("server did not report package size")?;
        if total == 0 || total > 1024u64.pow(4) {
            return Err("invalid package size".into());
        }
        return Ok((total, false));
    }
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err("unexpected package response".into());
    }
    let value = range
        .and_then(|r| r.strip_prefix("bytes "))
        .ok_or("missing byte range")?;
    let (span, total) = value.split_once('/').ok_or("invalid range")?;
    let (start, end) = span.split_once('-').ok_or("invalid range")?;
    let (start, end, total) = (
        start.parse::<u64>()?,
        end.parse::<u64>()?,
        total.parse::<u64>()?,
    );
    if start != offset || total <= offset || total > 1024u64.pow(4) || end != total - 1 {
        return Err("resumed range does not match partial file".into());
    }
    Ok((total, true))
}
async fn one_source(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    expected: &str,
) -> Result<(u64, String)> {
    reject_symlink(part)?;
    let offset = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    // A complete partial file can survive a power loss before publication.
    if offset > 0 {
        let (md5, sha) = digest(part)?;
        if md5 == expected {
            return Ok((offset, sha));
        }
    }
    let mut request = client.get(url).header("Accept-Encoding", "identity");
    if offset > 0 {
        request = request.header("Range", format!("bytes={offset}-"))
    }
    let mut response = request.send().await?.error_for_status()?;
    let (total, resume) = response_extent(
        response.status(),
        response
            .headers()
            .get("content-range")
            .and_then(|h| h.to_str().ok()),
        response.content_length(),
        offset,
    )?;
    let start = if resume { offset } else { 0 };
    if fs2::available_space(part.parent().ok_or("no cache parent")?)? < total - start + RESERVE {
        return Err("not enough free space on map filesystem".into());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume)
        .truncate(!resume)
        .open(part)?;
    let mut done = start;
    while let Some(chunk) = response.chunk().await? {
        done = done
            .checked_add(chunk.len() as u64)
            .ok_or("package size overflow")?;
        if done > total {
            return Err("server exceeded declared package size".into());
        }
        file.write_all(&chunk)?;
        println!("PROGRESS {done} {total}");
    }
    file.sync_all()?;
    drop(file);
    if done != total {
        return Err("incomplete package; partial file retained".into());
    }
    let (md5, sha) = digest(part)?;
    if md5 != expected {
        let evidence = part.with_extension(format!("invalid-{}", &sha[..16]));
        if !evidence.exists() {
            fs::rename(part, evidence)?;
        }
        return Err("package checksum mismatch".into());
    }
    Ok((total, sha))
}
pub async fn fetch(
    source: &str,
    directory: &Path,
    mirrors: &[String],
    refresh: bool,
) -> Result<Manifest> {
    if !safe_source(source) || mirrors.len() > 8 || mirrors.iter().any(|s| !safe_mirror(s)) {
        return Err("invalid map source or mirror".into());
    }
    if !directory.is_absolute() {
        return Err("absolute cache directory required".into());
    }
    reject_symlink(directory)?;
    fs::create_dir_all(directory)?;
    let filename = source.rsplit('/').next().ok_or("missing filename")?;
    let archive = directory.join(filename);
    let manifest = directory.join(format!("{filename}.json"));
    reject_symlink(&archive)?;
    reject_symlink(&manifest)?;
    let lock_path = directory.join(format!("{filename}.lock"));
    reject_symlink(&lock_path)?;
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    fs2::FileExt::try_lock_exclusive(&lock_file)
        .map_err(|_| "another download owns this map cache")?;
    // The OS releases this lock even on termination; a stale marker never blocks resume.
    let pending = directory.join(format!("{filename}.json.new"));
    reject_symlink(&pending)?;
    if pending.exists() && archive.exists() && cache_hit(source, &archive, &pending)?.is_some() {
        fs::rename(&pending, &manifest)?;
    }

    let cached = cache_hit(source, &archive, &manifest)?;
    if !refresh {
        if let Some(saved) = cached {
            println!("CACHE_HIT {}", archive.display());
            return Ok(saved);
        }
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(45))
        .build()?;
    let mut checksum_response = client
        .get(format!("{source}.md5"))
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = checksum_response.chunk().await? {
        if bytes.len() + chunk.len() > 512 {
            return Err("checksum document exceeds bound".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let checksum = std::str::from_utf8(&bytes)?;
    let fields: Vec<_> = checksum.split_whitespace().collect();
    if fields.len() != 2
        || fields[0].len() != 32
        || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
        || fields[1].trim_start_matches('*') != filename
    {
        return Err("unexpected source checksum document".into());
    }
    let expected = fields[0].to_ascii_lowercase();
    let part = directory.join(format!("{filename}.{expected}.part"));
    for candidate in mirrors
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(source))
    {
        let Ok((size, sha)) = one_source(&client, candidate, &part, &expected).await else {
            eprintln!("Source unavailable or invalid; trying the next source.");
            continue;
        };
        let saved = Manifest {
            version: 1,
            source: source.into(),
            mirror: candidate.into(),
            archive: PathBuf::from(filename),
            format: "osm.pbf".into(),
            bytes: size,
            md5: expected.clone(),
            sha256: sha,
            attribution: "OpenStreetMap contributors / Geofabrik".into(),
            license: "ODbL-1.0".into(),
        };
        let pending = directory.join(format!("{filename}.json.new"));
        if pending.exists() {
            // A crash before publication left the old archive intact. Preserve evidence
            // and ask for inspection rather than overwrite an unexplained journal.
            return Err("unfinished publication journal requires inspection".into());
        }
        let mut metadata = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)?;
        metadata.write_all(&serde_json::to_vec_pretty(&saved)?)?;
        metadata.sync_all()?;
        drop(metadata);
        // rename replaces a managed destination atomically on Windows and Unix.
        // The pending manifest allows recovery if power fails between the two renames.
        fs::rename(&part, &archive)?;
        fs::rename(&pending, &manifest)?;
        println!("READY {}", archive.display());
        return Ok(saved);
    }
    Err("all map sources failed; verified archive and partial data retained".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_refuse_offset_confusion_and_unknown_lengths() {
        assert!(response_extent(
            reqwest::StatusCode::PARTIAL_CONTENT,
            Some("bytes 50-99/100"),
            None,
            50
        )
        .is_ok());
        assert!(response_extent(
            reqwest::StatusCode::PARTIAL_CONTENT,
            Some("bytes 0-99/100"),
            None,
            50
        )
        .is_err());
        assert!(response_extent(
            reqwest::StatusCode::PARTIAL_CONTENT,
            Some("bytes 50-98/100"),
            None,
            50
        )
        .is_err());
        assert!(response_extent(reqwest::StatusCode::OK, None, None, 0).is_err());
    }
    #[tokio::test]
    async fn native_download_checks_bytes_and_then_reuses_cache_without_network() {
        use std::net::TcpListener;
        let data = b"native compressed map fixture".repeat(100);
        let expected = hex::encode(Md5::digest(&data));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let served = data.clone();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            assert!(socket.read(&mut request).unwrap() > 0);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                served.len()
            )
            .unwrap();
            socket.write_all(&served).unwrap();
        });
        let directory = std::env::temp_dir().join(format!(
            "archie-native-map-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let archive = directory.join("fixture.osm.pbf");
        let (_, sha) = one_source(
            &reqwest::Client::new(),
            &format!("http://{address}"),
            &archive,
            &expected,
        )
        .await
        .unwrap();
        thread.join().unwrap();
        assert_eq!(fs::read(&archive).unwrap(), data);
        let manifest = directory.join("fixture.osm.pbf.json");
        let source = "https://download.geofabrik.de/fixture.osm.pbf";
        let saved = Manifest {
            version: 1,
            source: source.into(),
            mirror: source.into(),
            archive: archive.clone(),
            format: "osm.pbf".into(),
            bytes: data.len() as u64,
            md5: expected,
            sha256: sha,
            attribution: "fixture".into(),
            license: "fixture".into(),
        };
        fs::write(&manifest, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(cache_hit(source, &archive, &manifest).unwrap().is_some());
        // Public fetch returns before any HTTP request, even with no fixture server.
        assert!(fetch(source, &directory, &[], false).await.is_ok());
        let relocated = directory.join("installed-mount");
        fs::create_dir(&relocated).unwrap();
        let moved_archive = relocated.join("fixture.osm.pbf");
        let moved_manifest = relocated.join("fixture.osm.pbf.json");
        fs::rename(&archive, &moved_archive).unwrap();
        fs::rename(&manifest, &moved_manifest).unwrap();
        assert!(fetch(source, &relocated, &[], false).await.is_ok());
        assert!(inspect_cache(source, &relocated).unwrap().is_some());
        fs::rename(&moved_archive, &archive).unwrap();
        fs::rename(&moved_manifest, &manifest).unwrap();
        fs::write(&archive, b"corrupt").unwrap();
        assert!(cache_hit(source, &archive, &manifest).unwrap().is_none());
    }

    async fn serve_partial_fixture(mode: &str) {
        use std::net::TcpListener;
        let data = b"resumable compressed map fixture".repeat(100);
        let expected = hex::encode(Md5::digest(&data));
        let directory = std::env::temp_dir().join(format!(
            "archie-map-{mode}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let part = directory.join("map.part");
        let offset = 100;
        fs::write(&part, &data[..offset]).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let served = data.clone();
        let mode = mode.to_owned();
        let response_mode = mode.clone();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            assert!(String::from_utf8(request)
                .unwrap()
                .to_lowercase()
                .contains("range: bytes=100-"));
            write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                served.len()-offset, offset, served.len()-1, served.len()).unwrap();
            match response_mode.as_str() {
                "truncated" => socket.write_all(&served[offset..offset + 50]).unwrap(),
                "corrupt" => socket
                    .write_all(&vec![b'x'; served.len() - offset])
                    .unwrap(),
                _ => socket.write_all(&served[offset..]).unwrap(),
            }
        });
        let result = one_source(
            &reqwest::Client::new(),
            &format!("http://{address}"),
            &part,
            &expected,
        )
        .await;
        thread.join().unwrap();
        match mode.as_str() {
            "truncated" => {
                assert!(result.is_err());
                let bytes = fs::read(&part).unwrap();
                assert!(bytes.len() >= offset && bytes.len() < data.len());
                assert_eq!(bytes, data[..bytes.len()]);
            }
            "corrupt" => {
                assert!(result.is_err());
                assert!(!part.exists());
                assert!(fs::read_dir(&directory).unwrap().any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("invalid-")));
            }
            _ => {
                assert_eq!(result.unwrap().0, data.len() as u64);
                assert_eq!(fs::read(&part).unwrap(), data);
            }
        }
    }
    #[tokio::test]
    async fn resumes_only_missing_bytes() {
        serve_partial_fixture("resume").await;
    }
    #[tokio::test]
    async fn truncated_transfer_keeps_reusable_prefix() {
        serve_partial_fixture("truncated").await;
    }
    #[tokio::test]
    async fn corrupt_mirror_is_quarantined() {
        serve_partial_fixture("corrupt").await;
    }
}
