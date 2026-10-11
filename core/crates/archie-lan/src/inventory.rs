//! The inventory: one entry per device on the LAN, from everything it announced.

use crate::classify::{classify, Evidence};
use crate::http::{get, LanUrl};
use crate::{mdns, roku, ssdp, upnp};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long the devices that answered are asked about themselves, all together.
const ASKING: Duration = Duration::from_secs(45);

/// One device, as it described itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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
    let addresses: BTreeSet<IpAddr> = answers.iter().map(|a| a.from).chain(hosts.keys().copied()).collect();
    let pick = |candidates: &[&str]| candidates.iter().find(|c| !c.is_empty()).map(|c| c.to_string()).unwrap_or_default();
    let devices = addresses
        .into_iter()
        .map(|address| {
            let heard: Vec<&ssdp::Answer> = answers.iter().filter(|a| a.from == address).collect();
            let description = descriptions.get(&address).cloned().unwrap_or_default();
            let host = hosts.get(&address).cloned().unwrap_or_default();
            let state = states.get(&address).cloned().unwrap_or_default();
            let fact = |key: &str| host.facts.get(key).map(String::as_str).unwrap_or("");
            let said = |key: &str| state.get(key).map(String::as_str).unwrap_or("");
            let mut evidence = Evidence {
                upnp: vec![description.device_type.clone(), description.manufacturer.clone(), description.model_name.clone()],
                services: host.services.iter().cloned().collect(),
            };
            evidence.upnp.extend(heard.iter().flat_map(|a| [a.target.clone(), a.server.clone()]));
            evidence.upnp.iter_mut().for_each(|text| *text = text.to_lowercase());
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
                name: pick(&[said("name"), &description.friendly_name, fact("fn"), host.name.as_deref().unwrap_or("")]),
                manufacturer: pick(&[&description.manufacturer, fact("manufacturer")]),
                model: pick(&[said("model"), &description.model_name, fact("md"), fact("ty"), fact("model"), fact("am")]),
                services: host.services.iter().cloned().collect(),
                seen_by,
                state,
            }
        })
        .collect();
    Inventory { format: FORMAT, scanned_unix, devices }
}

/// Listens on both protocols for `listen`, reads the descriptions devices published, asks
/// each Roku what is on, and returns the inventory.
pub fn scan(listen: Duration) -> std::io::Result<Inventory> {
    let local = crate::lan_address()?;
    let browsing = std::thread::spawn(move || mdns::browse(local, listen));
    let answers = ssdp::search(local, listen)?;
    let hosts = browsing.join().unwrap_or_else(|_| Ok(BTreeMap::new()))?;

    let mut descriptions = BTreeMap::new();
    let mut states = BTreeMap::new();
    // Devices are asked one after another, so the asking as a whole has an end too. One
    // not reached in time is still listed, from what it announced.
    let until = Instant::now() + ASKING;
    for answer in &answers {
        // A description is read only from the host that announced it: a device does not
        // get to send this node to fetch from somewhere else.
        let published = answer.location.as_deref().and_then(LanUrl::parse).filter(|url| url.host() == answer.from);
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
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok(assemble(&answers, &descriptions, &hosts, &states, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(from: &str, target: &str, server: &str) -> ssdp::Answer {
        ssdp::Answer { from: from.parse().unwrap(), location: None, target: target.into(), server: server.into(), usn: String::new() }
    }

    #[test]
    fn a_device_found_two_ways_is_one_device_with_everything_it_said() {
        let tv: IpAddr = "192.168.1.40".parse().unwrap();
        let printer: IpAddr = "192.168.1.60".parse().unwrap();
        let answers = [answer("192.168.1.40", "roku:ecp", "Roku/12.5.0 UPnP/1.0"), answer("192.168.1.40", "upnp:rootdevice", "Roku/12.5.0")];
        let descriptions = BTreeMap::from([(
            tv,
            upnp::Description { friendly_name: "Roku Express".into(), manufacturer: "Roku".into(), model_name: "3941X".into(), ..Default::default() },
        )]);
        let hosts = BTreeMap::from([
            (tv, mdns::Host { services: BTreeSet::from(["_airplay._tcp".to_string()]), name: Some("Living".into()), facts: BTreeMap::new() }),
            (
                printer,
                mdns::Host {
                    services: BTreeSet::from(["_ipp._tcp".to_string()]),
                    name: Some("Oficina".into()),
                    facts: BTreeMap::from([("ty".to_string(), "Brother HL-L2350DW".to_string())]),
                },
            ),
        ]);
        let states = BTreeMap::from([(tv, BTreeMap::from([("name".to_string(), "Living".to_string()), ("showing".to_string(), "Netflix".to_string())]))]);

        let inventory = assemble(&answers, &descriptions, &hosts, &states, 1_790_000_000);
        assert_eq!(inventory.devices.len(), 2);
        let television = &inventory.devices[0];
        assert_eq!(television.class.as_deref(), Some("television"));
        assert_eq!((television.name.as_str(), television.manufacturer.as_str(), television.model.as_str()), ("Living", "Roku", "3941X"));
        assert_eq!(television.seen_by, ["ssdp", "mdns"]);
        assert_eq!(television.state["showing"], "Netflix");
        let office = &inventory.devices[1];
        assert_eq!(office.class.as_deref(), Some("printer"));
        assert_eq!((office.name.as_str(), office.model.as_str()), ("Oficina", "Brother HL-L2350DW"));
        assert_eq!(office.seen_by, ["mdns"]);

        let json = serde_json::to_value(&inventory).unwrap();
        assert_eq!(json["format"], FORMAT);
        assert_eq!(json["devices"][1]["address"], "192.168.1.60");
    }

    #[test]
    fn a_device_that_only_said_it_exists_is_listed_without_a_kind() {
        let inventory = assemble(&[answer("10.0.0.9", "upnp:rootdevice", "Linux UPnP/1.0")], &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new(), 0);
        assert_eq!(inventory.devices[0].class, None);
        assert_eq!(inventory.devices[0].name, "");
    }
}
