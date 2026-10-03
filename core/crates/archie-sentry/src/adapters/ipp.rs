//! Read-only IPP/1.1 Get-Printer-Attributes (RFC 8010 / RFC 8011).
//! No Print-Job, cancellation, credential exchange or document content is sent.
use super::{body, client, endpoint, MAX_BODY};
use crate::engine::AdapterError;
use archie_types::Timestamp;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

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
            // An unexpected representation of a requested known attribute is not evidence.
            "printer-name"
            | "printer-uuid"
            | "printer-state"
            | "printer-is-accepting-jobs"
            | "printer-state-reasons"
            | "document-format-supported" => return Err(AdapterError::Protocol),
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
}
