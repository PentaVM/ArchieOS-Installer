//! The LAN inventory `archie-lan watch` keeps in a file (`archie-lan-inventory-v1`).
//!
//! The bridge only reads it. `archie-lan` is passive (SSDP and mDNS, no port probing); this
//! module never starts a scan, so opening the Sentry panel cannot put anything on the
//! network. A file that is missing, unreadable, malformed, of another format or old is each
//! its own state: an empty list and "the scanner is not running" must never look alike.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// The format string `archie-lan` writes (`archie_lan::inventory::FORMAT`).
pub const FORMAT: &str = "archie-lan-inventory-v1";
/// Largest inventory file read. A home network is kilobytes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Devices forwarded to the shell at most; the rest are counted, not sent.
pub const MAX_DEVICES: usize = 512;
/// `archie-lan watch` rescans every 300 s by default; three missed scans is stale.
pub const STALE_AFTER_SECONDS: u64 = 900;

/// One device, as `archie-lan` described it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub address: String,
    pub class: Option<String>,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub services: Vec<String>,
    pub seen_by: Vec<String>,
    pub state: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Raw {
    format: String,
    scanned_unix: u64,
    devices: Vec<Device>,
}

/// What the bridge can say about the LAN inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Lan {
    /// The bridge was not told where the inventory is.
    NotConfigured,
    /// No file there: the scanner has not run (or is not installed).
    Missing { path: String },
    /// The file exists but this account cannot read it.
    Unreadable { reason: String },
    /// Read, but not a usable `archie-lan-inventory-v1`.
    Invalid { reason: String },
    /// A real inventory. `stale` when it is older than three default scan intervals.
    Ok {
        scanned_unix: u64,
        age_seconds: u64,
        stale: bool,
        /// Devices in the file, which can exceed the `devices` sent.
        total: usize,
        devices: Vec<Device>,
    },
}

/// Read the inventory at `path`, if one is configured.
pub fn read(path: Option<&Path>, now_unix: u64) -> Lan {
    let Some(path) = path else {
        return Lan::NotConfigured;
    };
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Lan::Missing {
                path: path.display().to_string(),
            }
        }
        Err(e) => {
            return Lan::Unreadable {
                reason: e.to_string(),
            }
        }
    };
    let invalid = |reason: &str| Lan::Invalid {
        reason: reason.to_string(),
    };
    match file.metadata() {
        Ok(meta) if !meta.is_file() => return invalid("not a regular file"),
        Ok(meta) if meta.len() > MAX_FILE_BYTES => return invalid("larger than the 1 MiB limit"),
        Ok(_) => {}
        Err(e) => {
            return Lan::Unreadable {
                reason: e.to_string(),
            }
        }
    }
    let mut bytes = Vec::new();
    // Bounded even if the file grows between the metadata check and the read.
    if let Err(e) = file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes) {
        return Lan::Unreadable {
            reason: e.to_string(),
        };
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return invalid("larger than the 1 MiB limit");
    }
    let raw: Raw = match serde_json::from_slice(&bytes) {
        Ok(raw) => raw,
        Err(e) => {
            return Lan::Invalid {
                reason: format!("does not match the inventory schema: {e}"),
            }
        }
    };
    if raw.format != FORMAT {
        return Lan::Invalid {
            reason: format!("format {:?}, expected {FORMAT:?}", raw.format),
        };
    }
    // A scan time in the future (clock skew) is age 0, never a negative age.
    let age_seconds = now_unix.saturating_sub(raw.scanned_unix);
    let total = raw.devices.len();
    Lan::Ok {
        scanned_unix: raw.scanned_unix,
        age_seconds,
        stale: age_seconds > STALE_AFTER_SECONDS,
        total,
        devices: raw.devices.into_iter().take(MAX_DEVICES).collect(),
    }
}
