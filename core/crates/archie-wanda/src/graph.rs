use crate::{config::WandaConfig, types::PersonProjection, WandaError};
use archie_vault::ContextDomain;

pub async fn project(
    config: &WandaConfig,
    domain: &ContextDomain,
) -> Result<PersonProjection, WandaError> {
    use std::{process::Stdio, time::Duration};
    use tokio::io::AsyncReadExt;
    let binding = config
        .domains
        .iter()
        .find(|b| &b.domain == domain)
        .ok_or(WandaError::UnknownDomain)?;
    if !config.graph_binary.is_absolute() || !binding.graph_path.is_absolute() {
        return Err(WandaError::GraphUnavailable);
    }
    // Only the trusted config selects paths. The existing CLI loads, validates
    // and projects the graph; Wanda never implements an alternative TSV parser.
    let mut child = tokio::process::Command::new(&config.graph_binary)
        .args(["export-json", "--all"])
        .env("ARCHIE_SENTRY_GRAPH", &binding.graph_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| WandaError::GraphUnavailable)?;
    let stdout = child.stdout.take().ok_or(WandaError::GraphUnavailable)?;
    let output = tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        stdout
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| WandaError::GraphUnavailable)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(WandaError::TooLarge);
        }
        if !child
            .wait()
            .await
            .map_err(|_| WandaError::GraphUnavailable)?
            .success()
        {
            return Err(WandaError::GraphUnavailable);
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| WandaError::GraphUnavailable)??;
    let graph: PersonProjection =
        serde_json::from_slice(&output).map_err(|_| WandaError::GraphUnavailable)?;
    if graph.schema_version != 1 || graph.nodes.len() > 4096 || graph.edges.len() > 16384 {
        return Err(WandaError::GraphUnavailable);
    }
    Ok(graph)
}
