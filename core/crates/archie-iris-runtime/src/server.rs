//! The loopback HTTP API of `iris_serve`, served from Rust.
//!
//! `POST /detect?mode=all|phrases&phrases=a,b&threshold=0.35&embed=1` with a JPEG or PNG body,
//! `POST /act?session=&instruction=&skill=&adapter=&record=&app=&sensitivity=`,
//! `POST /act/reset?session=`, `POST /act/recorded?app=&run=`, `POST /read?boxes=x0,y0,x1,y1;…`
//! (the text of each crop, by IRIS's reading head), `GET /health`, and `GET /`.
//! `POST /live?focus=&mode=&phrases=&fps=&threshold=&priority=` answers each frame of a live
//! focus from damage and tracking without waiting for the model, `POST /live/advice` takes
//! Cortex's `ResourceAdvice`, and `GET /live` reports the foci and measured costs.
//! Frames are decoded in memory and dropped unless a caller asks for a run to be recorded on a
//! server started with `--rem-spool`. It binds to 127.0.0.1 only.

use crate::act::Actor;
use crate::detect::Detection;
use crate::detect::Detector;
use crate::live::{spawn_worker, InferRequest, LiveConfig, LiveShared};
use crate::preprocess::Frame;
use crate::rem::{recorded_samples, Recording};
use crate::Error;
use archie_accel::advice::{Envelope, ResourceAdvice};
use archie_accel::plan::{FocusMode, Priority};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

pub const DEMO_PAGE: &str = r#"<!doctype html>
<html lang="es"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>IRIS en vivo</title>
<style>
:root{--bg:#0f1115;--panel:#181b22;--text:#e8eaf0;--muted:#9aa0ad;--accent:#4cc2ff}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:15px/1.4 system-ui,sans-serif}
header{padding:12px 16px;display:flex;gap:12px;align-items:center;flex-wrap:wrap}
header h1{font-size:17px;margin:0}header .muted{color:var(--muted);font-size:13px}
main{display:grid;grid-template-columns:minmax(0,1fr) 320px;gap:12px;padding:0 16px 16px}
@media (max-width:860px){main{grid-template-columns:1fr}}
.stage{position:relative;background:#000;border-radius:10px;overflow:hidden}
video,canvas.overlay{width:100%;display:block}canvas.overlay{position:absolute;inset:0;height:100%}
aside{background:var(--panel);border-radius:10px;padding:12px;display:flex;flex-direction:column;gap:10px}
label{font-size:13px;color:var(--muted)}input[type=text]{width:100%;padding:8px;border-radius:6px;border:1px solid #2b303b;background:#10131a;color:var(--text)}
button{padding:8px 12px;border-radius:6px;border:0;background:var(--accent);color:#04121c;font-weight:600;cursor:pointer}
ul{list-style:none;margin:0;padding:0;font-size:14px}li{padding:4px 0;border-bottom:1px solid #232733}
.stat{font-variant-numeric:tabular-nums;color:var(--muted);font-size:13px}
</style></head><body>
<header><h1>IRIS en vivo</h1><span class="muted" id="model">cargando modelo…</span></header>
<main>
<div class="stage"><video id="video" autoplay playsinline muted></video><canvas class="overlay" id="overlay"></canvas></div>
<aside>
<div><label><input type="checkbox" id="everything" checked> Modo abierto: busca ~250 conceptos a la vez</label></div>
<div><label for="phrases">Además (o solo esto, sin modo abierto): cualquier texto, separado por comas</label>
<input type="text" id="phrases" value="" placeholder="p. ej. taza roja, logo de nvidia, mi llavero"></div>
<div><label><input type="checkbox" id="tags" checked> Mostrar etiquetas en las cajas</label></div>
<div><label><input type="checkbox" id="live" checked> Seguimiento en vivo: responde a cada cuadro y lee con el modelo cuando la escena cambia</label></div>
<div><label for="threshold">Umbral <span id="thresholdValue">0.35</span></label>
<input type="range" id="threshold" min="0.1" max="0.9" step="0.05" value="0.35" style="width:100%"></div>
<div style="display:flex;gap:8px"><button id="start">Iniciar cámara</button><button id="stop" style="background:#3a3f4b;color:var(--text)">Pausar</button></div>
<div class="stat" id="stats">—</div>
<div><label>Lo que IRIS ve</label><ul id="seen"></ul></div>
</aside></main>
<script>
const video=document.getElementById('video'),overlay=document.getElementById('overlay'),ctx=overlay.getContext('2d');
const grab=document.createElement('canvas');let running=false,stream=null;
const palette=['#4cc2ff','#ffaa00','#78dc5a','#ff5a78','#be78ff','#ffe650','#5affd2','#ff8c3c'];const colours={};
document.getElementById('threshold').oninput=e=>document.getElementById('thresholdValue').textContent=e.target.value;
fetch('health').then(r=>r.json()).then(h=>{document.getElementById('model').textContent=
 `paso ${h.step} · ${(h.parameters/1e6).toFixed(0)} M parámetros · ${h.device}`;}).catch(()=>{});
document.getElementById('start').onclick=async()=>{
 if(!stream){stream=await navigator.mediaDevices.getUserMedia({video:{width:{ideal:1280},height:{ideal:720}},audio:false});video.srcObject=stream;await video.play();}
 running=true;loop();setTimeout(loop,250);};
document.getElementById('stop').onclick=()=>{running=false;};
async function loop(){
 if(!running)return;
 const w=video.videoWidth,h=video.videoHeight;if(!w){setTimeout(loop,100);return;}
 const scale=Math.min(1,512/w);grab.width=Math.round(w*scale);grab.height=Math.round(h*scale);
 grab.getContext('2d').drawImage(video,0,0,grab.width,grab.height);
 const blob=await new Promise(r=>grab.toBlob(r,'image/jpeg',0.8));
 const phrases=encodeURIComponent(document.getElementById('phrases').value);
 const mode=document.getElementById('everything').checked?'all':'phrases';
 const threshold=document.getElementById('threshold').value;const t0=performance.now();
 const live=document.getElementById('live').checked;
 const liveMode=mode==='all'?(phrases?'focused_open':'open'):'focused';
 const url=live?`live?focus=demo&mode=${liveMode}&phrases=${phrases}&threshold=${threshold}`:`detect?mode=${mode}&phrases=${phrases}&threshold=${threshold}`;
 try{const r=await fetch(url,{method:'POST',body:blob});
  const out=await r.json();draw(out,scale,performance.now()-t0);}catch(e){document.getElementById('stats').textContent='error: '+e;}
 requestAnimationFrame(loop);}
function draw(out,scale,roundTrip){
 overlay.width=video.clientWidth;overlay.height=video.clientHeight;const k=overlay.width/(video.videoWidth*scale);
 ctx.clearRect(0,0,overlay.width,overlay.height);ctx.lineWidth=3;ctx.font='15px system-ui';const counts={};
 for(const d of out.detections){const c=colours[d.phrase]||(colours[d.phrase]=palette[Object.keys(colours).length%palette.length]);
  const [x0,y0,x1,y1]=d.box.map(v=>v*k);ctx.strokeStyle=c;ctx.strokeRect(x0,y0,x1-x0,y1-y0);
  if(document.getElementById('tags').checked){const label=`${d.phrase} ${(d.score*100).toFixed(0)}%${d.source==='tracked'?' · seguido':''}`;const tw=ctx.measureText(label).width+8;
  ctx.fillStyle=c;ctx.fillRect(x0,Math.max(0,y0-20),tw,20);ctx.fillStyle='#04121c';ctx.fillText(label,x0+4,Math.max(14,y0-5));}
  counts[d.phrase]=(counts[d.phrase]||0)+1;}
 document.getElementById('seen').innerHTML=Object.entries(counts).map(([p,n])=>`<li>${n} × ${p}</li>`).join('')||'<li>nada sobre el umbral</li>';
 const age=out.inference_age_ms==null?'':` · leído hace ${out.inference_age_ms} ms`;
 const how=out.decision?` · ${out.decision.do}`:'';
 document.getElementById('stats').textContent=`respuesta ${out.latency_ms} ms · ida y vuelta ${roundTrip.toFixed(0)} ms · ${(1000/roundTrip).toFixed(1)} fps${how}${age}`;}
</script></body></html>
"#;

pub struct Service {
    pub detector: Detector,
    pub actor: Option<Actor>,
    /// The reading head (`read_decoder`), when the release has one.
    pub reader: Option<crate::read::Reader>,
    pub info: Value,
    pub rem_spool: Option<PathBuf>,
    pub skills_dir: PathBuf,
    /// What Policy approved for live foci (`/live`): highest priority class, inference
    /// rate, memory, expert residency.
    pub live_envelope: Envelope,
    /// Where `scene.state` is exported to other processes (`scene.state.sock`).
    pub slink_dir: Option<PathBuf>,
    latencies: Vec<f64>,
    started: Instant,
}

impl Service {
    pub fn new(detector: Detector, actor: Option<Actor>, info: Value) -> Self {
        let skills_dir = std::env::var_os("IRIS_SKILLS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/workspace/iris-skills"));
        Self {
            detector,
            actor,
            reader: None,
            info,
            rem_spool: None,
            skills_dir,
            live_envelope: Envelope {
                max_priority: Priority::ActiveFocus,
                max_fps: 30.0,
                max_vram_bytes: 0,
                allow_partial_experts: false,
                expert_floor: 0.5,
            },
            slink_dir: None,
            latencies: Vec::new(),
            started: Instant::now(),
        }
    }

    pub fn with_rem_spool(mut self, rem_spool: Option<PathBuf>) -> Self {
        self.rem_spool = rem_spool;
        self
    }

    pub fn with_reader(mut self, reader: Option<crate::read::Reader>) -> Self {
        self.reader = reader;
        self
    }

    pub fn with_slink_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.slink_dir = dir;
        self
    }

    pub fn with_live_envelope(mut self, envelope: Envelope) -> Self {
        self.live_envelope = envelope;
        self
    }

    pub fn with_skills_dir(mut self, skills_dir: PathBuf) -> Self {
        self.skills_dir = skills_dir;
        self
    }

    fn record(&mut self, latency: f64) {
        self.latencies.push(latency);
        if self.latencies.len() > 200 {
            self.latencies.remove(0);
        }
    }

    pub fn health(&self) -> Value {
        let mut ordered = self.latencies.clone();
        ordered.sort_by(f64::total_cmp);
        let mut info = self.info.clone();
        info["requests"] = json!(ordered.len());
        info["p50_ms"] = ordered
            .get(ordered.len() / 2)
            .map_or(Value::Null, |v| json!((v * 10.0).round() / 10.0));
        info["text"] = self.detector.phrases.describe();
        info["runtime"] = json!("archie-iris-runtime");
        info["skill_adapters"] = json!(self.actor.as_ref().is_some_and(|a| a.bank.is_some()));
        info["rem_recording"] = json!(self.rem_spool.is_some());
        info["reading"] = json!(self.reader.is_some());
        info
    }

    pub fn detect(
        &mut self,
        frame: &Frame,
        query: &HashMap<String, String>,
    ) -> Result<Value, Error> {
        let image = &frame.image;
        let phrases: Vec<String> = query
            .get("phrases")
            .map(|p| {
                p.split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let threshold = query
            .get("threshold")
            .map_or(Ok(0.35), |t| t.parse::<f32>())
            .map_err(|_| Error::Refused("threshold is not a number".into()))?
            .clamp(0.01, 0.99);
        let everything = query.get("mode").is_some_and(|m| m == "all");
        let embed = query.get("embed").is_some_and(|e| e == "1");
        let started = Instant::now();
        let hits = if everything || phrases.is_empty() {
            self.detector
                .detect_everything_frame(frame, threshold, &phrases, 60, 0.5, embed)?
        } else {
            self.detector
                .detect_frame(frame, &phrases, threshold, 20, 0.5, embed)?
        };
        let latency = started.elapsed().as_secs_f64() * 1000.0;
        self.record(latency);
        Ok(
            json!({"detections": hits, "latency_ms": (latency * 10.0).round() / 10.0,
                  "image_size": [image.width, image.height], "step": self.info["step"]}),
        )
    }

    /// `/read?boxes=x0,y0,x1,y1;…`: the text of each crop (pixel boxes of the frame; the
    /// whole frame when none), read by IRIS's own reading head.
    pub fn read(&mut self, frame: &Frame, query: &HashMap<String, String>) -> Result<Value, Error> {
        let reader = self
            .reader
            .as_ref()
            .ok_or_else(|| Error::Refused("these weights have no reading head".into()))?;
        let boxes = crate::read::parse_boxes(query.get("boxes").map_or("", String::as_str))?;
        let started = Instant::now();
        let texts = reader.read(&self.detector.model, &frame.image, &boxes)?;
        let latency = started.elapsed().as_secs_f64() * 1000.0;
        self.record(latency);
        let reads: Vec<Value> = if boxes.is_empty() {
            texts
                .into_iter()
                .map(|text| json!({"box": null, "text": text}))
                .collect()
        } else {
            boxes
                .iter()
                .zip(texts)
                .map(|(b, text)| json!({"box": b, "text": text}))
                .collect()
        };
        Ok(
            json!({"reads": reads, "latency_ms": (latency * 10.0).round() / 10.0,
                  "image_size": [frame.image.width, frame.image.height], "step": self.info["step"]}),
        )
    }

    pub fn act(&mut self, frame: &Frame, query: &HashMap<String, String>) -> Result<Value, Error> {
        let field = |name: &str, limit: usize| {
            query
                .get(name)
                .map_or(String::new(), |v| v.chars().take(limit).collect())
        };
        let instruction = field("instruction", 300);
        let skill = field("skill", 300);
        let mut adapter = field("adapter", 64);
        if adapter == "auto" {
            adapter.clone_from(&skill);
        }
        let record_run = field("record", 128);
        let record_app = field("app", 64);
        let sensitivity = query
            .get("sensitivity")
            .map_or("public".to_owned(), |v| v.chars().take(32).collect());
        let session = query
            .get("session")
            .map_or("default".to_owned(), |v| v.chars().take(64).collect());
        let now = self.started.elapsed().as_secs_f64();
        let skills_dir = self.skills_dir.clone();
        let recording = match (!record_run.is_empty(), &self.rem_spool) {
            (true, Some(spool)) => {
                let base_sha = self.info["sha256"].as_str().unwrap_or_default();
                Some(Recording::validate_new(
                    spool.clone(),
                    &record_app,
                    &record_run,
                    base_sha,
                    &sensitivity,
                )?)
            }
            _ => None,
        };
        let actor = self
            .actor
            .as_mut()
            .ok_or_else(|| Error::Refused("these weights have no action head".into()))?;
        if !adapter.is_empty() {
            actor.use_skill(&adapter, &skills_dir)?;
        }
        let started = Instant::now();
        let mut control = actor.act_with_options(
            &self.detector.model,
            self.detector.phrases.as_mut(),
            &session,
            frame,
            &instruction,
            &skill,
            now,
            if adapter.is_empty() {
                None
            } else {
                Some(adapter.as_str())
            },
            recording.as_ref(),
        )?;
        let latency = started.elapsed().as_secs_f64() * 1000.0;
        self.record(latency);
        control["latency_ms"] = json!((latency * 10.0).round() / 10.0);
        control["step"] = self.info["step"].clone();
        Ok(control)
    }

    pub fn act_recorded(&self, query: &HashMap<String, String>) -> Result<Value, Error> {
        let (Some(spool), Some(actor)) = (&self.rem_spool, &self.actor) else {
            return Ok(json!({"samples": [], "errors": 0}));
        };
        let app: String = query
            .get("app")
            .map_or(String::new(), |v| v.chars().take(64).collect());
        let run: String = query
            .get("run")
            .map_or(String::new(), |v| v.chars().take(128).collect());
        let samples = recorded_samples(spool, &app, &run)?;
        Ok(json!({"samples": samples, "errors": actor.record_errors}))
    }
}

type Shared = Arc<Mutex<Service>>;

fn reply(result: Result<Value, Error>) -> Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}

async fn blocking(
    state: Shared,
    work: impl FnOnce(&mut Service) -> Result<Value, Error> + Send + 'static,
) -> Response {
    let joined = tokio::task::spawn_blocking(move || {
        let mut service = state
            .lock()
            .map_err(|_| Error::Refused("the model is unavailable".into()))?;
        work(&mut service)
    })
    .await;
    match joined {
        Ok(result) => reply(result),
        Err(_) => reply(Err(Error::Refused("inference stopped".into()))),
    }
}

/// Decodes and letterboxes the frame on a blocking thread before taking the model, so the
/// next request's preparation runs while the current one holds the GPU.
async fn with_frame(
    state: Shared,
    size: usize,
    body: Bytes,
    work: impl FnOnce(&mut Service, &Frame) -> Result<Value, Error> + Send + 'static,
) -> Response {
    let joined = tokio::task::spawn_blocking(move || {
        let frame = Frame::decode(&body, size)?;
        let mut service = state
            .lock()
            .map_err(|_| Error::Refused("the model is unavailable".into()))?;
        work(&mut service, &frame)
    })
    .await;
    match joined {
        Ok(result) => reply(result),
        Err(_) => reply(Err(Error::Refused("inference stopped".into()))),
    }
}

fn frame_size_ok(body: &Bytes) -> Option<Response> {
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Some(
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({"error": "frame_size_invalid"})),
            )
                .into_response(),
        );
    }
    None
}

/// Reads a batch of live requests: focused requests of up to four foci in one pass with a
/// capacity per image, open sweeps one by one. One result per request, in order. The
/// canvases are built before the model is taken, in parallel.
fn live_inference(
    state: &Shared,
    size: usize,
    batch: &[InferRequest],
) -> Vec<Result<Vec<Detection>, String>> {
    use rayon::prelude::*;
    let frames: Vec<Frame> = batch.par_iter().map(|r| r.frame(size)).collect();
    let mut service = match state.lock() {
        Ok(service) => service,
        Err(_) => {
            return batch
                .iter()
                .map(|_| Err("the model is unavailable".into()))
                .collect()
        }
    };
    let mut out: Vec<Result<Vec<Detection>, String>> =
        batch.iter().map(|_| Err("not read".into())).collect();
    let focused: Vec<usize> = (0..batch.len()).filter(|&i| !batch[i].open).collect();
    for chunk in focused.chunks(4) {
        let requests: Vec<(&Frame, &[String])> = chunk
            .iter()
            .map(|&i| (&frames[i], batch[i].phrases.as_slice()))
            .collect();
        let lowest = chunk
            .iter()
            .map(|&i| batch[i].threshold)
            .fold(f32::INFINITY, f32::min);
        match service
            .detector
            .detect_many(&requests, lowest, 20, 0.5, false)
        {
            Ok(lists) => {
                for (&i, found) in chunk.iter().zip(lists) {
                    let floor = f64::from(batch[i].threshold);
                    out[i] = Ok(found.into_iter().filter(|d| d.score >= floor).collect());
                }
            }
            Err(error) => {
                for &i in chunk {
                    out[i] = Err(error.to_string());
                }
            }
        }
    }
    for (i, request) in batch.iter().enumerate().filter(|(_, r)| r.open) {
        out[i] = service
            .detector
            .detect_everything_frame(
                &frames[i],
                request.threshold,
                &request.phrases,
                60,
                0.5,
                false,
            )
            .map_err(|error| error.to_string());
    }
    out
}

/// A live focus's budget from `/live`'s query, as advice clamped by the envelope.
fn live_advice(query: &HashMap<String, String>) -> Result<Option<(ResourceAdvice, f32)>, Error> {
    let focus = query
        .get("focus")
        .cloned()
        .ok_or_else(|| Error::Refused("live needs a focus".into()))?;
    let configured = [
        "phrases",
        "mode",
        "fps",
        "priority",
        "strength",
        "threshold",
    ]
    .iter()
    .any(|key| query.contains_key(*key));
    if !configured {
        return Ok(None);
    }
    let number = |key: &str, default: f32| -> Result<f32, Error> {
        query.get(key).map_or(Ok(default), |v| {
            v.parse::<f32>()
                .map_err(|_| Error::Refused(format!("{key} is not a number")))
        })
    };
    let mode = match query.get("mode").map(String::as_str) {
        None | Some("focused") | Some("phrases") => FocusMode::Focused,
        Some("open") | Some("all") => FocusMode::Open,
        Some("focused_open") => FocusMode::FocusedAndOpen,
        Some("off") => FocusMode::Off,
        Some(other) => return Err(Error::Refused(format!("unknown mode {other}"))),
    };
    let priority = match query.get("priority").map(String::as_str) {
        None | Some("active_focus") => Priority::ActiveFocus,
        Some("voice_reflex") => Priority::VoiceReflex,
        Some("wanda_interactive") => Priority::WandaInteractive,
        Some("background") => Priority::Background,
        Some("batch") => Priority::Batch,
        Some(other) => return Err(Error::Refused(format!("unknown priority {other}"))),
    };
    let phrases: Vec<String> = query
        .get("phrases")
        .map(|p| {
            p.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(Some((
        ResourceAdvice {
            focus,
            share: number("strength", 1.0)?,
            mode,
            phrases,
            read_text: false,
            priority,
            max_fps: number("fps", f32::MAX)?,
        },
        number("threshold", 0.35)?,
    )))
}

fn live_status(live: &LiveShared) -> Value {
    let foci: Vec<Value> = live
        .hub
        .lock()
        .map(|hub| {
            hub.foci()
                .into_iter()
                .map(|(budget, phrases, state)| {
                    json!({"budget": budget, "phrases": phrases, "state": state})
                })
                .collect()
        })
        .unwrap_or_default();
    let costs = live
        .costs
        .lock()
        .map(|c| json!({"focused_ms": c.estimate("focused"), "open_ms": c.estimate("open")}))
        .unwrap_or(Value::Null);
    json!({"foci": foci, "queued": live.queued(), "estimates": costs, "envelope": live.envelope})
}

pub fn router(service: Service) -> Router {
    let size = service.detector.model.config.image_size;
    let live = LiveShared::new(LiveConfig::default(), service.live_envelope.clone());
    #[cfg(target_os = "linux")]
    if let Some(dir) = &service.slink_dir {
        match live.export_scene(dir) {
            Ok(path) => println!(
                "{}",
                json!({"event": "slink_exported", "link": "scene.state", "socket": path})
            ),
            Err(error) => eprintln!(
                "{}",
                json!({"event": "slink_export_failed", "error": error.to_string()})
            ),
        }
    }
    let state: Shared = Arc::new(Mutex::new(service));
    let worker_state = state.clone();
    spawn_worker(live.clone(), move |batch| {
        live_inference(&worker_state, size, batch)
    });
    let live_frames = live.clone();
    let live_advice_state = live.clone();
    let live_status_state = live.clone();
    Router::new()
        .route(
            "/live",
            post(
                move |Query(query): Query<HashMap<String, String>>, body: Bytes| {
                    let live = live_frames.clone();
                    async move {
                        if let Some(refusal) = frame_size_ok(&body) {
                            return refusal;
                        }
                        let joined = tokio::task::spawn_blocking(move || -> Result<Value, Error> {
                            let decoding = Instant::now();
                            let image = crate::preprocess::decode(&body)?;
                            let decode_ms = decoding.elapsed().as_secs_f64() * 1000.0;
                            let focus = query
                                .get("focus")
                                .cloned()
                                .ok_or_else(|| Error::Refused("live needs a focus".into()))?;
                            if let Some((advice, threshold)) = live_advice(&query)? {
                                live.advise(&advice, threshold)?;
                            }
                            let started = Instant::now();
                            let (reply, request) = live
                                .hub
                                .lock()
                                .map_err(|_| Error::Refused("live state unavailable".into()))?
                                .observe(&focus, &image, live.now_ms())?;
                            if let Some(request) = request {
                                live.submit(request);
                            }
                            if !matches!(reply.decision, archie_accel::plan::Decision::Idle) {
                                live.publish_scene(&reply, live.now_ms())?;
                            }
                            let mut out = serde_json::to_value(&reply)
                                .map_err(|e| Error::Refused(e.to_string()))?;
                            out["latency_ms"] =
                                json!((started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0);
                            out["decode_ms"] = json!((decode_ms * 10.0).round() / 10.0);
                            out["image_size"] = json!([image.width, image.height]);
                            Ok(out)
                        })
                        .await;
                        match joined {
                            Ok(result) => reply(result),
                            Err(_) => reply(Err(Error::Refused("live stopped".into()))),
                        }
                    }
                },
            )
            .get(move || {
                let live = live_status_state.clone();
                async move { Json(live_status(&live)) }
            }),
        )
        .route(
            "/live/advice",
            post(move |Json(advice): Json<ResourceAdvice>| {
                let live = live_advice_state.clone();
                async move {
                    reply(
                        live.advise(&advice, 0.35)
                            .map(|_| json!({"focus": advice.focus, "applied": true})),
                    )
                }
            }),
        )
        .route(
            "/experts",
            get(|State(state): State<Shared>| async move {
                let report = state
                    .lock()
                    .map_err(|_| Error::Refused("the model is unavailable".into()))
                    .and_then(|s| {
                        Ok(json!({
                            "layers": s.detector.model.expert_usage()?,
                            "expert_bytes": s.detector.model.expert_bytes(),
                        }))
                    });
                reply(report)
            }),
        )
        .route("/", get(|| async { Html(DEMO_PAGE) }))
        .route("/index.html", get(|| async { Html(DEMO_PAGE) }))
        .route(
            "/health",
            get(|State(state): State<Shared>| async move {
                let health = state
                    .lock()
                    .map(|s| s.health())
                    .unwrap_or_else(|_| json!({"error": "unavailable"}));
                Json(health)
            }),
        )
        .route(
            "/detect",
            post(
                move |State(state): State<Shared>,
                      Query(query): Query<HashMap<String, String>>,
                      body: Bytes| async move {
                    if let Some(refusal) = frame_size_ok(&body) {
                        return refusal;
                    }
                    with_frame(state, size, body, move |s, f| s.detect(f, &query)).await
                },
            ),
        )
        .route(
            "/read",
            post(
                move |State(state): State<Shared>,
                      Query(query): Query<HashMap<String, String>>,
                      body: Bytes| async move {
                    if let Some(refusal) = frame_size_ok(&body) {
                        return refusal;
                    }
                    with_frame(state, size, body, move |s, f| s.read(f, &query)).await
                },
            ),
        )
        .route(
            "/act",
            post(
                move |State(state): State<Shared>,
                      Query(query): Query<HashMap<String, String>>,
                      body: Bytes| async move {
                    if let Some(refusal) = frame_size_ok(&body) {
                        return refusal;
                    }
                    with_frame(state, size, body, move |s, f| s.act(f, &query)).await
                },
            ),
        )
        .route(
            "/act/reset",
            post(
                |State(state): State<Shared>, Query(query): Query<HashMap<String, String>>| async move {
                    let session = query
                        .get("session")
                        .map_or("default".to_owned(), Clone::clone);
                    if let Ok(mut service) = state.lock() {
                        if let Some(actor) = service.actor.as_mut() {
                            actor.reset(&session);
                        }
                    }
                    Json(json!({"reset": true}))
                },
            ),
        )
        .route(
            "/act/recorded",
            post(
                |State(state): State<Shared>, Query(query): Query<HashMap<String, String>>| async move {
                    blocking(state, move |s| s.act_recorded(&query)).await
                },
            ),
        )
        .with_state(state)
        .layer(frame_body_limit())
}

fn frame_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_FRAME_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn frame_extraction_accepts_eight_mib_but_rejects_larger_bodies() {
        let app = Router::new()
            .route(
                "/frame",
                post(|body: Bytes| async move { body.len().to_string() }),
            )
            .layer(frame_body_limit());
        for (size, status) in [
            (3 * 1024 * 1024, StatusCode::OK),
            (MAX_FRAME_BYTES, StatusCode::OK),
            (MAX_FRAME_BYTES + 1, StatusCode::PAYLOAD_TOO_LARGE),
        ] {
            let request = Request::builder()
                .uri("/frame")
                .method("POST")
                .body(Body::from(vec![0u8; size]))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), status, "body size {size}");
        }
    }

    #[test]
    fn live_queries_become_advice_and_the_envelope_has_the_last_word() {
        let query = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        assert!(live_advice(&query(&[])).is_err(), "a focus is required");
        assert!(live_advice(&query(&[("focus", "cam")])).unwrap().is_none());
        let (advice, threshold) = live_advice(&query(&[
            ("focus", "cam"),
            ("mode", "all"),
            ("phrases", "taza, ,llaves"),
            ("priority", "voice_reflex"),
            ("fps", "240"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(advice.mode, FocusMode::Open);
        assert_eq!(advice.phrases, ["taza", "llaves"]);
        assert_eq!(threshold, 0.35);
        let envelope = Envelope {
            max_priority: Priority::ActiveFocus,
            max_fps: 30.0,
            max_vram_bytes: 0,
            allow_partial_experts: false,
            expert_floor: 0.5,
        };
        let (budget, _) = advice.apply(&envelope).unwrap();
        assert_eq!(budget.priority, Priority::ActiveFocus);
        assert_eq!(budget.max_fps, 30.0);
        assert!(live_advice(&query(&[("focus", "cam"), ("mode", "everything")])).is_err());
        assert!(live_advice(&query(&[("focus", "cam"), ("fps", "fast")])).is_err());
    }
}
