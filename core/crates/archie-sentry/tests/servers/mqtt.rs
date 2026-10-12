//! A minimal MQTT 3.1.1 broker and the devices that live behind it.
//!
//! The broker is a broker: CONNECT with optional username/password
//! (CONNACK 0 or 5), SUBSCRIBE with `+`/`#` filters (SUBACK, then the
//! retained message of every matching topic), PUBLISH routed to every
//! matching subscription, retained messages kept and replaced, PINGREQ and
//! DISCONNECT. QoS 0 only: a QoS 1/2 subscription is granted at 0.
//!
//! A [`Device`] is a separate MQTT client over TCP: it subscribes to its
//! command topic and, on each command, changes its own state and publishes
//! it retained on its state topic. The truth lives in the device; the broker
//! only keeps the last retained report — as with a real broker and plug.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

type Outbox = mpsc::UnboundedSender<Vec<u8>>;

#[derive(Default)]
struct BrokerState {
    retained: BTreeMap<String, Vec<u8>>,
    /// (client id, filter, outbox)
    subscriptions: Vec<(u64, String, Outbox)>,
    next_client: u64,
}

pub struct Broker {
    pub address: String,
    state: Arc<Mutex<BrokerState>>,
    /// `client:<CONNECT|SUBSCRIBE topic|PUBLISH topic>` in order.
    pub log: Arc<Mutex<Vec<String>>>,
}

pub fn filter_matches(filter: &str, topic: &str) -> bool {
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

fn remaining_length(mut n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            return out;
        }
    }
}
fn packet(header: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![header];
    out.extend(remaining_length(body.len()));
    out.extend_from_slice(body);
    out
}
fn string(s: &str) -> Vec<u8> {
    let mut out = (s.len() as u16).to_be_bytes().to_vec();
    out.extend_from_slice(s.as_bytes());
    out
}
pub fn publish_packet(topic: &str, payload: &[u8], retain: bool) -> Vec<u8> {
    let mut body = string(topic);
    body.extend_from_slice(payload);
    packet(0x30 | u8::from(retain), &body)
}

async fn read_packet(stream: &mut (impl AsyncReadExt + Unpin)) -> Option<(u8, Vec<u8>)> {
    let header = stream.read_u8().await.ok()?;
    let mut length = 0usize;
    for shift in 0..4 {
        let byte = stream.read_u8().await.ok()?;
        length |= usize::from(byte & 0x7f) << (7 * shift);
        if byte & 0x80 == 0 {
            let mut body = vec![0u8; length];
            stream.read_exact(&mut body).await.ok()?;
            return Some((header, body));
        }
    }
    None
}
fn take_string(body: &[u8], at: &mut usize) -> Option<String> {
    let len = usize::from(u16::from_be_bytes([*body.get(*at)?, *body.get(*at + 1)?]));
    let s = std::str::from_utf8(body.get(*at + 2..*at + 2 + len)?).ok()?;
    *at += 2 + len;
    Some(s.to_owned())
}

impl Broker {
    /// `credentials`: when set, CONNECT must carry exactly these.
    pub async fn start(credentials: Option<(&str, &str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let state: Arc<Mutex<BrokerState>> = Arc::default();
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        let credentials = credentials.map(|(u, p)| (u.to_owned(), p.to_owned()));
        let (s, l) = (state.clone(), log.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let (s, l, c) = (s.clone(), l.clone(), credentials.clone());
                tokio::spawn(async move { session(stream, s, l, c).await });
            }
        });
        Self {
            address,
            state,
            log,
        }
    }
    pub fn retained(&self, topic: &str) -> Option<Vec<u8>> {
        self.state.lock().unwrap().retained.get(topic).cloned()
    }
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

async fn session(
    stream: TcpStream,
    state: Arc<Mutex<BrokerState>>,
    log: Arc<Mutex<Vec<String>>>,
    credentials: Option<(String, String)>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let (outbox, mut queue) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(bytes) = queue.recv().await {
            if writer.write_all(&bytes).await.is_err() {
                return;
            }
        }
    });
    // CONNECT first, or nothing.
    let Some((0x10, body)) = read_packet(&mut reader).await else {
        return;
    };
    let mut at = 0;
    let (Some(protocol), Some(&level), Some(&flags)) =
        (take_string(&body, &mut at), body.get(at), body.get(at + 1))
    else {
        return;
    };
    at += 4; // level, flags, keep-alive
    let Some(client) = take_string(&body, &mut at) else {
        return;
    };
    let username = if flags & 0x80 != 0 {
        take_string(&body, &mut at)
    } else {
        None
    };
    let password = if flags & 0x40 != 0 {
        take_string(&body, &mut at)
    } else {
        None
    };
    if protocol != "MQTT" || level != 4 {
        let _ = outbox.send(vec![0x20, 2, 0, 1]);
        return;
    }
    let authorized = match &credentials {
        None => true,
        Some((u, p)) => username.as_deref() == Some(u) && password.as_deref() == Some(p),
    };
    log.lock().unwrap().push(format!("{client}:CONNECT"));
    if !authorized {
        let _ = outbox.send(vec![0x20, 2, 0, 5]);
        return;
    }
    let id = {
        let mut s = state.lock().unwrap();
        s.next_client += 1;
        s.next_client
    };
    let _ = outbox.send(vec![0x20, 2, 0, 0]);
    while let Some((header, body)) = read_packet(&mut reader).await {
        match header >> 4 {
            3 => {
                if header & 0x06 != 0 {
                    break; // QoS > 0 was never granted
                }
                let mut at = 0;
                let Some(topic) = take_string(&body, &mut at) else {
                    break;
                };
                let payload = body[at..].to_vec();
                log.lock()
                    .unwrap()
                    .push(format!("{client}:PUBLISH {topic}"));
                let mut s = state.lock().unwrap();
                if header & 0x01 != 0 {
                    if payload.is_empty() {
                        s.retained.remove(&topic);
                    } else {
                        s.retained.insert(topic.clone(), payload.clone());
                    }
                }
                let frame = publish_packet(&topic, &payload, false);
                s.subscriptions.retain(|(_, _, o)| !o.is_closed());
                for (_, filter, o) in &s.subscriptions {
                    if filter_matches(filter, &topic) {
                        let _ = o.send(frame.clone());
                    }
                }
            }
            8 => {
                if header & 0x0f != 0x02 || body.len() < 2 {
                    break;
                }
                let packet_id = [body[0], body[1]];
                let mut at = 2;
                let mut granted = Vec::new();
                let mut deliveries = Vec::new();
                while at < body.len() {
                    let Some(filter) = take_string(&body, &mut at) else {
                        return;
                    };
                    at += 1; // requested QoS; granted 0
                    log.lock()
                        .unwrap()
                        .push(format!("{client}:SUBSCRIBE {filter}"));
                    let mut s = state.lock().unwrap();
                    for (topic, payload) in &s.retained {
                        if filter_matches(&filter, topic) {
                            deliveries.push(publish_packet(topic, payload, true));
                        }
                    }
                    s.subscriptions.push((id, filter, outbox.clone()));
                    granted.push(0u8);
                }
                let mut ack = packet_id.to_vec();
                ack.extend(granted);
                let _ = outbox.send(packet(0x90, &ack));
                for d in deliveries {
                    let _ = outbox.send(d);
                }
            }
            12 => {
                let _ = outbox.send(vec![0xd0, 0]);
            }
            14 => break,
            _ => break,
        }
    }
    state
        .lock()
        .unwrap()
        .subscriptions
        .retain(|(c, _, _)| *c != id);
}

/// What a device does with a command payload; returns its new state payload,
/// or `None` to ignore a payload it does not understand.
pub type Behaviour = Box<dyn Fn(&[u8], &[u8]) -> Option<Vec<u8>> + Send>;

/// A device on the broker: its own MQTT client, its own state.
pub struct Device {
    pub state: Arc<Mutex<Vec<u8>>>,
}
impl Device {
    /// Connect, publish the initial state retained, then follow the command
    /// topic for as long as the test runs.
    pub async fn attach(
        broker: &str,
        name: &str,
        command_topic: &str,
        state_topic: &str,
        initial: &[u8],
        credentials: Option<(&str, &str)>,
        behaviour: Behaviour,
    ) -> Self {
        let mut stream = TcpStream::connect(broker).await.unwrap();
        let mut body = string("MQTT");
        body.push(4);
        body.push(0x02 | if credentials.is_some() { 0xc0 } else { 0 });
        body.extend_from_slice(&60u16.to_be_bytes());
        body.extend(string(name));
        if let Some((u, p)) = credentials {
            body.extend(string(u));
            body.extend(string(p));
        }
        stream.write_all(&packet(0x10, &body)).await.unwrap();
        let (header, ack) = read_packet(&mut stream).await.unwrap();
        assert_eq!(
            (header, ack),
            (0x20, vec![0, 0]),
            "device {name} refused by broker"
        );
        let mut sub = 1u16.to_be_bytes().to_vec();
        sub.extend(string(command_topic));
        sub.push(0);
        stream.write_all(&packet(0x82, &sub)).await.unwrap();
        let (header, _) = read_packet(&mut stream).await.unwrap();
        assert_eq!(header, 0x90);
        stream
            .write_all(&publish_packet(state_topic, initial, true))
            .await
            .unwrap();
        let state = Arc::new(Mutex::new(initial.to_vec()));
        let (s, state_topic) = (state.clone(), state_topic.to_owned());
        tokio::spawn(async move {
            while let Some((header, body)) = read_packet(&mut stream).await {
                if header >> 4 != 3 {
                    continue;
                }
                let mut at = 0;
                let Some(_topic) = take_string(&body, &mut at) else {
                    return;
                };
                let current = s.lock().unwrap().clone();
                if let Some(next) = behaviour(&current, &body[at..]) {
                    *s.lock().unwrap() = next.clone();
                    if stream
                        .write_all(&publish_packet(&state_topic, &next, true))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
        Self { state }
    }
}

/// A plug: `ON`/`OFF` in, `ON`/`OFF` out.
pub fn plug() -> Behaviour {
    Box::new(|_, payload| match payload {
        b"ON" | b"OFF" => Some(payload.to_vec()),
        _ => None,
    })
}
/// A dimmer: a percent in, the percent it settled on out (clamped to 0..=100).
pub fn dimmer() -> Behaviour {
    Box::new(|_, payload| {
        let value: u32 = std::str::from_utf8(payload).ok()?.trim().parse().ok()?;
        Some(value.min(100).to_string().into_bytes())
    })
}
/// A thermostat that only takes half-degree steps: the setpoint it reports
/// is the request rounded to 0.5 °C.
pub fn thermostat() -> Behaviour {
    Box::new(|_, payload| {
        let value: f64 = std::str::from_utf8(payload).ok()?.trim().parse().ok()?;
        if !(5.0..=35.0).contains(&value) {
            return None;
        }
        Some(format!("{:.1}", (value * 2.0).round() / 2.0).into_bytes())
    })
}
