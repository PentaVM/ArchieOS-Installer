//! UPnP IGD `WANIPConnection:1` over SOAP 1.1, the credential-free channel:
//! AddPortMapping / DeletePortMapping / GetSpecificPortMappingEntry /
//! GetStatusInfo / GetGenericPortMappingEntry (bounded enumeration).
//!
//! Parsing is strict: responses must carry the matching `*Response` element,
//! every extracted leaf is namespace-prefix tolerant but unique in the
//! document, values reject control characters and unknown entities, and a
//! returned mapping whose port or protocol differs from the request is
//! refused. SOAP faults map to [`AdapterError::Protocol`], except the two
//! standard "no such entry" codes (714 for a specific mapping, 713 to end an
//! enumeration), which are answers, not errors. HTTP 401/403 map to
//! [`AdapterError::Authentication`]; a non-success status without a SOAP
//! fault maps to [`AdapterError::Unavailable`]. Real gateways answer faults
//! with HTTP 500, so the body is still parsed when the status is an error.

use crate::{
    adapters::{client, endpoint, MAX_BODY},
    contracts::PortProtocol,
    engine::AdapterError,
};
use reqwest::{Client, Response, Url};
use std::net::Ipv4Addr;

/// Service type every action here is namespaced under.
pub const SERVICE_TYPE: &str = "urn:schemas-upnp-org:service:WANIPConnection:1";
/// Enumeration via GetGenericPortMappingEntry never walks past this index.
pub const MAX_MAPPINGS: u16 = 256;
/// UPnP error: no entry exists for the requested (port, protocol).
const NO_SUCH_ENTRY: u16 = 714;
/// UPnP error: enumeration index beyond the end of the table.
const INVALID_INDEX: u16 = 713;

/// A port mapping exactly as the router reported it (from
/// GetGenericPortMappingEntry, which echoes every field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingEntry {
    pub external_port: u16,
    pub protocol: PortProtocol,
    pub internal_port: u16,
    pub internal_client: Ipv4Addr,
    pub enabled: bool,
    pub description: Option<String>,
}

/// The answer to GetSpecificPortMappingEntry. That response does NOT echo
/// the external port or protocol (they are input arguments); identity comes
/// from the query itself, which is exactly the postcondition semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecificEntry {
    pub internal_port: u16,
    pub internal_client: Ipv4Addr,
    pub enabled: bool,
    pub description: Option<String>,
}

/// What GetStatusInfo said; `connected` is the postcondition-relevant fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusInfo {
    pub connected: bool,
    pub uptime_seconds: u64,
}

enum Outcome {
    Ok(String),
    /// Fault with the UPnP `errorCode` when the detail carried one.
    Fault(Option<u16>),
}

/// One WANIPConnection control endpoint. No credentials: IGD port-mapping
/// control on this class of device is unauthenticated by design.
pub struct IgdClient {
    url: Url,
    client: Client,
}
impl std::fmt::Debug for IgdClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IgdClient")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}
impl IgdClient {
    /// `control_url` is the WANIPConnection control URL (from SSDP discovery;
    /// discovery itself is out of scope here). The shared endpoint policy
    /// applies: private or loopback numeric IP, no credentials, no query.
    pub fn new(control_url: &str) -> Result<Self, AdapterError> {
        let url = endpoint(control_url, false)?;
        if url.path().len() > 512 {
            return Err(AdapterError::Protocol);
        }
        Ok(Self {
            url,
            client: client()?,
        })
    }
    async fn call(&self, action: &str, inner: &str) -> Result<Outcome, AdapterError> {
        let response = self
            .client
            .post(self.url.clone())
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .header("SOAPAction", format!("\"{SERVICE_TYPE}#{action}\""))
            .body(envelope(inner))
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        let (text, ok) = soap_body(response).await?;
        match fault_code(&text)? {
            Some(code) => Ok(Outcome::Fault(code)),
            None if ok => Ok(Outcome::Ok(text)),
            None => Err(AdapterError::Unavailable),
        }
    }
    /// Some(entry) when the router lists a mapping for exactly this
    /// (external port, protocol); None when it says 714 NoSuchEntryInArray.
    pub async fn get_specific_port_mapping_entry(
        &self,
        external_port: u16,
        protocol: PortProtocol,
    ) -> Result<Option<SpecificEntry>, AdapterError> {
        if external_port == 0 {
            return Err(AdapterError::Protocol);
        }
        let inner = format!(
            "<u:GetSpecificPortMappingEntry xmlns:u=\"{SERVICE_TYPE}\">\
             <NewRemoteHost></NewRemoteHost>\
             <NewExternalPort>{external_port}</NewExternalPort>\
             <NewProtocol>{}</NewProtocol>\
             </u:GetSpecificPortMappingEntry>",
            protocol.as_str()
        );
        match self.call("GetSpecificPortMappingEntry", &inner).await? {
            Outcome::Fault(Some(NO_SUCH_ENTRY)) => Ok(None),
            Outcome::Fault(_) => Err(AdapterError::Protocol),
            Outcome::Ok(text) => {
                if !has_element(&text, "GetSpecificPortMappingEntryResponse") {
                    return Err(AdapterError::Protocol);
                }
                Ok(Some(parse_specific_entry(&text)?))
            }
        }
    }
    /// Bounded table walk via GetGenericPortMappingEntry; 713 ends the walk.
    pub async fn list_port_mappings(&self) -> Result<Vec<MappingEntry>, AdapterError> {
        let mut entries = Vec::new();
        for index in 0..MAX_MAPPINGS {
            let inner = format!(
                "<u:GetGenericPortMappingEntry xmlns:u=\"{SERVICE_TYPE}\">\
                 <NewPortMappingIndex>{index}</NewPortMappingIndex>\
                 </u:GetGenericPortMappingEntry>"
            );
            match self.call("GetGenericPortMappingEntry", &inner).await? {
                Outcome::Fault(Some(INVALID_INDEX)) => break,
                Outcome::Fault(_) => return Err(AdapterError::Protocol),
                Outcome::Ok(text) => {
                    if !has_element(&text, "GetGenericPortMappingEntryResponse") {
                        return Err(AdapterError::Protocol);
                    }
                    entries.push(parse_mapping_entry(&text)?);
                }
            }
        }
        Ok(entries)
    }
    pub async fn add_port_mapping(
        &self,
        external_port: u16,
        protocol: PortProtocol,
        internal_port: u16,
        internal_client: Ipv4Addr,
        description: &str,
        lease_seconds: u32,
    ) -> Result<(), AdapterError> {
        if external_port == 0
            || internal_port == 0
            || !(internal_client.is_private() || internal_client.is_loopback())
            || description.is_empty()
            || description.len() > 64
            || description.bytes().any(|b| b < 0x20 || b == 0x7f)
            || description.contains(['<', '>', '&'])
        {
            return Err(AdapterError::Protocol);
        }
        let inner = format!(
            "<u:AddPortMapping xmlns:u=\"{SERVICE_TYPE}\">\
             <NewRemoteHost></NewRemoteHost>\
             <NewExternalPort>{external_port}</NewExternalPort>\
             <NewProtocol>{}</NewProtocol>\
             <NewInternalPort>{internal_port}</NewInternalPort>\
             <NewInternalClient>{internal_client}</NewInternalClient>\
             <NewEnabled>1</NewEnabled>\
             <NewPortMappingDescription>{description}</NewPortMappingDescription>\
             <NewLeaseDuration>{lease_seconds}</NewLeaseDuration>\
             </u:AddPortMapping>",
            protocol.as_str()
        );
        match self.call("AddPortMapping", &inner).await? {
            Outcome::Ok(text) if has_element(&text, "AddPortMappingResponse") => Ok(()),
            Outcome::Ok(_) => Err(AdapterError::Protocol),
            Outcome::Fault(_) => Err(AdapterError::Protocol),
        }
    }
    pub async fn delete_port_mapping(
        &self,
        external_port: u16,
        protocol: PortProtocol,
    ) -> Result<(), AdapterError> {
        if external_port == 0 {
            return Err(AdapterError::Protocol);
        }
        let inner = format!(
            "<u:DeletePortMapping xmlns:u=\"{SERVICE_TYPE}\">\
             <NewRemoteHost></NewRemoteHost>\
             <NewExternalPort>{external_port}</NewExternalPort>\
             <NewProtocol>{}</NewProtocol>\
             </u:DeletePortMapping>",
            protocol.as_str()
        );
        match self.call("DeletePortMapping", &inner).await? {
            Outcome::Ok(text) if has_element(&text, "DeletePortMappingResponse") => Ok(()),
            Outcome::Ok(_) => Err(AdapterError::Protocol),
            Outcome::Fault(_) => Err(AdapterError::Protocol),
        }
    }
    /// Readiness probe and honest link-state read.
    pub async fn get_status_info(&self) -> Result<StatusInfo, AdapterError> {
        let inner = format!("<u:GetStatusInfo xmlns:u=\"{SERVICE_TYPE}\"></u:GetStatusInfo>");
        match self.call("GetStatusInfo", &inner).await? {
            Outcome::Fault(_) => Err(AdapterError::Protocol),
            Outcome::Ok(text) => {
                if !has_element(&text, "GetStatusInfoResponse") {
                    return Err(AdapterError::Protocol);
                }
                let status = leaf_required(&text, "NewConnectionStatus")?;
                let connected = match status.as_str() {
                    "Connected" => true,
                    // Known non-connected states are a valid "no".
                    "Disconnected" | "Connecting" | "Disconnecting"
                    | "Authenticating" | "Unconfigured" => false,
                    _ => return Err(AdapterError::Protocol),
                };
                let uptime_seconds = leaf_required(&text, "NewUptime")?
                    .parse::<u64>()
                    .map_err(|_| AdapterError::Protocol)?;
                Ok(StatusInfo {
                    connected,
                    uptime_seconds,
                })
            }
        }
    }
}

/// A mapping entry from a GetGenericPortMappingEntry response body, which
/// carries every New* leaf including the echoed port and protocol.
fn parse_mapping_entry(xml: &str) -> Result<MappingEntry, AdapterError> {
    let external_port = leaf_u16(xml, "NewExternalPort")?;
    let protocol = match leaf_required(xml, "NewProtocol")?.as_str() {
        "TCP" => PortProtocol::Tcp,
        "UDP" => PortProtocol::Udp,
        _ => return Err(AdapterError::Protocol),
    };
    let specific = parse_specific_entry(xml)?;
    if external_port == 0 {
        return Err(AdapterError::Protocol);
    }
    Ok(MappingEntry {
        external_port,
        protocol,
        internal_port: specific.internal_port,
        internal_client: specific.internal_client,
        enabled: specific.enabled,
        description: specific.description,
    })
}

/// The fields a GetSpecificPortMappingEntry response actually carries.
fn parse_specific_entry(xml: &str) -> Result<SpecificEntry, AdapterError> {
    let internal_port = leaf_u16(xml, "NewInternalPort")?;
    if internal_port == 0 {
        return Err(AdapterError::Protocol);
    }
    let internal_client: Ipv4Addr = leaf_required(xml, "NewInternalClient")?
        .parse()
        .map_err(|_| AdapterError::Protocol)?;
    let enabled = match leaf_required(xml, "NewEnabled")?.as_str() {
        "1" | "true" => true,
        "0" | "false" => false,
        _ => return Err(AdapterError::Protocol),
    };
    let description = match leaf(xml, "NewPortMappingDescription")? {
        Some(d) if d.len() <= 64 && !d.is_empty() => Some(d),
        Some(_) => return Err(AdapterError::Protocol),
        None => None,
    };
    Ok(SpecificEntry {
        internal_port,
        internal_client,
        enabled,
        description,
    })
}

fn envelope(inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body>{inner}</s:Body></s:Envelope>"
    )
}

/// Bounded body read: 401/403 are Authentication, content type must be
/// text/xml, and the stream is capped at the shared MAX_BODY. Error statuses
/// are NOT rejected here: gateways answer SOAP faults with HTTP 500, and the
/// fault body is the useful part.
async fn soap_body(response: Response) -> Result<(String, bool), AdapterError> {
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err(AdapterError::Authentication);
    }
    let actual = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|s| s.split(';').next())
        .unwrap_or("")
        .trim();
    if actual != "text/xml" || response.content_length().is_some_and(|n| n > MAX_BODY as u64) {
        return Err(AdapterError::Protocol);
    }
    let mut bytes = Vec::new();
    let mut response = response;
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
    let text = String::from_utf8(bytes).map_err(|_| AdapterError::Protocol)?;
    Ok((text, status.is_success()))
}

fn local_name(tag: &str) -> &str {
    tag.rsplit(':').next().unwrap_or(tag)
}

/// True when an open tag with this local name exists anywhere in the document.
pub fn has_element(xml: &str, local: &str) -> bool {
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let after = &rest[lt + 1..];
        if after.starts_with(['/', '?', '!']) {
            rest = &after[1..];
            continue;
        }
        let end = after
            .find(|c: char| c == '>' || c == '/' || c.is_whitespace())
            .unwrap_or(after.len());
        if local_name(&after[..end]) == local {
            return true;
        }
        rest = &after[end..];
    }
    false
}

/// Some(code) when the body is a SOAP fault; the code is the UPnP
/// `errorCode` from the fault detail when present and numeric.
fn fault_code(xml: &str) -> Result<Option<Option<u16>>, AdapterError> {
    if !has_element(xml, "Fault") {
        return Ok(None);
    }
    let code = match leaf(xml, "errorCode")? {
        Some(raw) => Some(raw.parse::<u16>().map_err(|_| AdapterError::Protocol)?),
        None => None,
    };
    Ok(Some(code))
}

fn leaf_required(xml: &str, local: &str) -> Result<String, AdapterError> {
    leaf(xml, local)?.ok_or(AdapterError::Protocol)
}
fn leaf_u16(xml: &str, local: &str) -> Result<u16, AdapterError> {
    leaf_required(xml, local)?
        .parse::<u16>()
        .map_err(|_| AdapterError::Protocol)
}

/// Text content of the unique element with this local name. Strict: a
/// duplicate element, nested markup inside it, an unterminated tag, unknown
/// entities, or control characters are all Protocol errors. This is a leaf
/// extractor for the flat IGD responses, not a general XML parser.
pub fn leaf(xml: &str, local: &str) -> Result<Option<String>, AdapterError> {
    let mut rest = xml;
    let mut found: Option<String> = None;
    while let Some(lt) = rest.find('<') {
        let after = &rest[lt + 1..];
        if after.starts_with(['/', '?', '!']) {
            rest = &after[1..];
            continue;
        }
        let name_end = after
            .find(|c: char| c == '>' || c == '/' || c.is_whitespace())
            .unwrap_or(after.len());
        let name = local_name(&after[..name_end]);
        let gt = after.find('>').ok_or(AdapterError::Protocol)?;
        if name == local {
            if found.is_some() {
                return Err(AdapterError::Protocol); // duplicate element
            }
            if after[..gt].trim_end().ends_with('/') {
                found = Some(String::new());
            } else {
                let content = &after[gt + 1..];
                let close = content.find("</").ok_or(AdapterError::Protocol)?;
                let closing = &content[close + 2..];
                let close_end = closing.find('>').ok_or(AdapterError::Protocol)?;
                if local_name(closing[..close_end].trim()) != local {
                    return Err(AdapterError::Protocol);
                }
                let raw = &content[..close];
                if raw.contains('<') {
                    return Err(AdapterError::Protocol); // nested markup
                }
                found = Some(decode(raw.trim())?);
                rest = &closing[close_end..];
                continue;
            }
        }
        rest = &after[gt..];
    }
    Ok(found)
}

/// Entity decoding limited to the five standard entities; anything else is
/// refused, and control characters never survive.
fn decode(raw: &str) -> Result<String, AdapterError> {
    if raw.len() > 1024 {
        return Err(AdapterError::Protocol);
    }
    let mut out = String::new();
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        let (entity, decoded) = [
            ("&lt;", "<"),
            ("&gt;", ">"),
            ("&amp;", "&"),
            ("&quot;", "\""),
            ("&apos;", "'"),
        ]
        .into_iter()
        .find(|(entity, _)| after.starts_with(entity))
        .ok_or(AdapterError::Protocol)?;
        out.push_str(decoded);
        rest = &after[entity.len()..];
    }
    out.push_str(rest);
    if out.chars().any(char::is_control) {
        return Err(AdapterError::Protocol);
    }
    Ok(out)
}
