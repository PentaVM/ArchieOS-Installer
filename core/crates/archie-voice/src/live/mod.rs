//! A live session: Archie with ears, a voice and eyes, reached from a browser page.
//!
//! ```text
//!  browser ── mic 16 kHz ──► ws ─► Hearing::Stream ─► conversation (ears, floor, Nemotron, voice)
//!     ▲                                                              │
//!     └──── voice PCM, what was heard/said, the scene ◄── ws ◄───────┘
//!  browser ── camera JPEG ─► ws ─► eyes (IRIS) ─► scene ─► context of every turn
//!                                        └──► curiosity ─► identify ─► search ─► buffer
//! ```
//!
//! The conversation is the same loop `converse` runs, with the browser in place of the
//! microphone and the speaker. The floor still decides whether Archie speaks: it answers
//! what is said to it, and what curiosity found out waits in the buffer until it is asked
//! for. The page is served on loopback only; reach it through an SSH tunnel.
//!
//! What the model is told it sees is only ever what IRIS read, said as IRIS's: what the
//! camera shows now, how old that image is, what was there before and is not any more
//! ([`memory`]). It goes beside the utterance it answers, after the past conversation, so
//! nothing Archie said earlier about the room can pass for what is true now.

pub mod curiosity;
pub mod eyes;
pub mod language;
pub mod memory;
pub mod style;

use crate::conversation::{Moment, VoiceOut};
use archie_model_runtime::ToolDefinition;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use curiosity::{Buffer, Identifier, Researcher};
use eyes::{IrisEyes, Scene};
use memory::Memory;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Notify};

/// Who Archie is in a live session.
pub const PERSONA: &str = "Eres Archie, el asistente de ArchieOS, y ahora tienes ojos: una \
cámara que tu visión, IRIS, lee en vivo. Lo único que ves es lo que el contexto de este \
momento dice que IRIS ve ahora; IRIS todavía está aprendiendo y se equivoca, así que di \
«me parece» cuando no es seguro, y nunca afirmes que algo está si ese contexto no lo dice. \
Lo que se veía antes en la conversación puede haber cambiado: si el contexto dice que algo \
ya no se ve, o que no hay nadie, dilo así. Si la cámara no manda imagen, di que no ves. Contesta en el \
idioma en que te hablaron: español de Chile, natural y tuteando (nunca voseo), o inglés. Todo \
lo que escribes se dice en voz alta: frases cortas, sin markdown, sin listas, sin emojis; los \
números y las fechas con palabras. IRIS nombra categorías generales, como juguete, auto o \
botella, no nombres exactos: no inventes qué es exactamente algo. Si te preguntan qué es algo y \
no está en lo que ya averiguaste, usa la herramienta identify; para datos de internet, \
web_search. Lo que ya averiguaste no lo digas si nadie lo pregunta ni responde lo que te \
preguntaron. Nunca digas quién es una persona por su cara ni la busques: eso necesita su \
permiso. No inventes datos que no tienes. Si lo que oíste es un fragmento sin sentido o solo tu nombre, no lo repitas: pregunta en pocas palabras qué necesita, o quédate callado.";

/// The tools a live session adds: looking closer, and reading the web.
pub fn tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "identify".into(),
            description: "Mira de cerca un objeto frente a la cámara y averigua qué es \
                          exactamente (marca, modelo, personaje) y datos sobre él. Nunca \
                          personas."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {"what": {"type": "string",
                    "description": "Cuál objeto, como lo dijo la persona: «esto que tengo en la mano», «el auto»."}},
                "required": ["what"]
            }),
        },
        ToolDefinition {
            name: "web_search".into(),
            description: "Busca en internet y devuelve lo que dicen las primeras fuentes. \
                          Solo con palabras públicas: nada privado de la persona."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {"query": {"type": "string", "description": "Qué buscar."}},
                "required": ["query"]
            }),
        },
    ]
}

/// What the page is sent.
#[derive(Clone)]
pub enum Outgoing {
    /// Tag byte 1, then PCM16 little-endian at the voice's rate.
    Audio(Arc<Vec<u8>>),
    Text(Arc<str>),
}

/// Everything a live session shares between the page, the eyes and the conversation.
pub struct Senses {
    pub scene: Mutex<Scene>,
    /// What IRIS has seen over time, thing by thing.
    pub memory: Mutex<Memory>,
    pub buffer: Mutex<Buffer>,
    pub identifier: Identifier,
    pub researcher: Option<Researcher>,
    audio: mpsc::Sender<Vec<f32>>,
    frame: Mutex<Option<Arc<Vec<u8>>>>,
    frame_ready: Notify,
    out: broadcast::Sender<Outgoing>,
    /// Looks closer that curiosity may still take this session: each is a paid call.
    curiosity_left: AtomicU32,
    last_look: Mutex<Option<Instant>>,
    voice_rate: u32,
    /// What the person said last, so the model is told which language to answer in.
    last_heard: Mutex<String>,
    /// The page whose microphone the ears hear: the newest. Two pages streaming into the
    /// one ear interleave their frames, and the recogniser hears noise at twice the speed.
    microphone_owner: AtomicU64,
    pages: AtomicU64,
}

impl Senses {
    pub fn new(
        audio: mpsc::Sender<Vec<f32>>,
        identifier: Identifier,
        researcher: Option<Researcher>,
        voice_rate: u32,
        curiosity_budget: u32,
    ) -> Arc<Self> {
        let (out, _) = broadcast::channel(512);
        Arc::new(Self {
            scene: Mutex::new(Scene::default()),
            memory: Mutex::new(Memory::default()),
            buffer: Mutex::new(Buffer::default()),
            identifier,
            researcher,
            audio,
            frame: Mutex::new(None),
            frame_ready: Notify::new(),
            out,
            curiosity_left: AtomicU32::new(curiosity_budget),
            last_look: Mutex::new(None),
            voice_rate,
            last_heard: Mutex::new(String::new()),
            microphone_owner: AtomicU64::new(0),
            pages: AtomicU64::new(0),
        })
    }

    /// Remembers what was just heard (before the turn that answers it asks for context).
    pub fn heard(&self, text: &str) {
        *self.last_heard.lock().expect("heard lock") = text.to_string();
    }

    pub fn publish(&self, value: serde_json::Value) {
        let _ = self.out.send(Outgoing::Text(value.to_string().into()));
    }

    /// What the model should know this turn: the clock, what the camera shows now and
    /// what it showed before, and what was found out and not yet said.
    pub fn context(&self, clock: &str) -> String {
        let scene = {
            let scene = self.scene.lock().expect("scene lock");
            self.memory.lock().expect("memory lock").describe(&scene, Instant::now())
        };
        let known = self.buffer.lock().expect("buffer lock").describe();
        let heard = self.last_heard.lock().expect("heard lock").clone();
        // Last, where a small model weighs it most, and in the language it asks for.
        let answer_in = match language::spoken_language(&heard) {
            "en" => "IMPORTANT: the person just spoke English. Reply only in English, even though these instructions are in Spanish.",
            _ => "La persona te habló en español: responde en español.",
        };
        [clock.to_string(), scene, known, answer_in.to_string()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Carries out a live tool; `None` when the tool is not one of them.
    pub async fn handle_tool(&self, name: &str, raw_arguments: &str) -> Option<String> {
        let arguments: serde_json::Value = serde_json::from_str(raw_arguments).unwrap_or_default();
        match name {
            "identify" => Some(self.identify(arguments["what"].as_str().unwrap_or("")).await),
            "web_search" => Some(self.web_search(arguments["query"].as_str().unwrap_or("")).await),
            _ => None,
        }
    }

    async fn identify(&self, what: &str) -> String {
        let scene = self.scene.lock().expect("scene lock").clone();
        if scene.at.is_none_or(|at| at.elapsed() > Duration::from_secs(10)) {
            return "No me está llegando imagen de la cámara.".into();
        }
        let Some(seen) = eyes::pick(&scene, what).cloned() else {
            return "No veo ningún objeto claro frente a la cámara (a las personas no las identifico).".into();
        };
        if let Some(known) = self
            .buffer
            .lock()
            .expect("buffer lock")
            .entries()
            .find(|entry| entry.seen_as == seen.phrase && curiosity_overlap(entry, &seen))
            .cloned()
        {
            return spoken(&known);
        }
        match curiosity::look_closer(&scene, &seen, &self.identifier, self.researcher.as_ref()).await {
            Ok(known) => {
                let told = spoken(&known);
                self.publish(json!({"type": "known", "known": &known}));
                self.buffer.lock().expect("buffer lock").remember(known);
                told
            }
            Err(error) => format!("No pude mirarlo de cerca: {error}"),
        }
    }

    async fn web_search(&self, query: &str) -> String {
        let Some(researcher) = &self.researcher else {
            return "No tengo búsqueda web configurada (falta TAVILY_API_KEY).".into();
        };
        if query.trim().is_empty() {
            return "No se dijo qué buscar.".into();
        }
        match researcher.search(query).await {
            Ok(results) if !results.is_empty() => {
                let titles: Vec<String> = results.iter().map(|(title, _, _)| title.clone()).collect();
                format!(
                    "Fuentes: {}. Dicen: {}",
                    titles.join("; "),
                    curiosity::facts_from(&results)
                )
            }
            Ok(_) => "La búsqueda no devolvió nada.".into(),
            Err(error) => format!("La búsqueda falló: {error}"),
        }
    }
}

fn curiosity_overlap(known: &curiosity::Known, seen: &eyes::Seen) -> bool {
    let a = known.bbox;
    let b = seen.bbox;
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    w * h > 0.0
}

fn spoken(known: &curiosity::Known) -> String {
    let by = match known.by.as_str() {
        "web" => "según la búsqueda por imagen",
        "vision-model" => "según el modelo de visión",
        _ => "IRIS solo sabe la categoría",
    };
    let mut told = format!("Es {} ({by}).", known.name);
    if !known.facts.is_empty() {
        told.push_str(&format!(" Datos: {}", known.facts));
    }
    told
}

/// A moment of the conversation, as the page is told it.
pub fn moment_json(moment: &Moment) -> serde_json::Value {
    match moment {
        Moment::Heard { text, .. } => json!({"type": "heard", "text": text}),
        Moment::Floor { action, reason, .. } => json!({"type": "floor", "action": action, "reason": reason}),
        Moment::Said { text, first_audio_ms, stopped, .. } => {
            json!({"type": "said", "text": text, "first_audio_ms": first_audio_ms, "stopped": stopped})
        }
        Moment::Speaking { text } => json!({"type": "speaking", "text": text}),
        Moment::Proposed { tool, arguments } => json!({"type": "tool", "tool": tool, "arguments": arguments}),
        Moment::Did { report } => json!({"type": "did", "report": report}),
        Moment::Interrupted => json!({"type": "interrupted"}),
        Moment::Alert(message) => json!({"type": "alert", "text": message}),
        Moment::Level { level, voiced } => json!({"type": "level", "level": level, "voiced": voiced}),
        Moment::Ready => json!({"type": "ready"}),
    }
}

/// Carries the voice to the page: a thread, because the conversation's voice is a
/// `std` channel and the page's socket lives on the runtime.
pub fn forward_voice(voice: mpsc::Receiver<VoiceOut>, senses: Arc<Senses>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for out in voice {
            match out {
                VoiceOut::Audio(pcm) => {
                    let mut bytes = Vec::with_capacity(1 + pcm.len() * 2);
                    bytes.push(1u8);
                    for sample in pcm {
                        bytes.extend_from_slice(&sample.to_le_bytes());
                    }
                    let _ = senses.out.send(Outgoing::Audio(Arc::new(bytes)));
                }
                VoiceOut::Hush => senses.publish(json!({"type": "hush"})),
            }
        }
    })
}

/// Serves the page and its socket on `127.0.0.1:port`, and runs the eyes and curiosity.
pub async fn serve(senses: Arc<Senses>, eyes: IrisEyes, port: u16) -> Result<(), String> {
    tokio::spawn(see(senses.clone(), eyes));
    tokio::spawn(wonder(senses.clone()));
    let app = Router::new()
        .route("/", get(|| async { Html(PAGE) }))
        .route("/ws", get(upgrade))
        .with_state(senses);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("binding 127.0.0.1:{port}: {e}"))?;
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

const PAGE: &str = include_str!("page.html");

async fn upgrade(ws: WebSocketUpgrade, State(senses): State<Arc<Senses>>) -> impl IntoResponse {
    ws.max_message_size(4 << 20)
        .on_upgrade(move |socket| session(socket, senses))
}

async fn session(socket: WebSocket, senses: Arc<Senses>) {
    let page = senses.pages.fetch_add(1, Ordering::SeqCst) + 1;
    senses.microphone_owner.store(page, Ordering::SeqCst);
    senses.publish(json!({"type": "microphone_owner", "page": page}));
    let (mut sink, mut stream) = socket.split();
    let mut outgoing = senses.out.subscribe();
    let hello = json!({
        "type": "hello",
        "page": page,
        "voice_rate": senses.voice_rate,
        "identify": senses.identifier.means(),
        "search": senses.researcher.is_some(),
    });
    if sink.send(Message::Text(hello.to_string())).await.is_err() {
        return;
    }
    let writer = tokio::spawn(async move {
        loop {
            let message = match outgoing.recv().await {
                Ok(Outgoing::Audio(bytes)) => Message::Binary(bytes.as_ref().clone()),
                Ok(Outgoing::Text(text)) => Message::Text(text.to_string()),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });
    // What the microphone actually delivers: a page resampling wrong sends sound the ears
    // cannot read, and the level meter alone does not show it.
    let mut counted = (Instant::now(), 0usize, 0f32);
    while let Some(Ok(message)) = stream.next().await {
        if counted.0.elapsed() >= Duration::from_secs(5) && counted.1 > 0 {
            let rate = counted.1 as f32 / counted.0.elapsed().as_secs_f32();
            eprintln!("microphone: {rate:.0} samples/s (the ears want 16000), peak {:.3}", counted.2);
            senses.publish(json!({"type": "mic", "samples_per_s": rate.round(), "peak": counted.2}));
            counted = (Instant::now(), 0, 0.0);
        }
        match message {
            Message::Binary(data) if data.len() > 1 => match data[0] {
                // Microphone: f32 little-endian at 16 kHz, in 20 ms frames for the loop.
                1 if senses.microphone_owner.load(Ordering::SeqCst) != page => {}
                1 => {
                    let samples: Vec<f32> = data[1..]
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .collect();
                    counted.1 += samples.len();
                    counted.2 = samples.iter().fold(counted.2, |peak, s| peak.max(s.abs()));
                    for frame in samples.chunks(crate::audio::FRAME) {
                        let _ = senses.audio.send(frame.to_vec());
                    }
                }
                // Camera: one JPEG, replacing whatever the eyes have not read yet.
                2 => {
                    *senses.frame.lock().expect("frame lock") = Some(Arc::new(data[1..].to_vec()));
                    senses.frame_ready.notify_one();
                }
                _ => {}
            },
            Message::Close(_) => break,
            _ => {}
        }
    }
    writer.abort();
}

/// Reads each new frame with IRIS, one at a time, and publishes the scene.
async fn see(senses: Arc<Senses>, eyes: IrisEyes) {
    loop {
        senses.frame_ready.notified().await;
        let Some(frame) = senses.frame.lock().expect("frame lock").take() else {
            continue;
        };
        let asked = Instant::now();
        match eyes.look(frame).await {
            Ok(scene) => {
                let (seen, recalled) = {
                    let mut memory = senses.memory.lock().expect("memory lock");
                    memory.update(&scene);
                    let seen: Vec<serde_json::Value> = scene
                        .seen
                        .iter()
                        .enumerate()
                        .map(|(index, item)| json!({
                            "phrase": item.phrase, "score": item.score, "box": item.bbox,
                            "id": memory.id_of(index),
                        }))
                        .collect();
                    (seen, memory.recall(Instant::now()))
                };
                senses.publish(json!({
                    "type": "scene", "seen": seen, "width": scene.width,
                    "height": scene.height, "step": scene.step,
                    "iris_ms": asked.elapsed().as_millis() as u64, "memory": recalled,
                }));
                *senses.scene.lock().expect("scene lock") = scene;
            }
            Err(error) => senses.publish(json!({"type": "alert", "text": error})),
        }
    }
}

/// Every few seconds, looks closer at the most prominent object it does not know yet.
async fn wonder(senses: Arc<Senses>) {
    if senses.identifier.means().is_empty() {
        return; // nothing to look closer with: IRIS's categories are all there is
    }
    let mut tick = tokio::time::interval(Duration::from_secs(4));
    loop {
        tick.tick().await;
        if senses.curiosity_left.load(Ordering::Relaxed) == 0 {
            continue;
        }
        let recent = senses
            .last_look
            .lock()
            .expect("look lock")
            .is_some_and(|at| at.elapsed() < Duration::from_secs(12));
        if recent {
            continue;
        }
        let scene = senses.scene.lock().expect("scene lock").clone();
        if scene.at.is_none_or(|at| at.elapsed() > Duration::from_secs(3)) {
            continue;
        }
        let Some(seen) = curiosity::next_to_look_at(&scene, &senses.buffer.lock().expect("buffer lock")).cloned() else {
            continue;
        };
        *senses.last_look.lock().expect("look lock") = Some(Instant::now());
        senses.curiosity_left.fetch_sub(1, Ordering::Relaxed);
        match curiosity::look_closer(&scene, &seen, &senses.identifier, senses.researcher.as_ref()).await {
            Ok(known) => {
                senses.publish(json!({"type": "known", "known": &known}));
                senses.buffer.lock().expect("buffer lock").remember(known);
            }
            Err(error) => senses.publish(json!({"type": "alert", "text": format!("curiosidad: {error}")})),
        }
    }
}
