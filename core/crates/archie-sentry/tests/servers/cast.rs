//! A CAST v2 receiver over TLS: CONNECT/CLOSE, PING/PONG, GET_STATUS,
//! LAUNCH, STOP and SET_VOLUME against a receiver state it owns.
//!
//! Strictness the adapter must survive, as on a device: receiver requests
//! before CONNECT are ignored; before answering the first request on a
//! connection the receiver sends PING and does not answer until it gets PONG;
//! every state change is broadcast (requestId 0) before the direct reply;
//! volume is quantized to the receiver's step interval.
use archie_sentry::adapters::chromecast::wire::{self, CastMessage};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    ServerConfig,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub const CERT: &[u8] = include_bytes!("../fixtures/cast/cert.der");
pub const KEY: &[u8] = include_bytes!("../fixtures/cast/key.der");
pub const OTHER_CERT: &[u8] = include_bytes!("../fixtures/cast/other-cert.der");
pub const OTHER_KEY: &[u8] = include_bytes!("../fixtures/cast/other-key.der");
pub const BACKDROP: &str = "E8C28D3C";

#[derive(Debug, Clone)]
pub struct ReceiverState {
    /// Applications the receiver can launch.
    pub installed: BTreeSet<String>,
    /// Running non-idle application and its session.
    pub running: Option<(String, String)>,
    pub level: f64,
    pub muted: bool,
    pub step: f64,
    pub standby: bool,
    sessions: u32,
}
impl ReceiverState {
    fn status(&self) -> Value {
        let applications = match &self.running {
            Some((app, session)) => json!([{
                "appId": app, "displayName": app, "isIdleScreen": false,
                "sessionId": session, "transportId": session, "statusText": "",
                "namespaces": [{"name": "urn:x-cast:com.google.cast.media"}]
            }]),
            None => json!([{
                "appId": BACKDROP, "displayName": "Backdrop", "isIdleScreen": true,
                "sessionId": "idle-0", "transportId": "idle-0", "statusText": ""
            }]),
        };
        json!({
            "applications": applications,
            "isActiveInput": true,
            "isStandBy": self.standby,
            "volume": {"controlType": "attenuation", "level": self.level,
                       "muted": self.muted, "stepInterval": self.step}
        })
    }
}

/// What crossed the wire, in order: `namespace-suffix:TYPE`.
pub type Log = Arc<Mutex<Vec<String>>>;

pub struct CastReceiver {
    pub address: SocketAddr,
    pub state: Arc<Mutex<ReceiverState>>,
    pub log: Log,
}
impl CastReceiver {
    pub async fn start(installed: &[&str]) -> Self {
        Self::start_with(CERT, KEY, installed).await
    }
    pub async fn start_with(cert: &[u8], key: &[u8], installed: &[&str]) -> Self {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())),
            )
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(ReceiverState {
            installed: installed.iter().map(|s| s.to_string()).collect(),
            running: None,
            level: 0.5,
            muted: false,
            step: 0.01,
            standby: false,
            sessions: 0,
        }));
        let log: Log = Arc::default();
        let (s, l) = (state.clone(), log.clone());
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                let (s, l) = (s.clone(), l.clone());
                tokio::spawn(async move {
                    match acceptor.accept(tcp).await {
                        Ok(tls) => serve(tls, s, l).await,
                        Err(_) => l.lock().unwrap().push("tls:REFUSED".into()),
                    }
                });
            }
        });
        Self {
            address,
            state,
            log,
        }
    }
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

fn reply(namespace: &str, destination: &str, body: Value) -> CastMessage {
    CastMessage::json(wire::RECEIVER, destination, namespace, body.to_string())
}

async fn serve<S>(mut stream: S, state: Arc<Mutex<ReceiverState>>, log: Log)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut connected = false;
    let mut heartbeat_ok = false;
    loop {
        let Ok(message) = wire::read_frame(&mut stream).await else {
            return;
        };
        let sender = message.source_id.clone();
        let Some(body) = message
            .payload_utf8
            .as_deref()
            .and_then(|t| serde_json::from_str::<Value>(t).ok())
        else {
            return;
        };
        let kind = body["type"].as_str().unwrap_or("").to_owned();
        let suffix = message
            .namespace
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_owned();
        log.lock().unwrap().push(format!("{suffix}:{kind}"));
        match (message.namespace.as_str(), kind.as_str()) {
            (wire::NS_CONNECTION, "CONNECT") => connected = true,
            (wire::NS_CONNECTION, "CLOSE") => return,
            (wire::NS_HEARTBEAT, "PING") => {
                let pong = reply(wire::NS_HEARTBEAT, &sender, json!({"type": "PONG"}));
                if wire::write_frame(&mut stream, &pong).await.is_err() {
                    return;
                }
            }
            (wire::NS_HEARTBEAT, "PONG") => heartbeat_ok = true,
            (wire::NS_RECEIVER, _) if !connected => {}
            (wire::NS_RECEIVER, _) => {
                if !heartbeat_ok {
                    // Hold the request until the sender proves it is alive.
                    let ping = reply(wire::NS_HEARTBEAT, &sender, json!({"type": "PING"}));
                    if wire::write_frame(&mut stream, &ping).await.is_err() {
                        return;
                    }
                    loop {
                        let Ok(next) = wire::read_frame(&mut stream).await else {
                            return;
                        };
                        let text = next.payload_utf8.unwrap_or_default();
                        if next.namespace == wire::NS_HEARTBEAT && text.contains("\"PONG\"") {
                            log.lock().unwrap().push("heartbeat:PONG".into());
                            heartbeat_ok = true;
                            break;
                        }
                    }
                }
                let request_id = body["requestId"].clone();
                let (answer, changed) = handle(&state, &body);
                let mut out = Vec::new();
                if changed {
                    let status = state.lock().unwrap().status();
                    out.push(json!({"type": "RECEIVER_STATUS", "requestId": 0, "status": status}));
                }
                let mut answer = answer;
                answer["requestId"] = request_id;
                out.push(answer);
                for body in out {
                    let frame = reply(wire::NS_RECEIVER, &sender, body);
                    if wire::write_frame(&mut stream, &frame).await.is_err() {
                        return;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Apply one receiver request to the state. Returns the direct answer and
/// whether the state changed.
fn handle(state: &Mutex<ReceiverState>, body: &Value) -> (Value, bool) {
    let mut s = state.lock().unwrap();
    let status = |s: &ReceiverState| json!({"type": "RECEIVER_STATUS", "status": s.status()});
    match body["type"].as_str().unwrap_or("") {
        "GET_STATUS" => (status(&s), false),
        "LAUNCH" => {
            let app = body["appId"].as_str().unwrap_or("");
            if !s.installed.contains(app) {
                return (
                    json!({"type": "LAUNCH_ERROR", "reason": "NOT_FOUND"}),
                    false,
                );
            }
            s.sessions += 1;
            let session = format!("session-{}", s.sessions);
            s.running = Some((app.to_owned(), session));
            s.standby = false;
            (status(&s), true)
        }
        "STOP" => {
            let wanted = body["sessionId"].as_str().unwrap_or("");
            match &s.running {
                Some((_, session)) if session == wanted => {
                    s.running = None;
                    (status(&s), true)
                }
                _ => (
                    json!({"type": "INVALID_REQUEST", "reason": "INVALID_SESSION_ID"}),
                    false,
                ),
            }
        }
        "SET_VOLUME" => {
            let volume = &body["volume"];
            if let Some(level) = volume["level"].as_f64() {
                let level = level.clamp(0.0, 1.0);
                s.level = ((level / s.step).round() * s.step * 1e6).round() / 1e6;
            }
            if let Some(muted) = volume["muted"].as_bool() {
                s.muted = muted;
            }
            (status(&s), true)
        }
        _ => (
            json!({"type": "INVALID_REQUEST", "reason": "INVALID_COMMAND"}),
            false,
        ),
    }
}
