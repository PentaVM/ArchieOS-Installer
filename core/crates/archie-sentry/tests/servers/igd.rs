//! A UPnP IGD `WANIPConnection:1` control point over SOAP 1.1 with a real
//! mapping table: AddPortMapping (718 when another client holds the port),
//! DeletePortMapping (714 when absent), GetSpecificPortMappingEntry (714),
//! GetGenericPortMappingEntry (713 past the end) and GetStatusInfo. Faults
//! come back as HTTP 500 with a UPnPError body, as gateways send them.
use super::http::{HttpServer, Request, Response};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    time::Instant,
};

pub const SERVICE: &str = "urn:schemas-upnp-org:service:WANIPConnection:1";
pub const CONTROL_PATH: &str = "/upnp/control/WANIPConnection1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub internal_port: u16,
    pub internal_client: Ipv4Addr,
    pub enabled: bool,
    pub description: String,
}

#[derive(Debug)]
pub struct IgdState {
    /// (external port, "TCP"|"UDP") → mapping.
    pub table: BTreeMap<(u16, String), Mapping>,
    pub connected: bool,
    started: Instant,
}

pub struct Gateway {
    pub control_url: String,
    pub state: Arc<Mutex<IgdState>>,
    server: HttpServer,
}
impl Gateway {
    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(IgdState {
            table: BTreeMap::new(),
            connected: true,
            started: Instant::now(),
        }));
        let s = state.clone();
        let server = HttpServer::start(Arc::new(move |r: &Request| handle(&s, r))).await;
        Self {
            control_url: format!("{}{CONTROL_PATH}", server.base),
            state,
            server,
        }
    }
    pub fn log(&self) -> Vec<String> {
        self.server.log()
    }
}

/// Text of `<…Name>…</…Name>` regardless of namespace prefix.
fn arg(xml: &str, name: &str) -> Option<String> {
    let open = xml.find(&format!("{name}>"))?;
    let start = open + name.len() + 1;
    let end = xml[start..].find('<')? + start;
    Some(xml[start..end].trim().to_owned())
}

fn envelope(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>{body}</s:Body></s:Envelope>"
    )
}
fn ok(action: &str, inner: &str) -> Response {
    Response::new(
        200,
        "text/xml; charset=\"utf-8\"",
        envelope(&format!(
            "<u:{action}Response xmlns:u=\"{SERVICE}\">{inner}</u:{action}Response>"
        )),
    )
}
fn fault(code: u16, description: &str) -> Response {
    Response::new(
        500,
        "text/xml; charset=\"utf-8\"",
        envelope(&format!(
            "<s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring>\
             <detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\">\
             <errorCode>{code}</errorCode><errorDescription>{description}</errorDescription>\
             </UPnPError></detail></s:Fault>"
        )),
    )
}

fn entry_xml(port: u16, protocol: &str, m: &Mapping, with_key: bool) -> String {
    let key = if with_key {
        format!("<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>{protocol}</NewProtocol>")
    } else {
        String::new()
    };
    format!(
        "{key}<NewInternalPort>{}</NewInternalPort><NewInternalClient>{}</NewInternalClient>\
         <NewEnabled>{}</NewEnabled><NewPortMappingDescription>{}</NewPortMappingDescription>\
         <NewLeaseDuration>0</NewLeaseDuration>",
        m.internal_port,
        m.internal_client,
        u8::from(m.enabled),
        m.description
    )
}

fn handle(state: &Mutex<IgdState>, request: &Request) -> Response {
    if request.method != "POST" || request.path != CONTROL_PATH {
        return Response::empty(404);
    }
    let Some(action) = request
        .header("soapaction")
        .and_then(|a| a.trim_matches('"').strip_prefix(&format!("{SERVICE}#")))
        .map(str::to_owned)
    else {
        return fault(401, "Invalid Action");
    };
    let xml = request.text();
    if !xml.contains(&format!("{action} ")) && !xml.contains(&format!("{action}>")) {
        return fault(402, "Invalid Args");
    }
    let mut igd = state.lock().unwrap();
    let key = || -> Option<(u16, String)> {
        let port: u16 = arg(&xml, "NewExternalPort")?.parse().ok()?;
        let protocol = arg(&xml, "NewProtocol")?;
        (port != 0 && matches!(protocol.as_str(), "TCP" | "UDP")).then_some((port, protocol))
    };
    match action.as_str() {
        "GetStatusInfo" => ok(
            &action,
            &format!(
                "<NewConnectionStatus>{}</NewConnectionStatus>\
                 <NewLastConnectionError>ERROR_NONE</NewLastConnectionError>\
                 <NewUptime>{}</NewUptime>",
                if igd.connected {
                    "Connected"
                } else {
                    "Disconnected"
                },
                igd.started.elapsed().as_secs()
            ),
        ),
        "AddPortMapping" => {
            let Some(key) = key() else {
                return fault(402, "Invalid Args");
            };
            let parsed = (|| {
                Some(Mapping {
                    internal_port: arg(&xml, "NewInternalPort")?.parse().ok()?,
                    internal_client: arg(&xml, "NewInternalClient")?.parse().ok()?,
                    enabled: arg(&xml, "NewEnabled")? == "1",
                    description: arg(&xml, "NewPortMappingDescription")?,
                })
            })();
            let Some(mapping) = parsed else {
                return fault(402, "Invalid Args");
            };
            if let Some(existing) = igd.table.get(&key) {
                if existing.internal_client != mapping.internal_client {
                    return fault(718, "ConflictInMappingEntry");
                }
            }
            igd.table.insert(key, mapping);
            ok(&action, "")
        }
        "DeletePortMapping" => {
            let Some(key) = key() else {
                return fault(402, "Invalid Args");
            };
            match igd.table.remove(&key) {
                Some(_) => ok(&action, ""),
                None => fault(714, "NoSuchEntryInArray"),
            }
        }
        "GetSpecificPortMappingEntry" => {
            let Some(key) = key() else {
                return fault(402, "Invalid Args");
            };
            match igd.table.get(&key) {
                Some(m) => ok(&action, &entry_xml(key.0, &key.1, m, false)),
                None => fault(714, "NoSuchEntryInArray"),
            }
        }
        "GetGenericPortMappingEntry" => {
            let Some(index) =
                arg(&xml, "NewPortMappingIndex").and_then(|i| i.parse::<usize>().ok())
            else {
                return fault(402, "Invalid Args");
            };
            match igd.table.iter().nth(index) {
                Some(((port, protocol), m)) => ok(&action, &entry_xml(*port, protocol, m, true)),
                None => fault(713, "SpecifiedArrayIndexInvalid"),
            }
        }
        _ => fault(401, "Invalid Action"),
    }
}
