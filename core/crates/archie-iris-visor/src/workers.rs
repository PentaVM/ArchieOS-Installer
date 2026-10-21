//! The threads behind the window: the camera feed, IRIS's live answers, and reading text
//! for Wanda. Each writes what it learned to the shared state and never waits on the UI.

use crate::state::{Frame, OcrCall, Seen, State, OCR_LOG};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Largest frame accepted from the camera bridge.
const MAX_FRAME: usize = 8 * 1024 * 1024;
/// Phrases whose boxes hold text worth reading.
const TEXT_PHRASES: [&str; 6] = ["text", "sign", "label", "screen", "book", "poster"];
/// Crops one OCR call reads (each costs IRIS a pass of both encoders).
const OCR_BOXES: usize = 4;

pub struct Settings {
    pub camera_listen: String,
    pub iris: String,
    pub phrases: String,
    pub threshold: f32,
    pub wanda: String,
    pub wanda_token: Option<String>,
    pub domain: String,
    pub ocr_every: Duration,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("an HTTP client")
}

/// Frames from the camera bridge: `u32` big-endian length, then a JPEG. One bridge at a time;
/// a new connection replaces the old.
pub fn camera(state: State, listen: String) {
    let listener = match TcpListener::bind(&listen) {
        Ok(listener) => listener,
        Err(error) => {
            if let Ok(mut s) = state.lock() {
                s.camera_from = format!("no se pudo escuchar en {listen}: {error}");
            }
            return;
        }
    };
    let mut next_id = 1u64;
    for stream in listener.incoming().flatten() {
        let from = stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_default();
        if let Ok(mut s) = state.lock() {
            s.camera_connected = true;
            s.camera_from = from;
        }
        let _ = receive(&state, stream, &mut next_id);
        if let Ok(mut s) = state.lock() {
            s.camera_connected = false;
        }
    }
}

fn receive(state: &State, mut stream: TcpStream, next_id: &mut u64) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut header = [0u8; 4];
    loop {
        stream.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 || length > MAX_FRAME {
            return Err(std::io::Error::other("frame size"));
        }
        let mut jpeg = vec![0u8; length];
        stream.read_exact(&mut jpeg)?;
        let mut decoder = jpeg_decoder::Decoder::new(jpeg.as_slice());
        let Ok(pixels) = decoder.decode() else {
            continue;
        };
        let Some(info) = decoder.info() else {
            continue;
        };
        let (width, height) = (usize::from(info.width), usize::from(info.height));
        let rgb = match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => pixels,
            jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&v| [v, v, v]).collect(),
            _ => continue,
        };
        if rgb.len() != width * height * 3 {
            continue;
        }
        let id = *next_id;
        *next_id += 1;
        if let Ok(mut s) = state.lock() {
            s.camera.tick();
            s.frame = Some(Frame {
                id,
                width,
                height,
                rgb: Arc::new(rgb),
                jpeg: Arc::new(jpeg),
            });
        }
    }
}

fn seen_of(value: &Value) -> Vec<Seen> {
    value["detections"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|d| {
                    let b = d["box"].as_array()?;
                    let at = |i: usize| b.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    Some(Seen {
                        phrase: d["phrase"].as_str()?.to_owned(),
                        score: d["score"].as_f64().unwrap_or(0.0) as f32,
                        bbox: [at(0), at(1), at(2), at(3)],
                        track: d["track"].as_u64().unwrap_or(0),
                        source: d["source"].as_str().unwrap_or("inferred").to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every new frame to IRIS's live endpoint: an answer for each frame (damage and tracking),
/// a model read when the scene asks for one.
pub fn iris(state: State, settings: Arc<Settings>) {
    let client = client();
    let url = format!(
        "{}/live?focus=elgato&mode=focused_open&phrases={}&threshold={}&priority=active_focus",
        settings.iris.trim_end_matches('/'),
        settings.phrases,
        settings.threshold
    );
    let mut last_id = 0u64;
    let mut last_age: Option<u64> = None;
    loop {
        let frame = state.lock().ok().and_then(|s| {
            s.frame
                .as_ref()
                .filter(|f| f.id != last_id)
                .map(|f| (f.id, f.jpeg.clone()))
        });
        let Some((id, jpeg)) = frame else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        last_id = id;
        let started = Instant::now();
        let reply = client
            .post(&url)
            .header("content-type", "image/jpeg")
            .body(jpeg.as_ref().clone())
            .send()
            .and_then(|r| r.json::<Value>());
        let elapsed = started.elapsed().as_secs_f32() * 1000.0;
        let Ok(mut s) = state.lock() else {
            return;
        };
        match reply {
            Ok(value) if value.get("error").is_none() => {
                let age = value["inference_age_ms"].as_u64();
                // A smaller age than last time: the model read a newer frame.
                if let (Some(now), Some(before)) = (age, last_age) {
                    if now < before {
                        s.iris.reads.tick();
                    }
                }
                last_age = age;
                s.iris.online = true;
                s.iris.error = None;
                s.iris.frame_id = id;
                s.iris.seen = seen_of(&value);
                s.iris.decision = value["decision"]["do"].as_str().unwrap_or("").to_owned();
                s.iris.open_sweep = value["decision"]["open"].as_bool().unwrap_or(false);
                s.iris.damage = value["damage"].as_f64().unwrap_or(0.0) as f32;
                s.iris.inference_age_ms = age;
                s.iris.answer_ms = elapsed;
                s.iris.answers.tick();
            }
            Ok(value) => {
                s.iris.error = Some(value["error"].as_str().unwrap_or("error").to_owned());
            }
            Err(error) => {
                s.iris.online = false;
                s.iris.error = Some(short(&error.to_string()));
                drop(s);
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

fn short(text: &str) -> String {
    text.chars().take(160).collect()
}

/// Every `ocr_every`: the boxes IRIS sees text in, read by IRIS's reading head, and the lines
/// that are new or changed handed to Wanda's state update (`scene.text`).
pub fn ocr(state: State, settings: Arc<Settings>) {
    let client = client();
    let mut known: HashMap<u64, String> = HashMap::new();
    loop {
        std::thread::sleep(settings.ocr_every);
        let job = state.lock().ok().and_then(|s| {
            if !s.ocr_enabled {
                return None;
            }
            let frame = s.frame.as_ref()?;
            let mut boxes: Vec<Seen> = s
                .iris
                .seen
                .iter()
                .filter(|d| TEXT_PHRASES.iter().any(|p| d.phrase.contains(p)))
                .cloned()
                .collect();
            boxes.sort_by(|a, b| b.score.total_cmp(&a.score));
            boxes.truncate(OCR_BOXES);
            (!boxes.is_empty()).then(|| (frame.jpeg.clone(), boxes))
        });
        let Some((jpeg, boxes)) = job else {
            continue;
        };
        let spec: Vec<String> = boxes
            .iter()
            .map(|d| {
                format!(
                    "{:.0},{:.0},{:.0},{:.0}",
                    d.bbox[0], d.bbox[1], d.bbox[2], d.bbox[3]
                )
            })
            .collect();
        let started = Instant::now();
        let read = client
            .post(format!(
                "{}/read?boxes={}",
                settings.iris.trim_end_matches('/'),
                spec.join(";")
            ))
            .header("content-type", "image/jpeg")
            .body(jpeg.as_ref().clone())
            .send()
            .and_then(|r| r.json::<Value>());
        let read_ms = started.elapsed().as_secs_f32() * 1000.0;
        let texts: Vec<String> = match &read {
            Ok(value) => value["reads"]
                .as_array()
                .map(|r| {
                    r.iter()
                        .map(|x| x["text"].as_str().unwrap_or("").trim().to_owned())
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let mut lines = Vec::new();
        for (seen, text) in boxes.iter().zip(&texts) {
            if text.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
                continue;
            }
            if known.get(&seen.track) == Some(text) {
                continue;
            }
            known.insert(seen.track, text.clone());
            lines.push(
                json!({"track": seen.track, "text": text, "origin": "iris_read",
                              "confidence": seen.score, "observed_at_ms": now_ms()}),
            );
        }
        let wanda = if let Err(error) = &read {
            Err(format!("IRIS /read: {}", short(&error.to_string())))
        } else if lines.is_empty() {
            Ok("nada nuevo: no se llama a Wanda".to_owned())
        } else {
            send_to_wanda(&client, &settings, &state, &lines)
        };
        if let Ok(mut s) = state.lock() {
            s.ocr.push_front(OcrCall {
                at: Instant::now(),
                boxes: boxes.len(),
                read_ms,
                lines: texts,
                sent: lines.len(),
                wanda,
            });
            s.ocr.truncate(OCR_LOG);
        }
    }
}

fn send_to_wanda(
    client: &reqwest::blocking::Client,
    settings: &Settings,
    state: &State,
    lines: &[Value],
) -> Result<String, String> {
    let Some(token) = &settings.wanda_token else {
        return Err("sin token de Wanda (--wanda-token-file)".into());
    };
    let body = json!({"domain": settings.domain,
                      "delta": {"focus": "elgato", "lines": lines}});
    let started = Instant::now();
    let reply = client
        .post(format!(
            "{}/v1/scene/text",
            settings.wanda.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .json(&body)
        .send()
        .map_err(|e| short(&e.to_string()))?;
    let status = reply.status();
    let value: Value = reply.json().unwrap_or(Value::Null);
    let ms = started.elapsed().as_millis();
    let mut s = state.lock().map_err(|_| "estado".to_string())?;
    s.wanda.calls += 1;
    if !status.is_success() {
        let error = value["error"].as_str().unwrap_or("error").to_owned();
        s.wanda.error = Some(error.clone());
        return Err(format!("Wanda {status}: {error}"));
    }
    s.wanda.online = true;
    s.wanda.error = None;
    let read = &value["read"];
    if read.is_null() {
        return Ok(format!("{ms} ms · Wanda sin experto LNCM cargado"));
    }
    let n = |k: &str| read[k].as_u64().unwrap_or(0);
    s.wanda.applied += n("applied");
    s.wanda.unchanged += n("unchanged");
    s.wanda.rejected += n("rejected");
    Ok(format!(
        "{ms} ms · {} turnos · {} cambios aplicados · {} sin cambio · {} rechazados",
        n("turns"),
        n("applied"),
        n("unchanged"),
        n("rejected")
    ))
}

/// Whether Wanda answers, every ten seconds (owner-authenticated, read only).
pub fn wanda_health(state: State, settings: Arc<Settings>) {
    let client = client();
    loop {
        let online = settings.wanda_token.as_ref().is_some_and(|token| {
            client
                .get(format!(
                    "{}/v1/domains",
                    settings.wanda.trim_end_matches('/')
                ))
                .bearer_auth(token)
                .send()
                .is_ok_and(|r| r.status().is_success())
        });
        if let Ok(mut s) = state.lock() {
            s.wanda.online = online;
            s.wanda.domain = settings.domain.clone();
        }
        std::thread::sleep(Duration::from_secs(10));
    }
}
