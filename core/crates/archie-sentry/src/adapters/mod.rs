//! Native, bounded LAN protocols. Tokens and raw provider bodies never enter receipts.
pub mod home_assistant;
pub mod ipp;

use crate::engine::AdapterError;
use reqwest::{Client, Response, Url};
use std::{net::IpAddr, time::Duration};
const MAX_BODY: usize = 1_048_576;

fn endpoint(input: &str, credentials: bool) -> Result<Url, AdapterError> {
    let url = Url::parse(input).map_err(|_| AdapterError::Protocol)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AdapterError::Protocol);
    }
    let host = url
        .host_str()
        .ok_or(AdapterError::Protocol)?
        .trim_matches(['[', ']']);
    let ip: IpAddr = host.parse().map_err(|_| AdapterError::Protocol)?;
    let permitted = match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
    };
    if !permitted || (credentials && url.scheme() == "http" && !ip.is_loopback()) {
        return Err(AdapterError::Protocol);
    }
    Ok(url)
}
fn client() -> Result<Client, AdapterError> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| AdapterError::Transport)
}
async fn body(mut response: Response, content_type: &str) -> Result<Vec<u8>, AdapterError> {
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(AdapterError::Authentication);
    }
    if !response.status().is_success() {
        return Err(AdapterError::Unavailable);
    }
    let actual = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|s| s.split(';').next())
        .unwrap_or("")
        .trim();
    if actual != content_type
        || response
            .content_length()
            .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err(AdapterError::Protocol);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AdapterError::Transport)?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err(AdapterError::Protocol);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
