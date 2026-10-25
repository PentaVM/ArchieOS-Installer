//! Fixed loopback read-only projection. No command token, provider URLs or Core writes.
use archie_worldd::exploration::ExplorationSnapshot;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn current() -> ExplorationSnapshot {
    tokio::time::timeout(std::time::Duration::from_millis(1500), fetch())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| ExplorationSnapshot::unavailable("unavailable"))
}

async fn fetch() -> Result<ExplorationSnapshot, ()> {
    let token = archie_worldd::exploration_projection::read_token(
        &archie_worldd::exploration_projection::default_runtime_dir(),
    )
    .map_err(|_| ())?;
    let mut stream = tokio::net::TcpStream::connect("127.0.0.1:7667")
        .await
        .map_err(|_| ())?;
    stream
        .write_all(format!("GET /v1/exploration HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .map_err(|_| ())?;
    let mut bytes = Vec::new();
    stream
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| ())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(());
    }
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").ok_or(())?;
    let head = std::str::from_utf8(&bytes[..split]).map_err(|_| ())?;
    if head.lines().next().is_none_or(|s| s != "HTTP/1.1 200 OK") {
        return Err(());
    }
    let view: ExplorationSnapshot = serde_json::from_slice(&bytes[split + 4..]).map_err(|_| ())?;
    validate(&view)?;
    Ok(view)
}

fn validate(view: &ExplorationSnapshot) -> Result<(), ()> {
    if view.schema_version != 1
        || !["live", "disabled", "unavailable"].contains(&view.source.as_str())
        || view.jobs.len() > 32
        || view.providers.len() > 16
    {
        return Err(());
    }
    for job in &view.jobs {
        if job.domain != view.domain
            || job.request.domain != view.domain
            || job.id.len() != 32
            || !job.id.bytes().all(|b| b.is_ascii_hexdigit())
            || archie_geofabric::exploration::plan(&job.request).is_err()
            || job.artifacts.len() > 8194
        {
            return Err(());
        }
        if let Some(p) = &job.preview {
            let terrain = &p["terrain"];
            if !terrain.is_null() {
                let w = terrain["width"].as_u64().ok_or(())?;
                let h = terrain["height"].as_u64().ok_or(())?;
                if w == 0
                    || h == 0
                    || w > 33
                    || h > 33
                    || terrain["elevations_m"].as_array().ok_or(())?.len() as u64 != w * h
                {
                    return Err(());
                }
            }
            let buildings = p["buildings"].as_array().ok_or(())?;
            if buildings.len() > 128
                || buildings
                    .iter()
                    .any(|b| b["ring"].as_array().is_none_or(|ring| ring.len() > 2048))
            {
                return Err(());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_foreign_schema_and_unbounded_projection() {
        let mut v = ExplorationSnapshot::unavailable("unavailable");
        assert!(validate(&v).is_ok());
        v.schema_version = 2;
        assert!(validate(&v).is_err());
    }
}
