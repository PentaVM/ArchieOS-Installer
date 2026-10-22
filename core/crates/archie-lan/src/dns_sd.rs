//! Supplemental upstream DNS-SD observer. Advertised addresses are claims;
//! conversion never produces actuator endpoints or grants.
use crate::{
    classify::{classify, Evidence},
    inventory::Device,
    is_lan,
};
use mdns_sd::{
    DaemonEvent, IfKind, InterfaceId, ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum Backend {
    #[default]
    Native,
    MdnsSd,
    Hybrid,
}

pub struct Scope {
    network: u32,
    mask: u32,
    name: String,
    index: u32,
}
impl Scope {
    pub fn new(local: Ipv4Addr, mask: Ipv4Addr, name: &str, index: u32) -> Self {
        Self {
            network: u32::from(local) & u32::from(mask),
            mask: mask.into(),
            name: name.into(),
            index,
        }
    }
    fn interface_matches(&self, id: &InterfaceId) -> bool {
        if self.index != 0 {
            id.index == self.index
        } else {
            !self.name.is_empty() && id.name == self.name
        }
    }
    fn accepts(&self, ip: &ScopedIp) -> bool {
        if !is_lan(ip.to_ip_addr()) {
            return false;
        }
        match ip {
            ScopedIp::V4(v4) => {
                u32::from(*v4.addr()) & self.mask == self.network
                    && v4
                        .interface_ids()
                        .iter()
                        .any(|id| self.interface_matches(id))
            }
            ScopedIp::V6(v6) => self.interface_matches(v6.scope_id()),
            _ => false,
        }
    }
}

fn bounded(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(200).collect()
}

pub fn convert(info: &ResolvedService, scope: &Scope) -> Vec<Device> {
    let domain = info.ty_domain.to_ascii_lowercase();
    let Some(kind) = domain.strip_suffix(".local.") else {
        return Vec::new();
    };
    if !kind.starts_with('_')
        || !(kind.ends_with("._tcp") || kind.ends_with("._udp"))
        || kind.len() > 80
    {
        return Vec::new();
    }
    let name = info
        .fullname
        .strip_suffix(&info.ty_domain)
        .unwrap_or("")
        .trim_end_matches('.');
    if name.is_empty() {
        return Vec::new();
    }
    let fact = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| info.get_property_val_str(key).filter(|v| !v.is_empty()))
            .map(bounded)
            .unwrap_or_default()
    };
    let class = classify(&Evidence {
        upnp: Vec::new(),
        services: vec![kind.to_string()],
    })
    .map(|c| c.as_str().into());
    // One host publishes an IPv4 address and often link-local and private IPv6 ones too;
    // it is one device, listed once: by its IPv4 address when it has one in scope, else by
    // its lowest IPv6 address (private before link-local).
    let address = info
        .addresses
        .iter()
        .filter(|ip| scope.accepts(ip))
        .map(ScopedIp::to_ip_addr)
        .min_by_key(|ip| (ip.is_ipv6(), *ip));
    address
        .into_iter()
        .map(|ip| Device {
            address: ip.to_string(),
            class: class.clone(),
            name: bounded(name),
            model: fact(&["ty", "model", "md", "am"]),
            manufacturer: fact(&["manufacturer"]),
            services: vec![kind.into()],
            seen_by: vec!["mdns-sd".into()],
            state: BTreeMap::from([
                ("mdns.host".into(), bounded(&info.host)),
                ("mdns.port".into(), info.port.to_string()),
                ("mdns.interface_index".into(), scope.index.to_string()),
                (
                    "mdns.address_source".into(),
                    "advertised A/AAAA; peer unauthenticated".into(),
                ),
            ]),
        })
        .collect()
}

#[derive(Default)]
pub struct Report {
    pub devices: Vec<Device>,
    pub warnings: Vec<String>,
    pub registered: BTreeSet<IpAddr>,
}

impl Report {
    pub fn status(&self) -> crate::inventory::SourceStatus {
        let mut warnings = self.warnings.clone();
        if self.devices.is_empty() {
            warnings.push("DNS-SD reception unconfirmed: no valid service resolution; empty network or unavailable sockets/multicast".into());
        }
        crate::inventory::SourceStatus {
            // Some bind/join failures are swallowed by upstream. Constructor success,
            // no Error event, and even IpAdd do not prove working multicast reception.
            available: !self.devices.is_empty(),
            matches: self.devices.len(),
            error: (!warnings.is_empty()).then(|| warnings.join("; ")),
        }
    }
}

/// The constructor can succeed before socket errors arrive. Monitor those errors,
/// and always shut down the observer's own thread on every exit path.
pub fn browse(local: Ipv4Addr, listen: Duration) -> std::io::Result<Report> {
    let interfaces = if_addrs::get_if_addrs()?;
    let interface = interfaces
        .iter()
        .find(|i| i.addr.ip() == IpAddr::V4(local))
        .cloned()
        .ok_or_else(|| std::io::Error::other("DNS-SD local interface unavailable"))?;
    let if_addrs::IfAddr::V4(address) = &interface.addr else {
        unreachable!()
    };
    let scope = Scope::new(
        local,
        address.netmask,
        &interface.name,
        interface.index.unwrap_or_default(),
    );
    let daemon = ServiceDaemon::new().map_err(std::io::Error::other)?;
    let run = || -> Result<Report, mdns_sd::Error> {
        let monitor = daemon.monitor()?;
        daemon.disable_interface(IfKind::All)?;
        daemon.enable_interface(IfKind::Name(interface.name.clone()))?;
        daemon.set_ip_check_interval(5)?;
        let mut receivers = Vec::new();
        let mut kinds = BTreeSet::new();
        for kind in crate::mdns::service_types() {
            let kind = format!("{kind}.");
            receivers.push(daemon.browse(&kind)?);
            kinds.insert(kind);
        }
        let until = Instant::now() + listen;
        let mut report = Report::default();
        let mut packets = 0usize;
        while Instant::now() < until && packets < 8192 {
            let mut new_kinds = Vec::new();
            for receiver in &receivers {
                for _ in 0..32 {
                    let Ok(event) = receiver.try_recv() else {
                        break;
                    };
                    packets += 1;
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            for found in convert(&info, &scope) {
                                if report.devices.len() < 4096 {
                                    report.devices.retain(|d| {
                                        !(d.address == found.address
                                            && d.services == found.services)
                                    });
                                    report.devices.push(found);
                                }
                            }
                        }
                        ServiceEvent::ServiceFound(kind, fullname)
                            if kind == "_services._dns-sd._udp.local." =>
                        {
                            if fullname.len() <= 80
                                && fullname.starts_with('_')
                                && (fullname.ends_with("._tcp.local.")
                                    || fullname.ends_with("._udp.local."))
                            {
                                new_kinds.push(fullname);
                            }
                        }
                        ServiceEvent::ServiceRemoved(kind, fullname) => {
                            let name = fullname
                                .strip_suffix(&kind)
                                .unwrap_or("")
                                .trim_end_matches('.');
                            let service = kind.trim_end_matches('.').trim_end_matches(".local");
                            report.devices.retain(|d| {
                                !(d.name == name && d.services.iter().any(|s| s == service))
                            });
                        }
                        _ => {}
                    }
                }
            }
            for kind in new_kinds {
                if kinds.len() < 64 && kinds.insert(kind.clone()) {
                    receivers.push(daemon.browse(&kind)?);
                }
            }
            for _ in 0..32 {
                let Ok(event) = monitor.try_recv() else { break };
                match event {
                    DaemonEvent::Error(error) if report.warnings.len() < 8 => {
                        report.warnings.push(bounded(&error.to_string()))
                    }
                    DaemonEvent::IpAdd(ip) => {
                        report.registered.insert(ip);
                    }
                    DaemonEvent::IpDel(ip) => {
                        report.registered.remove(&ip);
                    }
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !report.registered.contains(&IpAddr::V4(local)) {
            report
                .warnings
                .push("selected IPv4 socket/interface registration not confirmed".into());
        }
        let expects_v6 = interfaces.iter().any(|i| {
            i.name == interface.name && i.addr.ip().is_ipv6() && crate::is_lan(i.addr.ip())
        });
        if expects_v6
            || report
                .devices
                .iter()
                .any(|d| d.address.parse::<IpAddr>().is_ok_and(|ip| ip.is_ipv6()))
        {
            let registered = interfaces.iter().any(|i| {
                i.name == interface.name
                    && i.addr.ip().is_ipv6()
                    && crate::is_lan(i.addr.ip())
                    && report.registered.contains(&i.addr.ip())
            });
            report.warnings.push(if registered {
                "IPv6 interface registered; upstream does not prove multicast join/delivery (AAAA can arrive over IPv4)"
            } else { "IPv6 socket/interface registration unconfirmed; advertised AAAA does not prove IPv6 transport coverage" }.into());
        }
        Ok(report)
    };
    let result = run();
    if let Ok(stopped) = daemon.shutdown() {
        let _ = stopped.recv_timeout(Duration::from_secs(2));
    }
    result.map_err(std::io::Error::other)
}

pub fn merge(target: &mut Vec<Device>, supplemental: &[Device]) {
    for found in supplemental {
        if let Some(device) = target.iter_mut().find(|d| d.address == found.address) {
            if device.class.is_none() {
                device.class.clone_from(&found.class);
            }
            for (slot, value) in [
                (&mut device.name, &found.name),
                (&mut device.model, &found.model),
                (&mut device.manufacturer, &found.manufacturer),
            ] {
                if slot.is_empty() {
                    slot.clone_from(value);
                }
            }
            for service in &found.services {
                if device.services.len() < 64 && !device.services.contains(service) {
                    device.services.push(service.clone());
                }
            }
            if !device.seen_by.iter().any(|s| s == "mdns-sd") {
                device.seen_by.push("mdns-sd".into());
            }
            for (key, value) in &found.state {
                if device.state.len() < 64 {
                    device
                        .state
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
            }
        } else if target.len() < 4096 {
            target.push(found.clone());
        }
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    #[test]
    fn silent_async_socket_failure_cannot_be_reported_available() {
        let report = Report::default();
        assert!(!report.status().available);
        assert!(report
            .status()
            .error
            .unwrap()
            .contains("reception unconfirmed"));
    }
    #[test]
    fn interface_registration_alone_is_not_receive_coverage() {
        let report = Report {
            registered: ["10.0.0.5".parse().unwrap()].into(),
            ..Default::default()
        };
        assert!(!report.status().available);
    }
}
