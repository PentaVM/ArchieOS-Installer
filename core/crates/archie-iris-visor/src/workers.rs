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
    /// A local vision-language model reading the text boxes (llama-server with its projector,
    /// OpenAI-compatible). None: IRIS's own reading head.
    pub ocr_url: Option<String>,
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

/// Every `ocr_every`: the boxes IRIS sees text in, read by the configured reader (a local
/// vision-language model given `--ocr-url`, or IRIS's own reading head), and the lines that
/// are new or changed handed to Wanda's state update (`scene.text`).
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
            (!boxes.is_empty()).then(|| {
                (
                    frame.jpeg.clone(),
                    frame.rgb.clone(),
                    frame.width,
                    frame.height,
                    boxes,
                )
            })
        });
        let Some((jpeg, rgb, width, height, boxes)) = job else {
            continue;
        };
        let started = Instant::now();
        let (read, origin, reader) = match &settings.ocr_url {
            Some(url) => (
                read_with_model(&client, url, &rgb, width, height, &boxes),
                "external_ocr",
                "Qwen3-VL",
            ),
            None => (
                read_with_iris(&client, &settings.iris, &jpeg, &boxes),
                "iris_read",
                "IRIS read_decoder",
            ),
        };
        let read_ms = started.elapsed().as_secs_f32() * 1000.0;
        let texts = read.clone().unwrap_or_default();
        let mut lines = Vec::new();
        for (seen, text) in boxes.iter().zip(&texts) {
            if text.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
                continue;
            }
            if known.get(&seen.track) == Some(text) {
                continue;
            }
            known.insert(seen.track, text.clone());
            lines.push(json!({"track": seen.track, "text": text, "origin": origin,
                              "confidence": seen.score, "observed_at_ms": now_ms()}));
        }
        let wanda = if let Err(error) = &read {
            Err(format!("{reader}: {error}"))
        } else if lines.is_empty() {
            Ok("nada nuevo: no se llama a Wanda".to_owned())
        } else {
            send_to_wanda(&client, &settings, &state, &lines)
        };
        if let Ok(mut s) = state.lock() {
            s.ocr.push_front(OcrCall {
                at: Instant::now(),
                reader,
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

/// IRIS's reading head (`POST /read`), all boxes in one call.
fn read_with_iris(
    client: &reqwest::blocking::Client,
    iris: &str,
    jpeg: &[u8],
    boxes: &[Seen],
) -> Result<Vec<String>, String> {
    let spec: Vec<String> = boxes
        .iter()
        .map(|d| {
            format!(
                "{:.0},{:.0},{:.0},{:.0}",
                d.bbox[0], d.bbox[1], d.bbox[2], d.bbox[3]
            )
        })
        .collect();
    let value: Value = client
        .post(format!(
            "{}/read?boxes={}",
            iris.trim_end_matches('/'),
            spec.join(";")
        ))
        .header("content-type", "image/jpeg")
        .body(jpeg.to_vec())
        .send()
        .and_then(|r| r.json())
        .map_err(|e| short(&e.to_string()))?;
    if let Some(error) = value["error"].as_str() {
        return Err(error.to_owned());
    }
    Ok(value["reads"]
        .as_array()
        .map(|r| {
            r.iter()
                .map(|x| x["text"].as_str().unwrap_or("").trim().to_owned())
                .collect()
        })
        .unwrap_or_default())
}

/// The pixels inside `b`, padded by a tenth of the box (at least 4 px), as a PNG.
fn crop_png(rgb: &[u8], width: usize, height: usize, b: [f32; 4]) -> Option<Vec<u8>> {
    let pad_x = ((b[2] - b[0]) * 0.1).max(4.0);
    let pad_y = ((b[3] - b[1]) * 0.1).max(4.0);
    let x0 = (b[0] - pad_x).max(0.0) as usize;
    let y0 = (b[1] - pad_y).max(0.0) as usize;
    let x1 = ((b[2] + pad_x).max(0.0) as usize).min(width);
    let y1 = ((b[3] + pad_y).max(0.0) as usize).min(height);
    if x1 <= x0 + 1 || y1 <= y0 + 1 {
        return None;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut pixels = Vec::with_capacity(w * h * 3);
    for y in y0..y1 {
        let start = (y * width + x0) * 3;
        pixels.extend_from_slice(&rgb[start..start + w * 3]);
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&pixels).ok()?;
    }
    Some(out)
}

const READ_PROMPT: &str = "Transcribe the text in this image exactly as written, in its \
original language. Reply with the text only, on one line. If there is no legible text, reply \
with nothing.";

/// A local vision-language model behind an OpenAI-compatible server (llama-server with its
/// projector): one crop per request, all at once.
fn read_with_model(
    client: &reqwest::blocking::Client,
    url: &str,
    rgb: &[u8],
    width: usize,
    height: usize,
    boxes: &[Seen],
) -> Result<Vec<String>, String> {
    use base64::Engine;
    let endpoint = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let read_one = |seen: &Seen| -> Result<String, String> {
        let Some(png) = crop_png(rgb, width, height, seen.bbox) else {
            return Ok(String::new());
        };
        let image = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        let body = json!({
            "messages": [{"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": image}},
                {"type": "text", "text": READ_PROMPT}
            ]}],
            "temperature": 0,
            "max_tokens": 48
        });
        let value: Value = client
            .post(&endpoint)
            .json(&body)
            .send()
            .and_then(|r| r.json())
            .map_err(|e| short(&e.to_string()))?;
        let text: String = value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_matches(|c| c == '"' || c == '`')
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        Ok(text)
    };
    let results: Vec<Result<String, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = boxes
            .iter()
            .map(|seen| scope.spawn(move || read_one(seen)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err("lector caído".into())))
            .collect()
    });
    results.into_iter().collect()
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
