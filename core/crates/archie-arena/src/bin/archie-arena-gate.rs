//! `archie-arena-gate` — the one thing left running on the training host between benchmarks.
//!
//! A few megabytes on 127.0.0.1:6080 (reach it through an SSH tunnel). Its page says whether
//! the ArchieOS session is up, starts and stops it, starts the benchmark suite, and shows
//! every Archie Arena run: score, critic, the picture beside its reference, and the full log.
//! When the session is up it also serves the noVNC viewer and carries its WebSocket to the
//! session's VNC server itself, so the desktop is on the same port.
//!
//! ```text
//! archie-arena-gate [--bind 127.0.0.1:6080] [--arena /workspace/arena]
//!                   [--session <archie-session.sh>] [--suite <arena-suite.sh>]
//!                   [--novnc /usr/share/novnc] [--vnc 127.0.0.1:5901]
//! ```

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Gate {
    arena: PathBuf,
    session: String,
    suite: String,
    novnc: PathBuf,
    vnc: String,
    /// The session's run folder: its `wayland-display` and `xdg/` runtime directory.
    run: String,
    /// Frames per second of the live video.
    fps: u32,
}

fn take(args: &mut Vec<String>, flag: &str, default: &str) -> String {
    match args.iter().position(|a| a == flag) {
        Some(at) if at + 1 < args.len() => {
            args.remove(at);
            args.remove(at)
        }
        _ => default.to_string(),
    }
}

/// A path under `root` from URL segments, refusing anything that climbs out of it.
fn inside(root: &Path, rest: &str) -> Option<PathBuf> {
    let relative = Path::new(rest);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some(root.join(relative))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "json" => "application/json; charset=utf-8",
        "jsonl" | "txt" | "log" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

async fn file(path: Option<PathBuf>) -> Response {
    let Some(path) = path else {
        return StatusCode::FORBIDDEN.into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(&path)),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn run(command: &str, argument: &str) -> String {
    match tokio::process::Command::new(command)
        .arg(argument)
        .output()
        .await
    {
        Ok(out) => {
            String::from_utf8_lossy(&out.stdout).into_owned()
                + &String::from_utf8_lossy(&out.stderr)
        }
        Err(e) => format!("{command}: {e}"),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn index(State(gate): State<Arc<Gate>>) -> Html<String> {
    let status = run(&gate.session, "status").await;
    let up = status.lines().any(|l| l.starts_with("stage: running"));
    let suite_running = tokio::fs::metadata(gate.arena.join("suite.running"))
        .await
        .is_ok();
    let results = tokio::fs::read_to_string(gate.arena.join("results.jsonl"))
        .await
        .unwrap_or_default();
    let mut rows: Vec<serde_json::Value> = results
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    rows.reverse();
    let mut cards = String::new();
    for r in rows.iter().take(40) {
        let task = r["task"].as_str().unwrap_or("");
        let run = r["run"].as_str().unwrap_or("");
        let critic = &r["critic"];
        let base = format!("/arena/{task}/{run}");
        cards.push_str(&format!(
            "<article><header><b>{}</b> · {} s · <span class=score>{:.1}</span>/100</header>\
             <img src=\"{base}/compare.png\" loading=lazy alt=\"referencia y dibujo\">\
             <p>parecido {} · realismo {} · SSIM {:.3} · ΔE {:.1} · {} decisiones · {} acciones ({} rechazadas)</p>\
             <p class=critique>{}</p><p class=lessons>{}</p>\
             <p><a href=\"{base}/trajectory.jsonl\">log</a> · <a href=\"{base}/result.json\">resultado</a> · <a href=\"{base}/first.png\">inicio</a></p></article>",
            escape(task), r["tier"], r["score"].as_f64().unwrap_or(0.0),
            critic["likeness"], critic["realism"], r["ssim"].as_f64().unwrap_or(0.0),
            r["colour_error_delta_e"].as_f64().unwrap_or(0.0), r["decisions"], r["actions_executed"], r["actions_refused"],
            escape(critic["critique"].as_str().or(critic["error"].as_str()).unwrap_or("")),
            escape(&r["lessons_learned"].as_array().map(|l| l.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" · ")).unwrap_or_default()),
        ));
    }
    Html(format!(
        "<!doctype html><html lang=es><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
         <title>Archie Arena</title><style>\
         :root{{--bg:#0d1017;--panel:#161b25;--text:#e8ebf2;--muted:#8d96a8;--accent:#5bc0ff}}\
         body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.45 system-ui,sans-serif}}\
         main{{max-width:1200px;margin:0 auto;padding:16px}}h1{{font-size:20px}}\
         .bar{{display:flex;gap:10px;flex-wrap:wrap;align-items:center}}button,a.button{{background:var(--accent);color:#04121c;border:0;border-radius:6px;padding:8px 14px;font-weight:600;cursor:pointer;text-decoration:none}}\
         .grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(340px,1fr));gap:14px;margin-top:16px}}\
         article{{background:var(--panel);border-radius:10px;padding:12px}}article img{{width:100%;border-radius:6px;background:#fff}}\
         .score{{color:var(--accent);font-weight:700}}.critique{{color:#ffd79a}}.lessons{{color:var(--muted);font-size:13px}}a{{color:var(--accent)}}pre{{color:var(--muted)}}\
         </style><main><h1>Archie Arena</h1><div class=bar>\
         <span>Sesión ArchieOS: <b>{}</b>{}</span>\
         <form method=post action=/session/up><button>Encender sesión</button></form>\
         <form method=post action=/session/down><button>Apagar sesión</button></form>\
         <form method=post action=/suite><button>Correr benchmarks</button></form>\
         {}{}</div><pre>{}</pre><div class=grid>{}</div></main></html>",
        if up { "encendida" } else { "apagada (solo entrena IRIS)" },
        if suite_running { " · <b>benchmarks corriendo</b>" } else { "" },
        if up { "<a class=button href=\"/live\">En vivo</a> <a class=button href=\"/vnc.html?autoconnect=1&resize=scale&path=websockify\">Controlar (VNC)</a>" } else { "<a class=button href=\"/live\">En vivo</a>" },
        if up { "<form method=post action=/session/stage><button>Mostrar Stage</button></form><form method=post action=/session/orb><button>Mostrar orbe</button></form>" } else { "" },
        escape(status.trim()),
        if cards.is_empty() { "<p>Todavía no hay corridas.</p>".to_string() } else { cards },
    ))
}

async fn session(State(gate): State<Arc<Gate>>, UrlPath(what): UrlPath<String>) -> Redirect {
    match what.as_str() {
        "up" | "down" => {
            let _ = run(&gate.session, &what).await;
        }
        "stage" | "orb" => {
            let _ = tokio::process::Command::new(&gate.session)
                .arg("show")
                .arg(&what)
                .output()
                .await;
        }
        _ => {}
    }
    Redirect::to("/")
}

async fn suite(State(gate): State<Arc<Gate>>) -> Redirect {
    if tokio::fs::metadata(gate.arena.join("suite.running"))
        .await
        .is_err()
    {
        let _ = tokio::process::Command::new(&gate.suite).spawn();
    }
    Redirect::to("/")
}

async fn arena_file(State(gate): State<Arc<Gate>>, UrlPath(rest): UrlPath<String>) -> Response {
    file(inside(&gate.arena, &rest)).await
}

async fn novnc_file(State(gate): State<Arc<Gate>>, UrlPath(rest): UrlPath<String>) -> Response {
    file(inside(&gate.novnc, &rest)).await
}

async fn vnc_page(State(gate): State<Arc<Gate>>) -> Response {
    file(Some(gate.novnc.join("vnc.html"))).await
}

/// noVNC's WebSocket, carried byte for byte to the session's VNC server.
async fn websockify(State(gate): State<Arc<Gate>>, upgrade: WebSocketUpgrade) -> Response {
    let vnc = gate.vnc.clone();
    upgrade
        .protocols(["binary"])
        .on_upgrade(move |socket| bridge(socket, vnc))
}

async fn bridge(socket: WebSocket, vnc: String) {
    let Ok(tcp) = tokio::net::TcpStream::connect(&vnc).await else {
        return;
    };
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    let (mut ws_write, mut ws_read) = socket.split();
    let to_vnc = async move {
        while let Some(Ok(message)) = ws_read.next().await {
            let bytes = match message {
                Message::Binary(bytes) => bytes,
                Message::Text(text) => text.into_bytes(),
                Message::Close(_) => break,
                _ => continue,
            };
            if tcp_write.write_all(&bytes).await.is_err() {
                break;
            }
        }
    };
    let to_page = async move {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match tcp_read.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ws_write
                        .send(Message::Binary(buffer[..n].to_vec()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    };
    tokio::select! { _ = to_vnc => {}, _ = to_page => {} }
}

/// The session's screen as live video: H.264 in fragmented MP4, pushed to the page.
///
/// noVNC asks for every frame and waits for it, so across an ocean it shows a few frames a
/// second. This pushes instead: `wf-recorder` captures the compositor's output and encodes
/// it, `ffmpeg` wraps it in fragments a `<video>` element plays as they arrive. One pair of
/// processes per viewer, niced, killed when the page closes.
async fn live_stream(State(gate): State<Arc<Gate>>) -> Response {
    use std::process::Stdio;
    let run = PathBuf::from(&gate.run);
    let Ok(display) = tokio::fs::read_to_string(run.join("wayland-display")).await else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no session is up").into_response();
    };
    let fps = gate.fps.to_string();
    let mut recorder = match tokio::process::Command::new("nice")
        // -D records every frame: a still screen still makes a steady stream.
        .args([
            "-n",
            "10",
            "wf-recorder",
            "-y",
            "-D",
            "-r",
            &fps,
            "-c",
            "libx264",
            "-x",
            "yuv420p",
            "-m",
            "h264",
        ])
        .args([
            "-p",
            "preset=ultrafast",
            "-p",
            "tune=zerolatency",
            "-p",
            "profile=baseline",
            "-p",
            "crf=26",
            "-p",
        ])
        .arg(format!("g={fps}"))
        .args(["-f", "/dev/stdout"])
        .env("XDG_RUNTIME_DIR", run.join("xdg"))
        .env("WAYLAND_DISPLAY", display.trim())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return (StatusCode::SERVICE_UNAVAILABLE, format!("wf-recorder: {e}")).into_response()
        }
    };
    // tokio implements TryInto<Stdio> for its pipes, not TryFrom.
    let Some(frames) = recorder
        .stdout
        .take()
        .and_then(|out| TryInto::<Stdio>::try_into(out).ok())
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut muxer = match tokio::process::Command::new("nice")
        .args([
            "-n",
            "10",
            "ffmpeg",
            "-loglevel",
            "error",
            "-fflags",
            "nobuffer",
            "-f",
            "h264",
            "-framerate",
            &fps,
            "-i",
            "pipe:0",
        ])
        .args([
            "-c",
            "copy",
            "-f",
            "mp4",
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof",
            "-frag_duration",
            "100000",
            "pipe:1",
        ])
        .stdin(frames)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, format!("ffmpeg: {e}")).into_response(),
    };
    let Some(out) = muxer.stdout.take() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    // The children live as long as the stream: when the viewer leaves, both are killed.
    let stream = futures_util::stream::unfold(
        (out, recorder, muxer),
        |(mut out, recorder, muxer)| async move {
            let mut buffer = vec![0u8; 64 * 1024];
            match out.read(&mut buffer).await {
                Ok(0) | Err(_) => None,
                Ok(n) => {
                    buffer.truncate(n);
                    Some((Ok::<_, std::io::Error>(buffer), (out, recorder, muxer)))
                }
            }
        },
    );
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("video/mp4"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The live page: the video, kept at the live edge, with the frame rate it is getting.
async fn live_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html><html lang="es"><head><meta charset="utf-8"><title>Archie en vivo</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
html,body{margin:0;height:100%;background:#111;color:#eee;font:14px system-ui,sans-serif}
video{width:100%;height:100%;object-fit:contain;display:block}
#hud{position:fixed;top:10px;right:14px;background:#0009;padding:4px 10px;border-radius:8px;font-variant-numeric:tabular-nums}
#hud b{color:#7aa7ff}
a{color:#7aa7ff}
</style></head><body>
<video id="v" autoplay muted playsinline></video>
<div id="hud"><b id="fps">…</b> fps · retraso <span id="lag">…</span> s · <a href="/">gate</a></div>
<script>
// Media Source Extensions: the stream is read as it arrives and appended fragment by
// fragment, so the video never waits for a file that has no end.
const v = document.getElementById('v');
const TYPE = 'video/mp4; codecs="avc1.42E028"';
let shown = 0, last = performance.now();
async function connect() {
  const ms = new MediaSource();
  v.src = URL.createObjectURL(ms);
  await new Promise(r => ms.addEventListener('sourceopen', r, {once: true}));
  const sb = ms.addSourceBuffer(TYPE);
  const queue = [];
  const pump = () => {
    if (sb.updating || !queue.length) return;
    // Keep ten seconds behind the playhead at most.
    if (v.currentTime > 15 && sb.buffered.length && sb.buffered.start(0) < v.currentTime - 10) {
      sb.remove(0, v.currentTime - 10);
      return;
    }
    sb.appendBuffer(queue.shift());
  };
  sb.addEventListener('updateend', pump);
  const reader = (await fetch('/live.mp4', {cache: 'no-store'})).body.getReader();
  for (;;) {
    const {value, done} = await reader.read();
    if (done) break;
    queue.push(value);
    pump();
  }
}
function restart() { setTimeout(() => connect().catch(restart).then(restart), 1500); }
connect().catch(restart).then(restart);
setInterval(() => {
  // Frames the page actually showed, not frames that arrived.
  const now = performance.now(), total = v.getVideoPlaybackQuality().totalVideoFrames;
  document.getElementById('fps').textContent = ((total - shown) * 1000 / (now - last)).toFixed(1);
  shown = total; last = now;
  if (v.buffered.length) {
    const edge = v.buffered.end(v.buffered.length - 1);
    const lag = edge - v.currentTime;
    document.getElementById('lag').textContent = lag.toFixed(2);
    // Stay live: jump to the edge when the buffer runs ahead.
    if (lag > 0.8) v.currentTime = edge - 0.15;
  }
  if (v.paused) v.play().catch(() => {});
}, 1000);
</script></body></html>"#,
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let bind = take(&mut args, "--bind", "127.0.0.1:6080");
    let gate = Arc::new(Gate {
        arena: PathBuf::from(take(&mut args, "--arena", "/workspace/arena")),
        session: take(
            &mut args,
            "--session",
            "/workspace/archieos/display/session/archie-session.sh",
        ),
        suite: take(
            &mut args,
            "--suite",
            "/workspace/archieos/display/session/arena-suite.sh",
        ),
        novnc: PathBuf::from(take(&mut args, "--novnc", "/usr/share/novnc")),
        vnc: take(&mut args, "--vnc", "127.0.0.1:5901"),
        run: take(&mut args, "--run", "/tmp/archie-session"),
        fps: take(&mut args, "--fps", "30")
            .parse()
            .unwrap_or(30)
            .clamp(24, 60),
    });
    let address: std::net::SocketAddr = bind.parse().expect("--bind is an address");
    assert!(
        address.ip().is_loopback(),
        "the gate serves loopback only; reach it through an SSH tunnel"
    );
    let app = Router::new()
        .route("/", get(index))
        .route("/session/:what", post(session))
        .route("/suite", post(suite))
        .route("/arena/*rest", get(arena_file))
        .route("/live", get(live_page))
        .route("/live.mp4", get(live_stream))
        .route("/vnc.html", get(vnc_page))
        .route("/websockify", get(websockify))
        .route("/*rest", get(novnc_file))
        .with_state(gate);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("binding the gate");
    println!("archie-arena-gate: http://{bind}");
    axum::serve(listener, app).await.expect("serving");
}
