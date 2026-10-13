//! Bounded, correlated WS-Discovery observation. No metadata URL is fetched.
use crate::{http::LanUrl, is_lan};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const OLD: &str = "http://schemas.xmlsoap.org/ws/2005/04/discovery";
const NEW: &str = "http://docs.oasis-open.org/ws-dd/ns/discovery/2009/01";
const SOAP: &str = "http://www.w3.org/2003/05/soap-envelope";
const A_OLD: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing";
const A_NEW: &str = "http://www.w3.org/2005/08/addressing";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub from: IpAddr,
    pub class: Option<String>,
    pub types: Vec<String>,
    pub endpoint: Option<String>,
}

pub fn parse(from: IpAddr, packet: &[u8], request_id: &str) -> Vec<Match> {
    if !is_lan(from) || packet.len() > 65_536 {
        return Vec::new();
    }
    let Some(doc) = std::str::from_utf8(packet)
        .ok()
        .and_then(|xml| roxmltree::Document::parse(xml).ok())
    else {
        return Vec::new();
    };
    let root = doc.root_element();
    if !root.has_tag_name((SOAP, "Envelope")) {
        return Vec::new();
    }
    let related = root
        .children()
        .filter(|n| n.has_tag_name((SOAP, "Header")))
        .flat_map(|n| n.children())
        .any(|n| {
            (n.has_tag_name((A_OLD, "RelatesTo")) || n.has_tag_name((A_NEW, "RelatesTo")))
                && n.text().is_some_and(|text| text.trim() == request_id)
        });
    if !related {
        return Vec::new();
    }
    let mut out = Vec::new();
    for body in root.children().filter(|n| n.has_tag_name((SOAP, "Body"))) {
        for matches in body.children().filter(|n| {
            n.has_tag_name((OLD, "ProbeMatches")) || n.has_tag_name((NEW, "ProbeMatches"))
        }) {
            let ns = matches.tag_name().namespace().unwrap();
            for entry in matches
                .children()
                .filter(|n| n.has_tag_name((ns, "ProbeMatch")))
                .take(32)
            {
                let mut types = Vec::new();
                let mut class = None;
                for node in entry.children().filter(|n| n.has_tag_name((ns, "Types"))) {
                    for token in node.text().unwrap_or_default().split_whitespace().take(16) {
                        let (prefix, local) = token
                            .split_once(':')
                            .map_or((None, token), |(p, n)| (Some(p), n));
                        let uri = node.lookup_namespace_uri(prefix).unwrap_or_default();
                        if uri == "http://schemas.microsoft.com/windows/2006/08/wdp/print"
                            && matches!(local, "PrintDeviceType" | "PrintServiceType")
                        {
                            class = Some("printer".to_string());
                        } else if uri == "http://www.onvif.org/ver10/network/wsdl"
                            && local == "NetworkVideoTransmitter"
                            && class.is_none()
                        {
                            class = Some("camera".to_string());
                        }
                        let expanded = format!("{{{uri}}}{local}");
                        if expanded.len() <= 256 && types.len() < 16 {
                            types.push(expanded);
                        }
                    }
                }
                let endpoint = entry
                    .children()
                    .filter(|n| n.has_tag_name((ns, "XAddrs")))
                    .filter_map(|n| n.text())
                    .flat_map(str::split_whitespace)
                    .find(|raw| {
                        raw.len() <= 256 && LanUrl::parse(raw).is_some_and(|url| url.host() == from)
                    })
                    .map(str::to_string);
                let found = Match {
                    from,
                    class,
                    types,
                    endpoint,
                };
                if !out.contains(&found) {
                    out.push(found);
                }
            }
        }
    }
    out
}

fn probe(id: &str, modern: bool) -> String {
    let (d, a, to) = if modern {
        (
            NEW,
            A_NEW,
            "urn:docs-oasis-open-org:ws-dd:ns:discovery:2009:01",
        )
    } else {
        (OLD, A_OLD, "urn:schemas-xmlsoap-org:ws:2005:04:discovery")
    };
    format!(
        r#"<s:Envelope xmlns:s="{SOAP}" xmlns:a="{a}" xmlns:d="{d}"><s:Header>
      <a:MessageID>{id}</a:MessageID><a:To>{to}</a:To><a:Action>{d}/Probe</a:Action>
      <a:ReplyTo><a:Address>{a}/anonymous</a:Address></a:ReplyTo>
      </s:Header><s:Body><d:Probe/></s:Body></s:Envelope>"#
    )
}

pub fn search(local: Ipv4Addr, listen: Duration) -> std::io::Result<Vec<Match>> {
    let socket = UdpSocket::bind((local, 0))?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    socket.set_multicast_ttl_v4(1)?;
    let group = SocketAddr::from((Ipv4Addr::new(239, 255, 255, 250), 3702));
    let requests = [
        (format!("urn:uuid:{}", uuid::Uuid::new_v4()), false),
        (format!("urn:uuid:{}", uuid::Uuid::new_v4()), true),
    ];
    for (id, modern) in &requests {
        socket.send_to(probe(id, *modern).as_bytes(), group)?;
    }
    let until = Instant::now() + listen;
    let mut retry = Some(Instant::now() + Duration::from_secs(1));
    let mut out = Vec::new();
    let mut buffer = [0u8; 65_536];
    let mut packets = 0;
    while Instant::now() < until && packets < 2048 && out.len() < 1024 {
        if retry.is_some_and(|at| Instant::now() >= at) {
            for (id, modern) in &requests {
                socket.send_to(probe(id, *modern).as_bytes(), group)?;
            }
            retry = None;
        }
        match socket.recv_from(&mut buffer) {
            Ok((length, from)) => {
                packets += 1;
                for (id, _) in &requests {
                    for found in parse(from.ip(), &buffer[..length], id) {
                        if out.len() < 1024 && !out.contains(&found) {
                            out.push(found);
                        }
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}
