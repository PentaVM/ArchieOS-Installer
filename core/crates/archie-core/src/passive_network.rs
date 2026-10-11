//! Offline, bounded mDNS metadata ingestion. No capture, sockets, trust or pairing.
//! Input is TShark fields: ip.src, udp.srcport, dns.ptr.domain_name (tab-separated).
use crate::control::{
    ClassifiedNetworkAsset, NetworkAssetObservation, NetworkEvidence, NetworkEvidenceSource,
    NetworkInventoryChange, NmapSurveyPlan, NmapSurveyProfile, SourcedNetworkEvidence,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::Ipv4Addr,
};

pub const MAX_EXPORT_BYTES: usize = 1_048_576;

/// A packet advertisement is untrusted analytic evidence, including when read from
/// a local file. Unsupported names are discarded; malformed framing fails closed.
pub fn observations(input: &str) -> Result<Vec<NetworkAssetObservation>, &'static str> {
    if input.len() > MAX_EXPORT_BYTES {
        return Err("network export too large");
    }
    let mut hosts: BTreeMap<Ipv4Addr, BTreeSet<NetworkEvidence>> = BTreeMap::new();
    for (line_number, line) in input.lines().enumerate() {
        if line_number >= 8192 || line.len() > 4096 {
            return Err("network export bounds exceeded");
        }
        if line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 3 {
            return Err("expected three TShark fields");
        }
        let ip: Ipv4Addr = fields[0].parse().map_err(|_| "invalid IPv4 source")?;
        if fields[1] != "5353" {
            return Err("expected mDNS source port");
        }
        let evidence = hosts.entry(ip).or_default();
        for name in fields[2].split(',') {
            if name.len() > 255 || name.chars().any(char::is_control) {
                return Err("invalid mDNS name");
            }
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            let services = [
                ("_roku-ecp._tcp.local", NetworkEvidence::MdnsRoku),
                ("_googlecast._tcp.local", NetworkEvidence::MdnsGoogleCast),
                ("_ipp._tcp.local", NetworkEvidence::MdnsIpp),
                ("_ipps._tcp.local", NetworkEvidence::MdnsIpp),
                ("_rtsp._tcp.local", NetworkEvidence::MdnsRtsp),
                (
                    "_home-assistant._tcp.local",
                    NetworkEvidence::MdnsHomeAssistant,
                ),
            ];
            for (service, signal) in services {
                if name == service
                    || name
                        .strip_suffix(service)
                        .is_some_and(|prefix| prefix.ends_with('.'))
                {
                    evidence.insert(signal);
                }
            }
        }
        if hosts.len() > 4096 {
            return Err("too many observed hosts");
        }
    }
    Ok(hosts
        .into_iter()
        .map(|(ipv4_address, signals)| NetworkAssetObservation {
            ipv4_address,
            evidence: signals
                .into_iter()
                .map(|evidence| SourcedNetworkEvidence {
                    source: NetworkEvidenceSource::PassiveMdns,
                    evidence,
                })
                .collect(),
        })
        .collect())
}

/// Reuses Core's scope and source validation. This plan is never executed.
pub fn analyze(
    scope: Vec<String>,
    current: &str,
    baseline: Option<&str>,
) -> Result<(Vec<ClassifiedNetworkAsset>, Vec<NetworkInventoryChange>), String> {
    let plan = NmapSurveyPlan::build(NmapSurveyProfile::HostDiscovery, scope)
        .map_err(|e| e.to_string())?;
    let current = observations(current).map_err(str::to_owned)?;
    let assets = plan
        .classify_observations(&current)
        .map_err(|e| e.to_string())?;
    let changes = match baseline {
        Some(input) => plan
            .compare_observations(&observations(input).map_err(str::to_owned)?, &current)
            .map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    Ok((assets, changes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merges_packets_discards_raw_names_and_reuses_scoped_classifier() {
        let input = "192.168.1.2\t5353\tOffice._ipp._tcp.local,Printer._ipps._tcp.local\n192.168.1.2\t5353\tprivate._unknown._tcp.local\n";
        let observations = observations(input).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].evidence.len(), 1);
        let (assets, _) = analyze(vec!["192.168.1.0/24".into()], input, None).unwrap();
        assert_eq!(assets.len(), 1);
        assert!(analyze(vec!["192.168.2.0/24".into()], input, None).is_err());
    }
    #[test]
    fn malformed_and_oversized_exports_are_rejected() {
        for bad in [
            "8.8.8.8\t5353\tx",
            "192.168.1.2\t22\tx",
            "192.168.1.2\t5353\tx\textra",
            "bad\t5353\tx",
        ] {
            assert!(analyze(vec!["192.168.1.0/24".into()], bad, None).is_err());
        }
        assert!(observations(&"x".repeat(MAX_EXPORT_BYTES + 1)).is_err());
        assert!(observations(&"\n".repeat(8193)).is_err());
    }
    #[test]
    fn spoofed_suffix_is_not_recognized_and_absence_is_only_an_observation() {
        let old = "192.168.1.2\t5353\tfoo_googlecast._tcp.local\n";
        assert!(observations(old).unwrap()[0].evidence.is_empty());
        let (_, changes) = analyze(vec!["192.168.1.0/24".into()], "", Some(old)).unwrap();
        assert!(matches!(
            changes.as_slice(),
            [NetworkInventoryChange::NoLongerObserved { .. }]
        ));
    }
}
