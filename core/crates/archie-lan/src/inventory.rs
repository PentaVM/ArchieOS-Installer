//! The inventory: one entry per device on the LAN, from everything it announced.

use crate::classify::{classify, Evidence};
use crate::http::{get, LanUrl};
use crate::{dns_sd, mdns, neighbors, presence, roku, spotify, ssdp, upnp, wsdiscovery};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long the devices that answered are asked about themselves, all together.
const ASKING: Duration = Duration::from_secs(45);

/// One device, as it described itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub address: String,
    /// The device family, when what it said supports one.
    pub class: Option<String>,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    /// The mDNS service kinds it offers.
    pub services: Vec<String>,
    /// How it was found: `ssdp`, `mdns` or both.
    pub seen_by: Vec<String>,
    /// What it says it is doing, where a device offers that: a Roku's power and what is on.
    pub state: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Inventory {
    pub format: &'static str,
    pub scanned_unix: u64,
    pub devices: Vec<Device>,
}

pub const FORMAT: &str = "archie-lan-inventory-v1";

/// Puts together what was heard. `descriptions` and `states` are keyed by address: the
/// description a device published and, for a Roku, what it reported.
pub fn assemble(
    answers: &[ssdp::Answer],
    descriptions: &BTreeMap<IpAddr, upnp::Description>,
    hosts: &BTreeMap<IpAddr, mdns::Host>,
    states: &BTreeMap<IpAddr, BTreeMap<String, String>>,
    scanned_unix: u64,
) -> Inventory {
    let addresses: BTreeSet<IpAddr> = answers
        .iter()
        .map(|a| a.from)
        .chain(hosts.keys().copied())
        .collect();
    let pick = |candidates: &[&str]| {
        candidates
            .iter()
            .find(|c| !c.is_empty())
            .map(|c| c.to_string())
            .unwrap_or_default()
    };
    let devices = addresses
        .into_iter()
        .map(|address| {
            let heard: Vec<&ssdp::Answer> = answers.iter().filter(|a| a.from == address).collect();
            let description = descriptions.get(&address).cloned().unwrap_or_default();
            let host = hosts.get(&address).cloned().unwrap_or_default();
            let mut state = states.get(&address).cloned().unwrap_or_default();
            for (kind, endpoint) in host.endpoints(address) {
                state.insert(format!("{kind}.endpoint"), endpoint);
            }
            let tags: Vec<&str> = [
                ("_matter._tcp", "matter"),
                ("_matterc._udp", "matter-commissionable"),
                ("_matterd._udp", "matter-commissioner"),
                ("_hap._tcp", "homekit"),
                ("_mqtt._tcp", "mqtt"),
                ("_secure-mqtt._tcp", "mqtt-tls"),
                ("_esphomelib._tcp", "esphome"),
                ("_wled._tcp", "wled"),
                ("_shelly._tcp", "shelly"),
                ("_hue._tcp", "hue"),
                ("_adb-tls-connect._tcp", "android-debug-service"),
            ]
            .into_iter()
            .filter(|(kind, _)| host.services.contains(*kind))
            .map(|(_, tag)| tag)
            .collect();
            if !tags.is_empty() {
                state.insert("discovery_tags".into(), tags.join(","));
            }
            let fact = |key: &str| host.facts.get(key).map(String::as_str).unwrap_or("");
            let said = |key: &str| state.get(key).map(String::as_str).unwrap_or("");
            let mut evidence = Evidence {
                upnp: vec![
                    description.device_type.clone(),
                    description.manufacturer.clone(),
                    description.model_name.clone(),
                ],
                services: host.services.iter().cloned().collect(),
            };
            evidence.upnp.extend(
                heard
                    .iter()
                    .flat_map(|a| [a.target.clone(), a.server.clone()]),
            );
            evidence
                .upnp
                .iter_mut()
                .for_each(|text| *text = text.to_lowercase());
            let mut seen_by = Vec::new();
            if !heard.is_empty() {
                seen_by.push("ssdp".to_string());
            }
            if !host.services.is_empty() {
                seen_by.push("mdns".to_string());
            }
            Device {
                address: address.to_string(),
                class: classify(&evidence).map(|class| class.as_str().to_string()),
                name: pick(&[
                    said("name"),
                    &description.friendly_name,
                    fact("fn"),
                    host.name.as_deref().unwrap_or(""),
                ]),
                manufacturer: pick(&[&description.manufacturer, fact("manufacturer")]),
                model: pick(&[
                    said("model"),
                    &description.model_name,
                    fact("md"),
                    fact("ty"),
                    fact("model"),
                    fact("am"),
                ]),
                services: host.services.iter().cloned().collect(),
                seen_by,
                state,
            }
        })
        .collect();
    Inventory {
        format: FORMAT,
        scanned_unix,
        devices,
    }
}

/// Listens on both protocols for `listen`, reads the descriptions devices published, asks
/// each Roku what is on, and returns the inventory.
pub fn scan(listen: Duration) -> std::io::Result<Inventory> {
    scan_with(ScanOptions::default(), listen).map(|report| report.inventory)
}

#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    pub bind: Option<Ipv4Addr>,
    pub probe_local: bool,
    pub subnet: Option<presence::Subnet>,
    pub probe_seconds: u64,
    pub mdns_backend: dns_sd::Backend,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            bind: None,
            probe_local: false,
            subnet: None,
            probe_seconds: 15,
            mdns_backend: dns_sd::Backend::Native,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SourceStatus {
    pub available: bool,
    pub matches: usize,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Diagnostics {
    pub format: &'static str,
    pub local_address: Ipv4Addr,
    pub sources: BTreeMap<String, SourceStatus>,
    pub presence: Option<presence::Report>,
    pub limitations: Vec<&'static str>,
}

pub struct ScanReport {
    pub inventory: Inventory,
    pub diagnostics: Diagnostics,
}

fn source<T: Default>(result: std::thread::Result<std::io::Result<T>>) -> (T, Option<String>) {
    match result {
        Ok(Ok(value)) => (value, None),
        Ok(Err(error)) => (T::default(), Some(error.to_string())),
        Err(_) => (T::default(), Some("discovery worker failed".into())),
    }
}

pub fn scan_with(options: ScanOptions, listen: Duration) -> std::io::Result<ScanReport> {
    let local = match options.bind {
        Some(address) => address,
        None => crate::lan_address()?,
    };
    if !crate::is_lan_v4(local) {
        return Err(std::io::Error::other(
            "selected interface must be private or link-local",
        ));
    }
    if !if_addrs::get_if_addrs()?
        .iter()
        .any(|interface| interface.addr.ip() == IpAddr::V4(local))
    {
        return Err(std::io::Error::other(
            "selected IPv4 address is not assigned to a local interface",
        ));
    }
    let subnet = if let Some(subnet) = options.subnet {
        Some(subnet)
    } else if options.probe_local {
        Some(presence::Subnet::for_interface(local).map_err(std::io::Error::other)?)
    } else {
        None
    };
    if subnet.is_some_and(|s| !s.contains(local)) {
        return Err(std::io::Error::other(
            "probe subnet must contain the selected local interface",
        ));
    }
    let browsing = (!matches!(options.mdns_backend, dns_sd::Backend::MdnsSd))
        .then(|| std::thread::spawn(move || mdns::browse_report(local, listen)));
    let library = (!matches!(options.mdns_backend, dns_sd::Backend::Native))
        .then(|| std::thread::spawn(move || dns_sd::browse(local, listen)));
    let searching = std::thread::spawn(move || ssdp::search(local, listen));
    let wsd = std::thread::spawn(move || wsdiscovery::search(local, listen));
    let probing = subnet.map(|s| {
        std::thread::spawn(move || {
            presence::scan(
                s,
                local,
                Duration::from_secs(options.probe_seconds.clamp(1, 120)),
            )
        })
    });
    let (answers, ssdp_error) = source(searching.join());
    let (browse, mdns_error) = browsing
        .map(|worker| source(worker.join()))
        .unwrap_or_else(|| {
            (
                mdns::BrowseReport::default(),
                Some("native mDNS disabled by selected backend".into()),
            )
        });
    let (supplemental, library_error) = library
        .map(|worker| source(worker.join()))
        .unwrap_or_default();
    let (discovered, wsd_error) = source(wsd.join());
    let probe = probing.and_then(|worker| worker.join().ok());
    let mut sources = BTreeMap::new();
    sources.insert(
        "ssdp".into(),
        SourceStatus {
            available: ssdp_error.is_none(),
            matches: answers.len(),
            error: ssdp_error,
        },
    );
    sources.insert(
        "mdns".into(),
        SourceStatus {
            available: mdns_error.is_none(),
            matches: browse.hosts.len(),
            error: mdns_error,
        },
    );
    sources.insert(
        "mdns-multicast".into(),
        SourceStatus {
            available: browse.multicast,
            matches: 0,
            error: browse.warning,
        },
    );
    sources.insert(
        "ws-discovery".into(),
        SourceStatus {
            available: wsd_error.is_none(),
            matches: discovered.len(),
            error: wsd_error,
        },
    );
    if subnet.is_some() {
        sources.insert(
            "tcp-presence".into(),
            SourceStatus {
                available: probe.is_some(),
                matches: probe.as_ref().map_or(0, |p| p.responders.len()),
                error: if probe.is_none() {
                    Some("presence worker failed".into())
                } else {
                    None
                },
            },
        );
    }
    let hosts = browse.hosts;
    if !matches!(options.mdns_backend, dns_sd::Backend::Native) {
        let mut status = supplemental.status();
        if let Some(error) = library_error {
            status.error = Some(error);
            status.available = false;
        }
        sources.insert("mdns-sd".into(), status);
    }

    let mut descriptions = BTreeMap::new();
    let mut states = BTreeMap::new();
    // Devices are asked one after another, so the asking as a whole has an end too. One
    // not reached in time is still listed, from what it announced.
    let until = Instant::now() + ASKING;
    for answer in &answers {
        // A description is read only from the host that announced it: a device does not
        // get to send this node to fetch from somewhere else.
        let published = answer
            .location
            .as_deref()
            .and_then(LanUrl::parse)
            .filter(|url| url.host() == answer.from);
        if let Some(url) = published {
            if !descriptions.contains_key(&answer.from) && Instant::now() < until {
                if let Ok(body) = get(&url) {
                    descriptions.insert(answer.from, upnp::parse(&body));
                }
            }
        }
        let is_roku = answer.target == "roku:ecp" || answer.server.to_lowercase().contains("roku");
        if is_roku && !states.contains_key(&answer.from) && Instant::now() < until {
            states.insert(answer.from, roku::observe(answer.from));
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut inventory = assemble(&answers, &descriptions, &hosts, &states, now);
    dns_sd::merge(&mut inventory.devices, &supplemental.devices);
    for device in &mut inventory.devices {
        if Instant::now() >= until {
            break;
        }
        if let Some(endpoint) = device.state.get("_spotify-connect._tcp.endpoint").cloned() {
            match spotify::observe(&endpoint) {
                Ok(snapshot) => {
                    if device.class.is_none() {
                        device.class = snapshot.class;
                    }
                    device.state.extend(snapshot.state);
                    device.seen_by.push("spotify-getinfo".into());
                }
                Err(error) => {
                    device.state.insert(
                        "spotify.probe_error".into(),
                        error.chars().take(200).collect(),
                    );
                }
            }
        }
    }
    merge_observations(
        &mut inventory,
        &discovered,
        probe.as_ref().map_or(&[], |p| p.responders.as_slice()),
    );
    // Observe after active probes so fresh ARP resolutions can contribute, but
    // keep the cached status: a cached entry does not prove a live device.
    match neighbors::collect(local) {
        Ok(records) => {
            sources.insert(
                "neighbor-cache".into(),
                SourceStatus {
                    available: true,
                    matches: records.len(),
                    error: None,
                },
            );
            for neighbor in records {
                let address = neighbor.address.to_string();
                if !inventory.devices.iter().any(|d| d.address == address) {
                    inventory.devices.push(Device {
                        address: address.clone(),
                        ..Default::default()
                    });
                }
                let device = inventory
                    .devices
                    .iter_mut()
                    .find(|d| d.address == address)
                    .unwrap();
                device.seen_by.push("neighbor-cache".into());
                device.state.insert("neighbor.mac".into(), neighbor.mac);
                device
                    .state
                    .insert("neighbor.status".into(), neighbor.status);
                device.state.insert(
                    "neighbor.source".into(),
                    "system-cache; reachability unconfirmed".into(),
                );
            }
        }
        Err(error) => {
            sources.insert(
                "neighbor-cache".into(),
                SourceStatus {
                    available: false,
                    matches: 0,
                    error: Some(error),
                },
            );
        }
    }
    inventory
        .devices
        .sort_by_key(|d| d.address.parse::<IpAddr>().unwrap());
    inventory.devices.truncate(4096);
    for device in &mut inventory.devices {
        if device.address == local.to_string() {
            device
                .state
                .insert("local_address_matches".into(), "true".into());
        }
    }
    Ok(ScanReport { inventory, diagnostics: Diagnostics {
        format: "archie-lan-diagnostics-v1", local_address: local, sources, presence: probe,
        limitations: vec![if matches!(options.mdns_backend, dns_sd::Backend::Native) { "native discovery uses IPv4; IPv6-only endpoints require another source" } else { "DNS-SD observes private scoped IPv6; HTTP and TCP discovery still use IPv4" },
            "one selected network; isolated VLANs are not visible", "Zigbee/Thread devices may require hub or border-router observations",
            "silent devices without advertised services or sampled open TCP ports may remain unseen",
            "advertisements are unauthenticated device claims; ownership is unknown"] } })
}

pub fn merge_observations(
    inventory: &mut Inventory,
    discovered: &[wsdiscovery::Match],
    presence: &[(Ipv4Addr, Vec<u16>)],
) {
    for found in discovered {
        let address = found.from.to_string();
        if !inventory.devices.iter().any(|d| d.address == address) {
            inventory.devices.push(Device {
                address: address.clone(),
                ..Default::default()
            });
        }
        let device = inventory
            .devices
            .iter_mut()
            .find(|d| d.address == address)
            .unwrap();
        if device.class.is_none() {
            device.class.clone_from(&found.class);
        }
        if !device.seen_by.iter().any(|s| s == "ws-discovery") {
            device.seen_by.push("ws-discovery".into());
        }
        if let Some(endpoint) = &found.endpoint {
            device
                .state
                .entry("wsd.endpoint".into())
                .or_insert_with(|| endpoint.clone());
        }
        let types = found.types.join(",");
        if types.len() <= 256 {
            device.state.entry("wsd.types".into()).or_insert(types);
        }
    }
    for (address, ports) in presence {
        let address = address.to_string();
        if !inventory.devices.iter().any(|d| d.address == address) {
            inventory.devices.push(Device {
                address: address.clone(),
                ..Default::default()
            });
        }
        let device = inventory
            .devices
            .iter_mut()
            .find(|d| d.address == address)
            .unwrap();
        if !device.seen_by.iter().any(|s| s == "tcp-presence") {
            device.seen_by.push("tcp-presence".into());
        }
        device.state.insert(
            "open_tcp_ports".into(),
            ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    inventory
        .devices
        .sort_by_key(|d| d.address.parse::<IpAddr>().unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(from: &str, target: &str, server: &str) -> ssdp::Answer {
        ssdp::Answer {
            from: from.parse().unwrap(),
            location: None,
            target: target.into(),
            server: server.into(),
            usn: String::new(),
        }
    }

    #[test]
    fn a_device_found_two_ways_is_one_device_with_everything_it_said() {
        let tv: IpAddr = "192.168.1.40".parse().unwrap();
        let printer: IpAddr = "192.168.1.60".parse().unwrap();
        let answers = [
            answer("192.168.1.40", "roku:ecp", "Roku/12.5.0 UPnP/1.0"),
            answer("192.168.1.40", "upnp:rootdevice", "Roku/12.5.0"),
        ];
        let descriptions = BTreeMap::from([(
            tv,
            upnp::Description {
                friendly_name: "Roku Express".into(),
                manufacturer: "Roku".into(),
                model_name: "3941X".into(),
                ..Default::default()
            },
        )]);
        let hosts = BTreeMap::from([
            (
                tv,
                mdns::Host {
                    services: BTreeSet::from(["_airplay._tcp".to_string()]),
                    name: Some("Living".into()),
                    facts: BTreeMap::new(),
                    ..Default::default()
                },
            ),
            (
                printer,
                mdns::Host {
                    services: BTreeSet::from(["_ipp._tcp".to_string()]),
                    name: Some("Oficina".into()),
                    facts: BTreeMap::from([("ty".to_string(), "Brother HL-L2350DW".to_string())]),
                    ..Default::default()
                },
            ),
        ]);
        let states = BTreeMap::from([(
            tv,
            BTreeMap::from([
                ("name".to_string(), "Living".to_string()),
                ("showing".to_string(), "Netflix".to_string()),
            ]),
        )]);

        let inventory = assemble(&answers, &descriptions, &hosts, &states, 1_790_000_000);
        assert_eq!(inventory.devices.len(), 2);
        let television = &inventory.devices[0];
        assert_eq!(television.class.as_deref(), Some("television"));
        assert_eq!(
            (
                television.name.as_str(),
                television.manufacturer.as_str(),
                television.model.as_str()
            ),
            ("Living", "Roku", "3941X")
        );
        assert_eq!(television.seen_by, ["ssdp", "mdns"]);
        assert_eq!(television.state["showing"], "Netflix");
        let office = &inventory.devices[1];
        assert_eq!(office.class.as_deref(), Some("printer"));
        assert_eq!(
            (office.name.as_str(), office.model.as_str()),
            ("Oficina", "Brother HL-L2350DW")
        );
        assert_eq!(office.seen_by, ["mdns"]);

        let json = serde_json::to_value(&inventory).unwrap();
        assert_eq!(json["format"], FORMAT);
        assert_eq!(json["devices"][1]["address"], "192.168.1.60");
    }

    #[test]
    fn a_device_that_only_said_it_exists_is_listed_without_a_kind() {
        let inventory = assemble(
            &[answer("10.0.0.9", "upnp:rootdevice", "Linux UPnP/1.0")],
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            0,
        );
        assert_eq!(inventory.devices[0].class, None);
        assert_eq!(inventory.devices[0].name, "");
    }

    #[test]
    fn independent_source_failure_retains_an_available_ssdp_observation() {
        let (hosts, error) = source::<BTreeMap<IpAddr, mdns::Host>>(Ok(Err(
            std::io::Error::other("mDNS source failed"),
        )));
        assert_eq!(error.as_deref(), Some("mDNS source failed"));
        let (answers, error) = source(Ok(Ok(vec![answer(
            "10.0.0.9",
            "upnp:rootdevice",
            "generic",
        )])));
        assert!(error.is_none());
        let inventory = assemble(&answers, &BTreeMap::new(), &hosts, &BTreeMap::new(), 42);
        assert_eq!(inventory.devices.len(), 1);
        assert_eq!(inventory.devices[0].address, "10.0.0.9");
        assert_eq!(inventory.devices[0].seen_by, ["ssdp"]);
    }
}
