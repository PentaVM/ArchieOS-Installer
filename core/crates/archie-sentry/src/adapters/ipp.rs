//! IPP/1.1 printing and observation (RFC 8010 / RFC 8011):
//! Get-Printer-Attributes, Get-Jobs and Print-Job over HTTP.
//!
//! The verified postcondition of `PrintSubmit` is "the printer holds the job":
//! a not-completed queue entry named `archie-<sha256[0..16]>` reported by
//! Get-Jobs. It is NOT "paper came out". Media completion is observed later,
//! outside the 30 s intent deadline, by queue tracking that does not exist
//! yet. No cancellation, credential exchange or format conversion happens
//! here: content must already be in a format the printer declared in
//! `document-format-supported`, and bytes are verified against the command's
//! SHA-256 before anything is sent.
//!
//! The standalone operator tool lives in [`print`]; there is no credential
//! exchange anywhere in this module.
pub mod print;

use super::{body, client, endpoint, MAX_BODY};
use crate::{
    contracts::{
        validate_id, Command, CommandKind, Observation, ObservationOrigin, ResourceState,
    },
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Largest spool document this adapter will read or transmit.
const MAX_SPOOL: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterState {
    Idle,
    Processing,
    Stopped,
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    #[test]
    fn native_ipp_schemes_use_port_631_and_preserve_explicit_ports() {
        for (uri, port) in [
            ("ipp://127.0.0.1/ipp/print", 631),
            ("ipps://127.0.0.1/ipp/print", 631),
            ("ipp://127.0.0.1:80/ipp/print", 80),
            ("ipps://127.0.0.1:443/ipp/print", 443),
        ] {
            let printer = IppClient::new(uri).unwrap();
            assert_eq!(printer.http.port_or_known_default(), Some(port));
            assert_eq!(
                Url::parse(&printer.uri).unwrap().port().unwrap_or(631),
                port
            );
        }
    }
    #[test]
    fn http_inputs_preserve_their_port_in_the_ipp_printer_uri() {
        for (uri, port) in [
            ("http://127.0.0.1/ipp/print", 80),
            ("https://127.0.0.1/ipp/print", 443),
        ] {
            let printer = IppClient::new(uri).unwrap();
            assert_eq!(
                Url::parse(&printer.uri).unwrap().port().unwrap_or(631),
                port
            );
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrinterSnapshot {
    pub name: Option<String>,
    pub uuid: Option<String>,
    pub state: PrinterState,
    pub accepting_jobs: Option<bool>,
    pub reasons: Vec<String>,
    pub document_formats: Vec<String>,
    /// printer-make-and-model, used by driver planning only.
    #[serde(default)]
    pub make_model: Option<String>,
    pub observed_at: Timestamp,
}

fn attribute(bytes: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) -> Result<(), AdapterError> {
    let name_len = u16::try_from(name.len()).map_err(|_| AdapterError::Protocol)?;
    let value_len = u16::try_from(value.len()).map_err(|_| AdapterError::Protocol)?;
    bytes.push(tag);
    bytes.extend_from_slice(&name_len.to_be_bytes());
    bytes.extend_from_slice(name.as_bytes());
    bytes.extend_from_slice(&value_len.to_be_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}
pub fn encode_get_printer_attributes(uri: &str, request_id: u32) -> Result<Vec<u8>, AdapterError> {
    if request_id == 0 || request_id > i32::MAX as u32 || uri.len() > 1024 {
        return Err(AdapterError::Protocol);
    }
    let http = if let Some(s) = uri.strip_prefix("ipp://") {
        format!("http://{s}")
    } else if let Some(s) = uri.strip_prefix("ipps://") {
        format!("https://{s}")
    } else {
        return Err(AdapterError::Protocol);
    };
    let _ = endpoint(&http, false)?;
    let mut bytes = vec![1, 1, 0, 11];
    bytes.extend_from_slice(&request_id.to_be_bytes());
    bytes.push(1);
    attribute(&mut bytes, 0x47, "attributes-charset", b"utf-8")?;
    attribute(&mut bytes, 0x48, "attributes-natural-language", b"en")?;
    attribute(&mut bytes, 0x45, "printer-uri", uri.as_bytes())?;
    for (index, name) in [
        "printer-name",
        "printer-uuid",
        "printer-state",
        "printer-state-reasons",
        "printer-is-accepting-jobs",
        "document-format-supported",
        "printer-make-and-model",
    ]
    .iter()
    .enumerate()
    {
        attribute(
            &mut bytes,
            0x44,
            if index == 0 {
                "requested-attributes"
            } else {
                ""
            },
            name.as_bytes(),
        )?;
    }
    bytes.push(3);
    Ok(bytes)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], AdapterError> {
        let end = self
            .at
            .checked_add(count)
            .filter(|n| *n <= self.bytes.len())
            .ok_or(AdapterError::Protocol)?;
        let data = &self.bytes[self.at..end];
        self.at = end;
        Ok(data)
    }
    fn byte(&mut self) -> Result<u8, AdapterError> {
        Ok(self.take(1)?[0])
    }
    fn short(&mut self) -> Result<usize, AdapterError> {
        let b = self.take(2)?;
        Ok(usize::from(u16::from_be_bytes([b[0], b[1]])))
    }
}
fn text(value: &[u8], max: usize) -> Result<String, AdapterError> {
    if value.is_empty() || value.len() > max {
        return Err(AdapterError::Protocol);
    }
    let value = std::str::from_utf8(value).map_err(|_| AdapterError::Protocol)?;
    if value.chars().any(char::is_control) {
        return Err(AdapterError::Protocol);
    }
    Ok(value.to_owned())
}
pub fn parse_printer_attributes(
    bytes: &[u8],
    request_id: u32,
) -> Result<PrinterSnapshot, AdapterError> {
    if bytes.len() < 9 || bytes.len() > MAX_BODY || request_id == 0 || request_id > i32::MAX as u32
    {
        return Err(AdapterError::Protocol);
    }
    if !matches!((bytes[0], bytes[1]), (1, 1) | (2, 0) | (2, 1) | (2, 2))
        || u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| AdapterError::Protocol)?)
            != request_id
    {
        return Err(AdapterError::Protocol);
    }
    let status = u16::from_be_bytes([bytes[2], bytes[3]]);
    if status > 2 {
        return Err(AdapterError::Unavailable);
    }
    let mut cursor = Cursor { bytes, at: 8 };
    let mut group = 0;
    let mut previous = String::new();
    let mut ended = false;
    let mut count = 0;
    let mut name = None;
    let mut uuid = None;
    let mut state = None;
    let mut accepting = None;
    let mut reasons = vec![];
    let mut formats = vec![];
    let mut make_model = None;
    while cursor.at < bytes.len() {
        let tag = cursor.byte()?;
        if tag == 3 {
            ended = true;
            break;
        }
        if (1..=15).contains(&tag) {
            group = tag;
            previous.clear();
            continue;
        }
        if group == 0 || group == 3 {
            return Err(AdapterError::Protocol);
        }
        count += 1;
        if count > 4096 {
            return Err(AdapterError::Protocol);
        }
        let name_len = cursor.short()?;
        if name_len > 255 {
            return Err(AdapterError::Protocol);
        }
        if name_len > 0 {
            previous = text(cursor.take(name_len)?, 255)?;
        } else if previous.is_empty() {
            return Err(AdapterError::Protocol);
        }
        let length = cursor.short()?;
        let value = cursor.take(length)?;
        if previous == "attributes-charset" && (group != 1 || tag != 0x47 || value != b"utf-8") {
            return Err(AdapterError::Protocol);
        }
        if group != 4 {
            continue;
        }
        match previous.as_str() {
            "printer-name" if tag == 0x42 => {
                if name.is_some() {
                    return Err(AdapterError::Protocol);
                }
                name = Some(text(value, 255)?);
            }
            "printer-uuid" if tag == 0x45 => {
                if uuid.is_some() {
                    return Err(AdapterError::Protocol);
                }
                let id = text(value, 64)?;
                let raw = id.strip_prefix("urn:uuid:").ok_or(AdapterError::Protocol)?;
                if raw.len() != 36
                    || !raw.bytes().enumerate().all(|(i, b)| {
                        if [8, 13, 18, 23].contains(&i) {
                            b == b'-'
                        } else {
                            b.is_ascii_hexdigit()
                        }
                    })
                {
                    return Err(AdapterError::Protocol);
                }
                uuid = Some(id.to_ascii_lowercase());
            }
            "printer-state" if tag == 0x23 && value.len() == 4 => {
                if state.is_some() {
                    return Err(AdapterError::Protocol);
                }
                state = Some(
                    match i32::from_be_bytes(value.try_into().map_err(|_| AdapterError::Protocol)?)
                    {
                        3 => PrinterState::Idle,
                        4 => PrinterState::Processing,
                        5 => PrinterState::Stopped,
                        _ => return Err(AdapterError::Protocol),
                    },
                );
            }
            "printer-is-accepting-jobs" if tag == 0x22 && value.len() == 1 => {
                if accepting.is_some() || value[0] > 1 {
                    return Err(AdapterError::Protocol);
                }
                accepting = Some(value[0] == 1);
            }
            "printer-state-reasons" if tag == 0x44 => {
                if reasons.len() >= 64 {
                    return Err(AdapterError::Protocol);
                }
                reasons.push(text(value, 255)?);
            }
            "document-format-supported" if tag == 0x49 => {
                if formats.len() >= 64 {
                    return Err(AdapterError::Protocol);
                }
                formats.push(text(value, 255)?);
            }
            "printer-make-and-model" if tag == 0x41 => {
                if make_model.is_some() {
                    return Err(AdapterError::Protocol);
                }
                make_model = Some(text(value, 255)?);
            }
            // An unexpected representation of a requested known attribute is not evidence.
            "printer-name"
            | "printer-uuid"
            | "printer-state"
            | "printer-is-accepting-jobs"
            | "printer-state-reasons"
            | "document-format-supported"
            | "printer-make-and-model" => return Err(AdapterError::Protocol),
            _ => {}
        }
    }
    if !ended || cursor.at != bytes.len() {
        return Err(AdapterError::Protocol);
    }
    Ok(PrinterSnapshot {
        name,
        uuid,
        state: state.ok_or(AdapterError::Protocol)?,
        accepting_jobs: accepting,
        reasons,
        document_formats: formats,
        make_model,
        observed_at: Timestamp::now(),
    })
}

pub struct IppClient {
    http: Url,
    uri: String,
    client: Client,
}
impl IppClient {
    pub fn new(input: &str) -> Result<Self, AdapterError> {
        let source = Url::parse(input).map_err(|_| AdapterError::Protocol)?;
        let native = matches!(source.scheme(), "ipp" | "ipps");
        let http_input = if let Some(s) = input.strip_prefix("ipp://") {
            format!("http://{s}")
        } else if let Some(s) = input.strip_prefix("ipps://") {
            format!("https://{s}")
        } else {
            input.to_owned()
        };
        let mut http = endpoint(&http_input, false)?;
        if native {
            http.set_port(Some(source.port().unwrap_or(631)))
                .map_err(|_| AdapterError::Protocol)?;
        }
        if http.path() == "/" || http.path().len() > 512 {
            return Err(AdapterError::Protocol);
        }
        let uri = if native {
            source.to_string()
        } else if http.scheme() == "https" {
            http.as_str().replacen("https://", "ipps://", 1)
        } else {
            http.as_str().replacen("http://", "ipp://", 1)
        };
        let mut uri = Url::parse(&uri).map_err(|_| AdapterError::Protocol)?;
        if !native {
            uri.set_port(http.port_or_known_default())
                .map_err(|_| AdapterError::Protocol)?;
        }
        Ok(Self {
            http,
            uri: uri.to_string(),
            client: client()?,
        })
    }
    pub async fn get_printer_attributes(
        &self,
        request_id: u32,
    ) -> Result<PrinterSnapshot, AdapterError> {
        let request = encode_get_printer_attributes(&self.uri, request_id)?;
        let response = self
            .client
            .post(self.http.clone())
            .header("Content-Type", "application/ipp")
            .header("Accept", "application/ipp")
            .body(request)
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        parse_printer_attributes(&body(response, "application/ipp").await?, request_id)
    }
    pub async fn get_jobs(&self, request_id: u32) -> Result<Vec<String>, AdapterError> {
        let request = encode_get_jobs(&self.uri, request_id)?;
        let response = self
            .client
            .post(self.http.clone())
            .header("Content-Type", "application/ipp")
            .header("Accept", "application/ipp")
            .body(request)
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        parse_job_names(&body(response, "application/ipp").await?, request_id)
    }
    /// Print-Job: IPP request immediately followed by the raw document bytes.
    /// The document is sent exactly as read from the verified spool file; no
    /// conversion of any kind is attempted.
    pub async fn print_job(
        &self,
        request_id: u32,
        job_name: &str,
        document_format: &str,
        document: &[u8],
    ) -> Result<(), AdapterError> {
        if document.is_empty() || document.len() > MAX_SPOOL {
            return Err(AdapterError::Protocol);
        }
        let mut request = encode_print_job(&self.uri, request_id, job_name, document_format)?;
        request.extend_from_slice(document);
        let response = self
            .client
            .post(self.http.clone())
            .header("Content-Type", "application/ipp")
            .header("Accept", "application/ipp")
            .body(request)
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        parse_operation_status(&body(response, "application/ipp").await?, request_id)
    }
}

/// Print-Job (0x0002) request header; document bytes are appended by the caller.
pub fn encode_print_job(
    uri: &str,
    request_id: u32,
    job_name: &str,
    document_format: &str,
) -> Result<Vec<u8>, AdapterError> {
    if request_id == 0
        || request_id > i32::MAX as u32
        || uri.len() > 1024
        || job_name.is_empty()
        || job_name.len() > 64
        || job_name.bytes().any(|b| b.is_ascii_control())
        || document_format.is_empty()
        || document_format.len() > 64
        || document_format.bytes().any(|b| b.is_ascii_control())
    {
        return Err(AdapterError::Protocol);
    }
    let mut bytes = vec![1, 1, 0, 2];
    bytes.extend_from_slice(&request_id.to_be_bytes());
    bytes.push(1);
    attribute(&mut bytes, 0x47, "attributes-charset", b"utf-8")?;
    attribute(&mut bytes, 0x48, "attributes-natural-language", b"en")?;
    attribute(&mut bytes, 0x45, "printer-uri", uri.as_bytes())?;
    attribute(&mut bytes, 0x42, "job-name", job_name.as_bytes())?;
    attribute(&mut bytes, 0x49, "document-format", document_format.as_bytes())?;
    bytes.push(3);
    Ok(bytes)
}

/// Get-Jobs (0x000A) for the not-completed queue, requesting job-name only.
pub fn encode_get_jobs(uri: &str, request_id: u32) -> Result<Vec<u8>, AdapterError> {
    if request_id == 0 || request_id > i32::MAX as u32 || uri.len() > 1024 {
        return Err(AdapterError::Protocol);
    }
    let mut bytes = vec![1, 1, 0, 10];
    bytes.extend_from_slice(&request_id.to_be_bytes());
    bytes.push(1);
    attribute(&mut bytes, 0x47, "attributes-charset", b"utf-8")?;
    attribute(&mut bytes, 0x48, "attributes-natural-language", b"en")?;
    attribute(&mut bytes, 0x45, "printer-uri", uri.as_bytes())?;
    attribute(&mut bytes, 0x44, "which-jobs", b"not-completed")?;
    attribute(&mut bytes, 0x44, "requested-attributes", b"job-name")?;
    bytes.push(3);
    Ok(bytes)
}

/// Header-only status check for operations whose response body carries no
/// evidence this adapter uses (Print-Job).
fn parse_operation_status(bytes: &[u8], request_id: u32) -> Result<(), AdapterError> {
    if bytes.len() < 8 || bytes.len() > MAX_BODY || request_id == 0 || request_id > i32::MAX as u32
    {
        return Err(AdapterError::Protocol);
    }
    if !matches!((bytes[0], bytes[1]), (1, 1) | (2, 0) | (2, 1) | (2, 2))
        || u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| AdapterError::Protocol)?)
            != request_id
    {
        return Err(AdapterError::Protocol);
    }
    match u16::from_be_bytes([bytes[2], bytes[3]]) {
        // successful-ok and its informational variants
        0..=2 => Ok(()),
        // client errors, e.g. client-error-document-format-not-supported (0x040A)
        0x400..=0x4ff => Err(AdapterError::Unsupported),
        _ => Err(AdapterError::Unavailable),
    }
}

/// Job names from a Get-Jobs response (job attributes groups, tag 2).
/// Names are bounded to 64 bytes and control characters are rejected.
pub fn parse_job_names(bytes: &[u8], request_id: u32) -> Result<Vec<String>, AdapterError> {
    if bytes.len() < 9 || bytes.len() > MAX_BODY || request_id == 0 || request_id > i32::MAX as u32
    {
        return Err(AdapterError::Protocol);
    }
    if !matches!((bytes[0], bytes[1]), (1, 1) | (2, 0) | (2, 1) | (2, 2))
        || u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| AdapterError::Protocol)?)
            != request_id
    {
        return Err(AdapterError::Protocol);
    }
    let status = u16::from_be_bytes([bytes[2], bytes[3]]);
    if status > 2 {
        return Err(AdapterError::Unavailable);
    }
    let mut cursor = Cursor { bytes, at: 8 };
    let mut group = 0;
    let mut previous = String::new();
    let mut ended = false;
    let mut count = 0;
    let mut names = vec![];
    while cursor.at < bytes.len() {
        let tag = cursor.byte()?;
        if tag == 3 {
            ended = true;
            break;
        }
        if (1..=15).contains(&tag) {
            group = tag;
            previous.clear();
            continue;
        }
        if group == 0 || group == 3 {
            return Err(AdapterError::Protocol);
        }
        count += 1;
        if count > 4096 {
            return Err(AdapterError::Protocol);
        }
        let name_len = cursor.short()?;
        if name_len > 255 {
            return Err(AdapterError::Protocol);
        }
        if name_len > 0 {
            previous = text(cursor.take(name_len)?, 255)?;
        } else if previous.is_empty() {
            return Err(AdapterError::Protocol);
        }
        let length = cursor.short()?;
        let value = cursor.take(length)?;
        if previous == "attributes-charset" && (group != 1 || tag != 0x47 || value != b"utf-8") {
            return Err(AdapterError::Protocol);
        }
        if group != 2 {
            continue;
        }
        match previous.as_str() {
            "job-name" if tag == 0x42 => {
                if names.len() >= 64 {
                    return Err(AdapterError::Protocol);
                }
                names.push(text(value, 64)?);
            }
            // A job-name in any other representation is not evidence.
            "job-name" => return Err(AdapterError::Protocol),
            _ => {}
        }
    }
    if !ended || cursor.at != bytes.len() {
        return Err(AdapterError::Protocol);
    }
    Ok(names)
}

/// One paired IPP printer: submits verified spool content and observes the
/// queue. See the module docs for the postcondition contract.
pub struct IppAdapter {
    edge: String,
    adapter: String,
    resource: String,
    ipp: IppClient,
    spool: PathBuf,
    request: u32,
    observations: u64,
}
impl std::fmt::Debug for IppAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IppAdapter")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("resource", &self.resource)
            .field("spool", &self.spool)
            .finish_non_exhaustive()
    }
}
impl IppAdapter {
    /// `spool` is a local directory where the caller staged content as
    /// `spool/<sha256>`; the file's real SHA-256 is verified before sending.
    pub fn new(
        edge: &str,
        adapter: &str,
        printer_uri: &str,
        resource: &str,
        spool: PathBuf,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        validate_id(resource).map_err(|_| AdapterError::Protocol)?;
        if !spool.is_absolute() {
            return Err(AdapterError::Protocol);
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            resource: resource.into(),
            ipp: IppClient::new(printer_uri)?,
            spool,
            request: 0,
            observations: 0,
        })
    }
    fn next_request(&mut self) -> u32 {
        self.request = if self.request >= i32::MAX as u32 {
            1
        } else {
            self.request + 1
        };
        self.request
    }
    fn resource_check(&self, resource: &ResourceDescriptor) -> Result<(), AdapterError> {
        if !resource.paired
            || resource.adapter_id != self.adapter
            || resource.edge_id != self.edge
            || resource.resource_id != self.resource
        {
            return Err(AdapterError::Unsupported);
        }
        Ok(())
    }
}
#[async_trait]
impl EdgeAdapter for IppAdapter {
    fn edge_id(&self) -> &str {
        &self.edge
    }
    fn adapter_id(&self) -> &str {
        &self.adapter
    }
    /// Get-Printer-Attributes (printer-state) + Get-Jobs (not-completed).
    /// `idle` is true only for printer-state 3 (idle); 4 (processing) and
    /// 5 (stopped) are not idle. `job_present` is the first not-completed
    /// queue entry named `archie-*`, else None.
    async fn observe(&mut self, resource: &ResourceDescriptor) -> Result<Observation, AdapterError> {
        self.resource_check(resource)?;
        let attributes_request = self.next_request();
        let attributes = self.ipp.get_printer_attributes(attributes_request).await?;
        let jobs_request = self.next_request();
        let jobs = self.ipp.get_jobs(jobs_request).await?;
        let state = ResourceState::Printer {
            idle: attributes.state == PrinterState::Idle,
            job_present: jobs.into_iter().find(|name| name.starts_with("archie-")),
        };
        let now = Timestamp::now();
        self.observations = self.observations.saturating_add(1);
        // The counter keeps evidence identity honest when nothing changed.
        let report = serde_json::to_vec(&(&state, now.0, self.observations))
            .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            evidence_id: format!("ipp-{:x}", Sha256::digest(report)),
            observed_at: now,
            origin: ObservationOrigin::Measurement,
            state,
        })
    }
    async fn apply(
        &mut self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        command.validate().map_err(|_| AdapterError::Protocol)?;
        let Command::PrintSubmit {
            sha256, format, ..
        } = command
        else {
            return Err(AdapterError::Unsupported);
        };
        self.resource_check(resource)?;
        if !resource.capabilities.contains(&CommandKind::PrintSubmit) {
            return Err(AdapterError::Unsupported);
        }
        let job_name = Command::print_job_name(sha256).map_err(|_| AdapterError::Protocol)?;
        // sha256 is validated lowercase hex, so this join cannot escape the spool.
        let path = self.spool.join(sha256);
        let metadata = std::fs::metadata(&path).map_err(|_| AdapterError::Unavailable)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SPOOL as u64 {
            return Err(AdapterError::Protocol);
        }
        let document = std::fs::read(&path).map_err(|_| AdapterError::Unavailable)?;
        // Unverified bytes are never sent.
        if format!("{:x}", Sha256::digest(&document)) != *sha256 {
            return Err(AdapterError::Protocol);
        }
        let attributes_request = self.next_request();
        let attributes = self.ipp.get_printer_attributes(attributes_request).await?;
        if !attributes
            .document_formats
            .iter()
            .any(|f| f == format.mime())
        {
            // Operator log only: supported formats never enter receipts.
            eprintln!(
                "archie-sentry ipp: printer does not accept {}; document-format-supported: {:?}",
                format.mime(),
                attributes.document_formats
            );
            return Err(AdapterError::Unsupported);
        }
        let print_request = self.next_request();
        self.ipp
            .print_job(print_request, &job_name, format.mime(), &document)
            .await?;
        Ok(AdapterAck { accepted: true })
    }
}
