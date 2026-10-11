//! IPP Print-Job and Get-Job-Attributes (RFC 8011 §4.2.1, §4.3.4).
//!
//! A document is sent in a format the printer said it accepts, and the job is then read
//! back until the printer reports how it ended. `Completed` is the printer's own report
//! that it finished the job; it is not a camera on the output tray.
use super::{attribute, text, Cursor, IppClient};
use crate::adapters::{body, MAX_BODY};
use crate::engine::AdapterError;
use archie_types::Timestamp;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The largest document sent to a printer.
pub const MAX_DOCUMENT: usize = 32 * 1024 * 1024;
const PRINT_JOB: u16 = 0x0002;
const GET_JOB_ATTRIBUTES: u16 = 0x0009;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Pending,
    Held,
    Processing,
    Stopped,
    Canceled,
    Aborted,
    Completed,
}
impl JobState {
    /// Whether the printer will do nothing more with the job.
    pub fn ended(self) -> bool {
        matches!(self, Self::Canceled | Self::Aborted | Self::Completed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobSnapshot {
    pub job_id: u32,
    pub state: JobState,
    pub reasons: Vec<String>,
    pub observed_at: Timestamp,
}

/// The format of a document, from how it begins. `None` is not something to send as is.
pub fn format_of(document: &[u8]) -> Option<&'static str> {
    match document {
        [b'%', b'P', b'D', b'F', ..] => Some("application/pdf"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'R', b'a', b'S', b'2', ..] => Some("image/pwg-raster"),
        [b'U', b'N', b'I', b'R', b'A', b'S', b'T', 0, ..] => Some("image/urf"),
        _ => None,
    }
}

fn operation(code: u16, uri: &str, request_id: u32) -> Result<Vec<u8>, AdapterError> {
    if request_id == 0 || request_id > i32::MAX as u32 || uri.len() > 1024 {
        return Err(AdapterError::Protocol);
    }
    let mut bytes = vec![1, 1];
    bytes.extend_from_slice(&code.to_be_bytes());
    bytes.extend_from_slice(&request_id.to_be_bytes());
    bytes.push(1);
    attribute(&mut bytes, 0x47, "attributes-charset", b"utf-8")?;
    attribute(&mut bytes, 0x48, "attributes-natural-language", b"en")?;
    attribute(&mut bytes, 0x45, "printer-uri", uri.as_bytes())?;
    attribute(&mut bytes, 0x42, "requesting-user-name", b"archie")?;
    Ok(bytes)
}

/// The request that precedes the document's bytes.
pub fn encode_print_job(uri: &str, request_id: u32, job_name: &str, format: &str) -> Result<Vec<u8>, AdapterError> {
    let named = !job_name.is_empty() && job_name.len() <= 255 && !job_name.chars().any(char::is_control);
    let typed = !format.is_empty() && format.len() <= 255 && format.bytes().all(|b| b.is_ascii_graphic());
    if !named || !typed {
        return Err(AdapterError::Protocol);
    }
    let mut bytes = operation(PRINT_JOB, uri, request_id)?;
    attribute(&mut bytes, 0x42, "job-name", job_name.as_bytes())?;
    attribute(&mut bytes, 0x49, "document-format", format.as_bytes())?;
    bytes.push(3);
    Ok(bytes)
}

pub fn encode_get_job_attributes(uri: &str, request_id: u32, job_id: u32) -> Result<Vec<u8>, AdapterError> {
    if job_id == 0 || job_id > i32::MAX as u32 {
        return Err(AdapterError::Protocol);
    }
    let mut bytes = operation(GET_JOB_ATTRIBUTES, uri, request_id)?;
    attribute(&mut bytes, 0x21, "job-id", &job_id.to_be_bytes())?;
    attribute(&mut bytes, 0x44, "requested-attributes", b"job-id")?;
    attribute(&mut bytes, 0x44, "", b"job-state")?;
    attribute(&mut bytes, 0x44, "", b"job-state-reasons")?;
    bytes.push(3);
    Ok(bytes)
}

/// Reads the job a printer describes in answer to either request.
pub fn parse_job(bytes: &[u8], request_id: u32) -> Result<JobSnapshot, AdapterError> {
    if bytes.len() < 9 || bytes.len() > MAX_BODY || !matches!((bytes[0], bytes[1]), (1, 1) | (2, 0..=2)) {
        return Err(AdapterError::Protocol);
    }
    if u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) != request_id {
        return Err(AdapterError::Protocol);
    }
    match u16::from_be_bytes([bytes[2], bytes[3]]) {
        0..=2 => {}
        0x0401..=0x0403 => return Err(AdapterError::Authentication),
        // The printer does not take this format, or this kind of document.
        0x040a | 0x040b => return Err(AdapterError::Unsupported),
        _ => return Err(AdapterError::Unavailable),
    }
    let mut cursor = Cursor { bytes, at: 8 };
    let (mut group, mut previous, mut ended, mut count) = (0u8, String::new(), false, 0usize);
    let (mut job_id, mut state, mut reasons) = (None, None, Vec::new());
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
        count += 1;
        if group == 0 || count > 4096 {
            return Err(AdapterError::Protocol);
        }
        let name_len = cursor.short()?;
        if name_len > 0 {
            previous = text(cursor.take(name_len)?, 255)?;
        } else if previous.is_empty() {
            return Err(AdapterError::Protocol);
        }
        let length = cursor.short()?;
        let value = cursor.take(length)?;
        if group != 2 {
            continue;
        }
        match (previous.as_str(), tag, value.len()) {
            ("job-id", 0x21, 4) if job_id.is_none() => {
                let id = i32::from_be_bytes([value[0], value[1], value[2], value[3]]);
                job_id = Some(u32::try_from(id).ok().filter(|id| *id > 0).ok_or(AdapterError::Protocol)?);
            }
            ("job-state", 0x23, 4) if state.is_none() => {
                state = Some(match i32::from_be_bytes([value[0], value[1], value[2], value[3]]) {
                    3 => JobState::Pending,
                    4 => JobState::Held,
                    5 => JobState::Processing,
                    6 => JobState::Stopped,
                    7 => JobState::Canceled,
                    8 => JobState::Aborted,
                    9 => JobState::Completed,
                    _ => return Err(AdapterError::Protocol),
                });
            }
            ("job-state-reasons", 0x44, _) if reasons.len() < 64 => reasons.push(text(value, 255)?),
            // A job identifier or state in another shape, or said twice, is not evidence.
            ("job-id" | "job-state", _, _) => return Err(AdapterError::Protocol),
            _ => {}
        }
    }
    if !ended || cursor.at != bytes.len() {
        return Err(AdapterError::Protocol);
    }
    Ok(JobSnapshot {
        job_id: job_id.ok_or(AdapterError::Protocol)?,
        state: state.ok_or(AdapterError::Protocol)?,
        reasons,
        observed_at: Timestamp::now(),
    })
}

impl IppClient {
    async fn exchange(&self, request: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, AdapterError> {
        let response = self
            .client
            .post(self.http.clone())
            .header("Content-Type", "application/ipp")
            .header("Accept", "application/ipp")
            .timeout(timeout)
            .body(request)
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        body(response, "application/ipp").await
    }

    /// Sends one document as one job. It is sent once: a failure after the request left
    /// may still have printed, and is reported, never retried.
    pub async fn print_job(&self, request_id: u32, job_name: &str, format: &str, document: &[u8]) -> Result<JobSnapshot, AdapterError> {
        if document.is_empty() || document.len() > MAX_DOCUMENT {
            return Err(AdapterError::Protocol);
        }
        let mut request = encode_print_job(&self.uri, request_id, job_name, format)?;
        request.extend_from_slice(document);
        parse_job(&self.exchange(request, Duration::from_secs(120)).await?, request_id)
    }

    /// What the printer says of a job it accepted.
    pub async fn job(&self, request_id: u32, job_id: u32) -> Result<JobSnapshot, AdapterError> {
        let request = encode_get_job_attributes(&self.uri, request_id, job_id)?;
        parse_job(&self.exchange(request, Duration::from_secs(5)).await?, request_id)
    }
}
