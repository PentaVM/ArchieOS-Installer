//! What a device's own traffic says it is, read passively: who made its network card,
//! whether its hardware address is a private random one, and hints at its operating
//! system from the way its DHCP client, its TCP stack and its TLS library behave.
//!
//! Every hint names the evidence it rests on. None is a verdict: a vendor class can be
//! set by hand, a TTL changes behind a router, and a random MAC hides the maker on purpose.
//! Nothing here sends a frame to anyone.

use crate::audit::Device;
use serde::Serialize;

/// Who registered the first half of a hardware address with the IEEE, unless the address
/// is locally administered (random), in which case it names no maker.
pub fn vendor(mac: &str) -> Option<&'static str> {
    if randomized(mac) {
        return None;
    }
    oui_data::lookup(mac).map(|entry| entry.organization())
}

/// A locally administered address: phones and laptops pick one per network for privacy.
pub fn randomized(mac: &str) -> bool {
    u8::from_str_radix(mac.get(0..2).unwrap_or("00"), 16).is_ok_and(|first| first & 0b10 != 0)
}

/// The TTL a host most likely started from, given what arrived: the next of the usual
/// starting values (32, 64, 128, 255) at or above it.
pub fn initial_hops(observed: u8) -> u8 {
    match observed {
        0..=32 => 32,
        33..=64 => 64,
        65..=128 => 128,
        _ => 255,
    }
}

fn hop_family(initial: u8) -> &'static str {
    match initial {
        64 => "Linux, Android, macOS, iOS or most embedded systems",
        128 => "Windows",
        255 => "network equipment, or a Solaris or BSD stack",
        _ => "an older or unusual stack",
    }
}

/// The client a DHCP vendor class names. Only classes with one well-known meaning.
fn dhcp_family(vendor: &str) -> Option<&'static str> {
    let vendor = vendor.to_ascii_lowercase();
    if vendor.starts_with("msft") {
        Some("Windows")
    } else if vendor.starts_with("android-dhcp") {
        Some("Android")
    } else if vendor.starts_with("dhcpcd") {
        Some("dhcpcd (Linux, Android or BSD)")
    } else if vendor.starts_with("udhcp") {
        Some("BusyBox udhcpc (embedded Linux)")
    } else {
        None
    }
}

/// One hint and what it rests on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hint {
    pub says: String,
    pub basis: String,
}

/// Everything passive the audit knows about one device's identity.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Profile {
    pub mac: String,
    pub vendor: Option<&'static str>,
    pub randomized_mac: bool,
    pub hints: Vec<Hint>,
    /// The DHCP options it asks for, in order: equal strings mean the same DHCP stack.
    pub dhcp_fingerprint: Option<String>,
    pub ja4: Vec<String>,
    pub user_agents: Vec<String>,
    pub lldp_name: Option<String>,
    pub netbios_names: Vec<String>,
}

pub fn profile(mac: &str, device: &Device) -> Profile {
    let mut hints = Vec::new();
    if let Some(vendor) = &device.dhcp_vendor {
        if let Some(family) = dhcp_family(vendor) {
            hints.push(Hint {
                says: family.into(),
                basis: format!("DHCP vendor class {vendor:?}"),
            });
        }
    }
    if let Some(initial) = device.hops {
        hints.push(Hint {
            says: hop_family(initial).into(),
            basis: format!("connections it opens start with a TTL of {initial}"),
        });
    }
    if randomized(mac) {
        hints.push(Hint {
            says: "a locally administered address: usually a phone, tablet or laptop with \
                   MAC privacy on, though anything can choose one"
                .into(),
            basis: "the locally administered bit of its hardware address is set".into(),
        });
    }
    Profile {
        mac: mac.into(),
        vendor: vendor(mac),
        randomized_mac: randomized(mac),
        hints,
        dhcp_fingerprint: device.dhcp_requested.clone(),
        ja4: device.ja4.iter().cloned().collect(),
        user_agents: device.user_agents.iter().cloned().collect(),
        lldp_name: device.lldp_name.clone(),
        netbios_names: device.nbns_names.iter().cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_address_names_no_maker() {
        assert!(randomized("da:a1:19:00:00:01"));
        assert!(!randomized("00:1a:11:00:00:01"));
        assert_eq!(vendor("da:a1:19:00:00:01"), None);
        // 00:1A:11 is registered to Google.
        assert_eq!(vendor("00:1a:11:12:34:56"), Some("Google, Inc."));
    }

    #[test]
    fn hints_rest_on_what_was_seen() {
        assert_eq!(initial_hops(57), 64);
        assert_eq!(initial_hops(121), 128);
        assert_eq!(initial_hops(250), 255);
        let device = Device {
            dhcp_vendor: Some("MSFT 5.0".into()),
            hops: Some(128),
            ..Device::default()
        };
        let seen = profile("00:1a:11:12:34:56", &device);
        assert_eq!(seen.hints.len(), 2);
        assert!(seen.hints.iter().all(|hint| hint.says == "Windows"));
        assert!(seen.hints[0].basis.contains("MSFT 5.0"));
        assert!(profile("00:1a:11:12:34:56", &Device::default())
            .hints
            .is_empty());
    }
}
