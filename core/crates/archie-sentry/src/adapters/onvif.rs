//! Observation-only ONVIF camera probe over SOAP (WS-I / ONVIF Core, SOAP 1.2).
//!
//! Cameras register as telemetry resources, like Home Assistant sensors, not as
//! actuable resources. This module therefore exposes a plain [`OnvifProbe`]
//! returning [`CameraInfo`] instead of an `EdgeAdapter` implementation: none of
//! the observable camera facts (device information, stream URIs) is a
//! postcondition of a `Command`, and faking one as `ResourceState::Unknown`
//! would be dishonest. PTZ control is deliberately unimplemented: `GetStatus`
//! on the devices reachable here reports relative move state, not absolute
//! position, so a `MoveTo` command has no measurable postcondition and this
//! engine does not actuate what it cannot verify.
//!
//! Authentication uses WS-Security UsernameToken with PasswordDigest
//! (SHA-1 over nonce + created + password, base64). Neither `sha1` nor `base64`
//! is a dependency of this crate, so both are implemented locally below; SHA-1
//! is required by the WS-Security profile and is used only for this legacy
//! digest, never as an integrity primitive inside Archie. Credentials come from
//! constructor config, never from URLs, and are redacted in `Debug`. The shared
//! endpoint policy applies: credentialed probes require HTTPS or loopback —
//! plaintext HTTP with a password off-loopback is refused.
//!
//! RTSP stream URIs are returned as data only; they are validated as URIs and
//! never opened by this crate.
use super::{body, client, endpoint};
use crate::{contracts::validate_id, engine::AdapterError};
use archie_types::Timestamp;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SOAP_ACTION_DEVICE: &str = "http://www.onvif.org/ver10/device/wsdl";
const SOAP_ACTION_MEDIA: &str = "http://www.onvif.org/ver10/media/wsdl";
/// At most this many profiles are queried for a stream URI per probe.
const MAX_STREAM_PROFILES: usize = 2;
const MAX_STREAM_URIS: usize = 8;

/// Measurable camera facts. All strings are bounded and control-character free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraInfo {
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    /// RTSP/RTP stream endpoints as data; validated, never opened.
    pub stream_uris: Vec<String>,
    /// Local wall-clock time of the probe; the camera reports no timestamp here.
    pub probed_at: Timestamp,
}

pub struct OnvifProbe {
    base: Url,
    client: Client,
    credentials: Option<(String, String)>,
}
impl std::fmt::Debug for OnvifProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnvifProbe")
            .field("base", &self.base.as_str())
            .field("credentials", &"[redacted]")
            .finish_non_exhaustive()
    }
}
impl OnvifProbe {
    /// `base` is the device service URL, e.g. `http://192.168.1.20/onvif/device_service`.
    pub fn new(base: &str, credentials: Option<(String, String)>) -> Result<Self, AdapterError> {
        let base = endpoint(base, credentials.is_some())?;
        if base.path().is_empty() {
            return Err(AdapterError::Protocol);
        }
        if let Some((username, password)) = &credentials {
            let valid = |s: &str, max: usize| {
                !s.is_empty() && s.len() <= max && !s.bytes().any(|b| b < 0x20 || b == 0x7f)
            };
            if !valid(username, 128) || !valid(password, 256) {
                return Err(AdapterError::Authentication);
            }
        }
        Ok(Self {
            base,
            client: client()?,
            credentials,
        })
    }
    async fn soap(&self, inner: &str) -> Result<String, AdapterError> {
        let wsse = self.credentials.as_ref().map(|(username, password)| {
            let created = Timestamp::now().to_rfc3339();
            // No RNG dependency: the nonce mixes the secret with nanosecond time.
            let nonce = fresh_nonce(password, &created);
            wsse_header(username, password, &nonce, &created)
        });
        let envelope = soap_envelope(inner, wsse.as_deref());
        let response = self
            .client
            .post(self.base.clone())
            .header("Content-Type", "application/soap+xml; charset=utf-8")
            .body(envelope)
            .send()
            .await
            .map_err(|_| AdapterError::Transport)?;
        // SOAP 1.2 only; devices answering text/xml (SOAP 1.1) are refused.
        let bytes = body(response, "application/soap+xml").await?;
        let text = String::from_utf8(bytes).map_err(|_| AdapterError::Protocol)?;
        if xml_contains_tag(&text, "Fault") {
            return Err(AdapterError::Protocol);
        }
        Ok(text)
    }
    /// GetDeviceInformation, then GetProfiles + GetStreamUri for the first
    /// profiles. Any step failing fails the probe; nothing is fabricated.
    pub async fn probe(&self) -> Result<CameraInfo, AdapterError> {
        let info = self.soap("<tds:GetDeviceInformation/>").await?;
        let bounded = |value: Option<String>| -> Result<String, AdapterError> {
            let value = value.ok_or(AdapterError::Protocol)?;
            if value.is_empty() || value.len() > 64 || value.bytes().any(|b| b < 0x20 || b == 0x7f)
            {
                return Err(AdapterError::Protocol);
            }
            Ok(value)
        };
        let manufacturer = bounded(xml_text(&info, "Manufacturer"))?;
        let model = bounded(xml_text(&info, "Model"))?;
        let firmware = bounded(xml_text(&info, "FirmwareVersion"))?;
        let profiles = self.soap("<trt:GetProfiles/>").await?;
        let tokens = xml_attr_values(&profiles, "Profiles", "token");
        let mut stream_uris = Vec::new();
        for token in tokens.iter().take(MAX_STREAM_PROFILES) {
            let inner = format!(
                "<trt:GetStreamUri><trt:StreamSetup>\
                 <tt:Stream xmlns:tt=\"http://www.onvif.org/ver10/schema\">RTP-Unicast</tt:Stream>\
                 <tt:Transport xmlns:tt=\"http://www.onvif.org/ver10/schema\">\
                 <tt:Protocol>RTSP</tt:Protocol></tt:Transport></trt:StreamSetup>\
                 <trt:ProfileToken>{}</trt:ProfileToken></trt:GetStreamUri>",
                xml_escape(token)
            );
            let response = self.soap(&inner).await?;
            if let Some(uri) = xml_text(&response, "Uri") {
                if stream_uris.len() < MAX_STREAM_URIS && valid_stream_uri(&uri) {
                    stream_uris.push(uri);
                }
            }
        }
        Ok(CameraInfo {
            manufacturer,
            model,
            firmware,
            stream_uris,
            probed_at: Timestamp::now(),
        })
    }
}

fn valid_stream_uri(uri: &str) -> bool {
    if uri.len() > 512 || uri.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return false;
    }
    let rest = uri
        .strip_prefix("rtsp://")
        .or_else(|| uri.strip_prefix("rtsps://"));
    rest.is_some_and(|r| {
        r.split(['/', ':'])
            .next()
            .is_some_and(|host| !host.is_empty())
    })
}

/// A nonce this process never repeats. `created` has millisecond resolution, so
/// requests in one millisecond shared a nonce, and a camera that refuses replays (as
/// WS-Security recommends) rejected all but the first: a counter makes each unique.
fn fresh_nonce(password: &str, created: &str) -> [u8; 16] {
    static ISSUED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = ISSUED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let digest = Sha256::digest(format!(
        "onvif-nonce\0{password}\0{created}\0{}\0{count}\0{nanos}",
        std::process::id()
    ));
    let mut nonce = [0u8; 16];
    nonce.copy_from_slice(&digest[..16]);
    nonce
}

/// Standard base64 (RFC 4648, with padding). Implemented locally because the
/// `base64` crate is not a dependency of archie-sentry.
pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Minimal SHA-1 (FIPS 180-1), implemented locally solely for the WS-Security
/// PasswordDigest profile. Archie-internal integrity uses SHA-256.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bit_length = (data.len() as u64).wrapping_mul(8);
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());
    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// WS-Security PasswordDigest: base64(SHA-1(nonce || created || password)).
pub fn password_digest(nonce: &[u8], created: &str, password: &str) -> String {
    let mut input = nonce.to_vec();
    input.extend_from_slice(created.as_bytes());
    input.extend_from_slice(password.as_bytes());
    base64_encode(&sha1(&input))
}

/// UsernameToken header block. Deterministic given (username, password, nonce,
/// created); credentials are XML-escaped before interpolation.
pub fn wsse_header(username: &str, password: &str, nonce: &[u8], created: &str) -> String {
    format!(
        "<wsse:Security s:mustUnderstand=\"1\" \
         xmlns:wsse=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd\" \
         xmlns:wsu=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd\">\
         <wsse:UsernameToken>\
         <wsse:Username>{}</wsse:Username>\
         <wsse:Password Type=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest\">{}</wsse:Password>\
         <wsse:Nonce EncodingType=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary\">{}</wsse:Nonce>\
         <wsu:Created>{}</wsu:Created>\
         </wsse:UsernameToken></wsse:Security>",
        xml_escape(username),
        password_digest(nonce, created, password),
        base64_encode(nonce),
        xml_escape(created)
    )
}

pub fn soap_envelope(inner: &str, wsse: Option<&str>) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\" \
         xmlns:tds=\"{SOAP_ACTION_DEVICE}\" xmlns:trt=\"{SOAP_ACTION_MEDIA}\">\
         <s:Header>{}</s:Header><s:Body>{inner}</s:Body></s:Envelope>",
        wsse.unwrap_or(""),
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn local_name(tag: &str) -> &str {
    tag.rsplit(':').next().unwrap_or(tag)
}
/// True when an element with this local name exists anywhere in the document.
fn xml_contains_tag(xml: &str, local: &str) -> bool {
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
/// Text content of the first element with this local name, entity-decoded.
/// Sufficient for the flat ONVIF responses used here; not a general XML parser.
fn xml_text(xml: &str, local: &str) -> Option<String> {
    let mut rest = xml;
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
        let gt = after.find('>')?;
        if name == local {
            if after[..gt].trim_end().ends_with('/') {
                return Some(String::new());
            }
            let content = &after[gt + 1..];
            let mut cursor = content;
            while let Some(close) = cursor.find("</") {
                let closing = &cursor[close + 2..];
                let close_end = closing.find('>')?;
                if local_name(closing[..close_end].trim()) == local {
                    return Some(xml_unescape(cursor[..close].trim()));
                }
                cursor = &closing[close_end..];
            }
            return None;
        }
        rest = &after[gt + 1..];
    }
    None
}
/// All values of `attr` on opening tags with this local name, in document order.
fn xml_attr_values(xml: &str, local: &str, attr: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = xml;
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
        let gt = after.find('>').unwrap_or(after.len());
        if name == local {
            let tag = &after[..gt];
            let needle = format!("{attr}=\"");
            if let Some(at) = tag.find(&needle) {
                let value = &tag[at + needle.len()..];
                if let Some(end) = value.find('"') {
                    values.push(xml_unescape(&value[..end]));
                }
            }
        }
        rest = &after[gt..];
    }
    values
}

/// Resource ids of cameras are still stable Archie identifiers; callers wiring
/// probe results into the registry should validate with this before use.
pub fn validate_camera_resource_id(id: &str) -> Result<(), AdapterError> {
    validate_id(id).map_err(|_| AdapterError::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_in_one_millisecond_are_all_different() {
        let created = "2026-10-12T11:27:00.123Z";
        let nonces: std::collections::BTreeSet<[u8; 16]> =
            (0..1000).map(|_| fresh_nonce("secret", created)).collect();
        assert_eq!(nonces.len(), 1000);
    }
}
