//! An ONVIF device and media service over SOAP 1.2: GetDeviceInformation,
//! GetProfiles and GetStreamUri. When a user is configured, every request
//! must carry a WS-Security UsernameToken whose PasswordDigest verifies
//! (base64(SHA-1(nonce ‖ created ‖ password))) and whose nonce was never
//! seen before; otherwise the camera answers `ter:NotAuthorized`.
use super::http::{HttpServer, Request, Response};
use archie_sentry::adapters::onvif::{base64_encode, sha1};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

pub const PATH: &str = "/onvif/device_service";

#[derive(Debug)]
pub struct CameraState {
    pub user: Option<(String, String)>,
    pub profiles: Vec<(String, String)>,
    pub nonces: BTreeSet<String>,
    pub refused: usize,
}

pub struct Camera {
    pub url: String,
    pub state: Arc<Mutex<CameraState>>,
}
impl Camera {
    pub async fn start(user: Option<(&str, &str)>, profiles: &[(&str, &str)]) -> Self {
        let state = Arc::new(Mutex::new(CameraState {
            user: user.map(|(u, p)| (u.to_owned(), p.to_owned())),
            profiles: profiles
                .iter()
                .map(|(t, u)| (t.to_string(), u.to_string()))
                .collect(),
            nonces: BTreeSet::new(),
            refused: 0,
        }));
        let s = state.clone();
        let server = HttpServer::start(Arc::new(move |r: &Request| handle(&s, r))).await;
        Self {
            url: format!("{}{PATH}", server.base),
            state,
        }
    }
}

fn text(xml: &str, local: &str) -> Option<String> {
    let at = [format!(":{local}>"), format!(":{local} ")]
        .iter()
        .filter_map(|needle| xml.find(needle.as_str()))
        .min()?;
    let open_end = xml[at..].find('>')? + at + 1;
    let close = xml[open_end..].find('<')? + open_end;
    Some(xml[open_end..close].to_owned())
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut count = 0;
    let mut out = Vec::new();
    for c in input.bytes().filter(|c| *c != b'=') {
        let v = ALPHABET.iter().position(|a| *a == c)? as u32;
        bits = (bits << 6) | v;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}

fn soap(body: &str) -> Response {
    Response::new(
        200,
        "application/soap+xml; charset=utf-8",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><env:Envelope \
             xmlns:env=\"http://www.w3.org/2003/05/soap-envelope\" \
             xmlns:tds=\"http://www.onvif.org/ver10/device/wsdl\" \
             xmlns:trt=\"http://www.onvif.org/ver10/media/wsdl\" \
             xmlns:tt=\"http://www.onvif.org/ver10/schema\"><env:Body>{body}</env:Body></env:Envelope>"
        ),
    )
}
fn not_authorized() -> Response {
    let mut r = soap(
        "<env:Fault><env:Code><env:Value>env:Sender</env:Value><env:Subcode>\
         <env:Value>ter:NotAuthorized</env:Value></env:Subcode></env:Code>\
         <env:Reason><env:Text xml:lang=\"en\">Sender not Authorized</env:Text></env:Reason></env:Fault>",
    );
    r.status = 400;
    r
}

fn handle(state: &Mutex<CameraState>, request: &Request) -> Response {
    if request.method != "POST" || request.path != PATH {
        return Response::empty(404);
    }
    let xml = request.text();
    let mut camera = state.lock().unwrap();
    if let Some((user, password)) = camera.user.clone() {
        let verified = (|| {
            let name = text(&xml, "Username")?;
            let digest = text(&xml, "Password")?;
            let nonce = text(&xml, "Nonce")?;
            let created = text(&xml, "Created")?;
            let raw = base64_decode(&nonce)?;
            let mut input = raw;
            input.extend_from_slice(created.as_bytes());
            input.extend_from_slice(password.as_bytes());
            (name == user && digest == base64_encode(&sha1(&input))).then_some(nonce)
        })();
        match verified {
            Some(nonce) if camera.nonces.insert(nonce.clone()) => {}
            _ => {
                camera.refused += 1;
                return not_authorized();
            }
        }
    }
    if xml.contains("GetDeviceInformation") {
        soap(
            "<tds:GetDeviceInformationResponse><tds:Manufacturer>Loopback</tds:Manufacturer>\
             <tds:Model>Cam-1</tds:Model><tds:FirmwareVersion>1.0.0</tds:FirmwareVersion>\
             <tds:SerialNumber>LOOP0001</tds:SerialNumber><tds:HardwareId>1</tds:HardwareId>\
             </tds:GetDeviceInformationResponse>",
        )
    } else if xml.contains("GetProfiles") {
        let profiles: String = camera
            .profiles
            .iter()
            .map(|(token, _)| format!("<trt:Profiles token=\"{token}\" fixed=\"true\"><tt:Name>{token}</tt:Name></trt:Profiles>"))
            .collect();
        soap(&format!(
            "<trt:GetProfilesResponse>{profiles}</trt:GetProfilesResponse>"
        ))
    } else if xml.contains("GetStreamUri") {
        let token = text(&xml, "ProfileToken").unwrap_or_default();
        match camera.profiles.iter().find(|(t, _)| *t == token) {
            Some((_, uri)) => soap(&format!(
                "<trt:GetStreamUriResponse><trt:MediaUri><tt:Uri>{uri}</tt:Uri>\
                 <tt:InvalidAfterConnect>false</tt:InvalidAfterConnect></trt:MediaUri>\
                 </trt:GetStreamUriResponse>"
            )),
            None => {
                let mut r = soap(
                    "<env:Fault><env:Code><env:Value>env:Sender</env:Value></env:Code></env:Fault>",
                );
                r.status = 400;
                r
            }
        }
    } else {
        let mut r =
            soap("<env:Fault><env:Code><env:Value>env:Receiver</env:Value></env:Code></env:Fault>");
        r.status = 400;
        r
    }
}
