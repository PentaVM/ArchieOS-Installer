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

/// The highest TLS version a client offered, as its JA4 says in the second and third
/// characters ("t13…" is TLS 1.3 over TCP, "q13…" over QUIC).
pub fn tls_version(ja4: &str) -> Option<&'static str> {
    Some(match ja4.get(1..3)? {
        "13" => "TLS 1.3",
        "12" => "TLS 1.2",
        "11" => "TLS 1.1",
        "10" => "TLS 1.0",
        "s3" => "SSL 3.0",
        "s2" => "SSL 2.0",
        _ => return None,
    })
}

/// Whether the first label of a DNS name looks generated rather than chosen: 10 or more
/// characters, spread over many symbols (Shannon entropy of 3 bits or more) and few vowels
/// (under 30 % of its letters). Words, brands and tracker names rarely pass both; random
/// strings mostly do. A hint for counting, not a verdict on one name.
pub fn random_label(name: &str) -> bool {
    let label = name.split('.').next().unwrap_or("");
    if label.len() < 10 || label.len() > 63 {
        return false;
    }
    let mut counts = [0u32; 256];
    for byte in label.bytes() {
        counts[usize::from(byte.to_ascii_lowercase())] += 1;
    }
    let length = label.len() as f64;
    let entropy: f64 = counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(c) / length;
            -p * p.log2()
        })
        .sum();
    let letters = label.bytes().filter(u8::is_ascii_alphabetic).count();
    let vowels = label
        .bytes()
        .filter(|b| matches!(b.to_ascii_lowercase(), b'a' | b'e' | b'i' | b'o' | b'u'))
        .count();
    letters > 0 && entropy >= 3.0 && (vowels as f64) < 0.3 * letters as f64
}

/// Versions RFC 8996 (2021) retired: a client that offers nothing newer runs a stack
/// that has not been updated in years.
pub fn retired_tls(version: &str) -> bool {
    matches!(version, "TLS 1.1" | "TLS 1.0" | "SSL 3.0" | "SSL 2.0")
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
    /// The highest TLS version each of its clients offered, from `ja4`.
    pub tls_versions: Vec<&'static str>,
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
        tls_versions: {
            let mut versions: Vec<&'static str> =
                device.ja4.iter().filter_map(|j| tls_version(j)).collect();
            versions.sort_unstable();
            versions.dedup();
            versions
        },
        user_agents: device.user_agents.iter().cloned().collect(),
        lldp_name: device.lldp_name.clone(),
        netbios_names: device.nbns_names.iter().cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_labels_look_random_and_chosen_ones_do_not() {
        for name in ["kqzvtrwbxlmp.com", "x7k2qz9vbwtp.net", "PMXWQHZJTCRB.org"] {
            assert!(random_label(name), "{name}");
        }
        for name in [
            "securepubads.g.doubleclick.net",
            "googletagmanager.com",
            "scorecardresearch.com",
            "clientservices.googleapis.com",
            "kqzvtrwb.com",
            "",
        ] {
            assert!(!random_label(name), "{name}");
        }
    }

    #[test]
    fn a_random_address_names_no_maker() {
        assert!(randomized("da:a1:19:00:00:01"));
        assert!(!randomized("00:1a:11:00:00:01"));
        assert_eq!(vendor("da:a1:19:00:00:01"), None);
        // 00:1A:11 is registered to Google.
        assert_eq!(vendor("00:1a:11:12:34:56"), Some("Google, Inc."));
    }

    #[test]
    fn the_tls_version_is_read_from_ja4() {
        assert_eq!(
            tls_version("t13d1516h2_8daaf6152771_02713d6af862"),
            Some("TLS 1.3")
        );
        assert_eq!(
            tls_version("q13d0312h3_55b375c5d22e_06cda9e17597"),
            Some("TLS 1.3")
        );
        assert_eq!(
            tls_version("t10d030100_5704bdfc91c6_e3b0c44298fc"),
            Some("TLS 1.0")
        );
        assert_eq!(
            tls_version("ts3d020000_000000000000_000000000000"),
            Some("SSL 3.0")
        );
        assert_eq!(tls_version("t"), None);
        assert!(retired_tls("TLS 1.0") && !retired_tls("TLS 1.2"));
        let device = Device {
            ja4: ["t13d1516h2_a_b", "t10d030100_c_d", "t13d1516h2_e_f"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ..Device::default()
        };
        assert_eq!(
            profile("00:1a:11:12:34:56", &device).tls_versions,
            ["TLS 1.0", "TLS 1.3"]
        );
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
