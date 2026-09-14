//! Downloading an authorised artifact, and believing nothing about it until it verifies.
//!
//! The size is checked as the bytes stream, so a server that sends more than was declared
//! is cut off rather than allowed to fill the disk. The digest is checked at the end, and a
//! file that fails either check is removed: a half-written or wrong artifact never sits
//! where an install could pick it up. Redirects are followed only to HTTPS hosts the catalog
//! names, and only a few times.

use crate::gate::AuthorizedInstall;
use futures_util::StreamExt;
use reqwest::redirect::Policy;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const MAX_REDIRECTS: usize = 5;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("could not reach the artifact: {0}")]
    Http(#[from] reqwest::Error),
    #[error("the server answered {0}")]
    Status(u16),
    #[error("the download was redirected somewhere the catalog does not allow")]
    RedirectRefused,
    #[error("the server announced {announced} bytes; the catalog declares {declared}")]
    AnnouncedSize { announced: u64, declared: u64 },
    #[error("the server sent more than the {0} bytes the catalog declares")]
    TooLarge(u64),
    #[error("the download ended at {received} bytes; the catalog declares {declared}")]
    Truncated { received: u64, declared: u64 },
    #[error("the artifact's SHA-256 does not match the catalog")]
    Digest,
    #[error("writing the download: {0}")]
    Io(#[from] std::io::Error),
}

impl FetchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Http(_) => "fetch_unreachable",
            Self::Status(_) => "fetch_status",
            Self::RedirectRefused => "fetch_redirect_refused",
            Self::AnnouncedSize { .. } | Self::TooLarge(_) | Self::Truncated { .. } => {
                "fetch_size_mismatch"
            }
            Self::Digest => "fetch_digest_mismatch",
            Self::Io(_) => "fetch_io",
        }
    }
}

/// Downloads `install`'s artifact to `dest`, which must not exist. On any error after the
/// file is created, it is removed again; a file that was already there is never touched.
pub async fn download(
    install: &AuthorizedInstall,
    dest: &Path,
    timeout: Duration,
) -> Result<(), FetchError> {
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .await?;
    let result = download_into(install, file, timeout).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

async fn download_into(
    install: &AuthorizedInstall,
    mut file: tokio::fs::File,
    timeout: Duration,
) -> Result<(), FetchError> {
    let artifact = install.artifact();
    let hosts: Arc<Vec<String>> = Arc::new(install.redirect_hosts().to_vec());
    let policy = Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.stop();
        }
        let url = attempt.url();
        let allowed = url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| hosts.iter().any(|allowed| allowed == host));
        if allowed {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });
    let client = Client::builder()
        .redirect(policy)
        .connect_timeout(Duration::from_secs(20))
        .timeout(timeout)
        .build()?;

    let response = client.get(&artifact.url).send().await?;
    let status = response.status();
    if status.is_redirection() {
        return Err(FetchError::RedirectRefused);
    }
    if !status.is_success() {
        return Err(FetchError::Status(status.as_u16()));
    }
    if let Some(announced) = response.content_length() {
        if announced != artifact.size {
            return Err(FetchError::AnnouncedSize {
                announced,
                declared: artifact.size,
            });
        }
    }

    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        received += chunk.len() as u64;
        if received > artifact.size {
            return Err(FetchError::TooLarge(artifact.size));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    if received != artifact.size {
        return Err(FetchError::Truncated {
            received,
            declared: artifact.size,
        });
    }
    if hex::encode(hasher.finalize()) != artifact.sha256 {
        return Err(FetchError::Digest);
    }
    Ok(())
}
