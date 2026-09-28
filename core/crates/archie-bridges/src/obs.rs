//! OBS Studio: scenes, overlays, recording and screenshots through obs-websocket 5, the
//! JSON API built into OBS since version 28.
//!
//! The bridge connects only to OBS on this machine's loopback. It never starts a stream:
//! going live publishes in the person's name, so that stays a click of theirs.

use crate::manifest::Manifest;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;
use tungstenite::{Message, WebSocket};

pub const MANIFEST: &str = include_str!("../../../../bridges/obs/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../bridges/obs/SKILL.md");

const DEFAULT_URL: &str = "ws://127.0.0.1:4455";
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_IMAGE: usize = 700 * 1024;
const IMAGE_KINDS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

pub struct Obs {
    manifest: Manifest,
    workspace: Workspace,
    url: String,
    password_file: Option<PathBuf>,
}

/// obs-websocket's authentication string for `password`, `salt` and `challenge`.
pub fn authentication(password: &str, salt: &str, challenge: &str) -> String {
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let secret = b64(ring::digest::digest(
        &ring::digest::SHA256,
        format!("{password}{salt}").as_bytes(),
    )
    .as_ref());
    b64(ring::digest::digest(
        &ring::digest::SHA256,
        format!("{secret}{challenge}").as_bytes(),
    )
    .as_ref())
}

struct Session {
    socket: WebSocket<TcpStream>,
    next: u64,
}

impl Session {
    fn read(&mut self) -> Result<Value, String> {
        loop {
            match self.socket.read().map_err(|e| format!("OBS: {e}"))? {
                Message::Text(text) => {
                    return serde_json::from_str(text.as_str()).map_err(|e| e.to_string())
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                Message::Binary(_) => continue,
                Message::Close(frame) => {
                    return Err(format!(
                        "OBS closed the connection{}",
                        frame.map(|f| format!(": {}", f.reason)).unwrap_or_default()
                    ))
                }
            }
        }
    }

    fn request(&mut self, kind: &str, data: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next.to_string();
        let message =
            json!({"op": 6, "d": {"requestType": kind, "requestId": id, "requestData": data}});
        self.socket
            .send(Message::text(message.to_string()))
            .map_err(|e| format!("OBS: {e}"))?;
        loop {
            let reply = self.read()?;
            if reply["op"] == 7 && reply["d"]["requestId"] == json!(id) {
                let status = &reply["d"]["requestStatus"];
                if status["result"] == json!(true) {
                    return Ok(reply["d"]["responseData"].clone());
                }
                return Err(format!(
                    "OBS refused {kind}: {}",
                    status["comment"].as_str().unwrap_or("no reason given")
                ));
            }
        }
    }
}

impl Obs {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        let url = programs.get("obs-websocket").display().to_string();
        let password = programs.get("obs-password");
        Self {
            manifest,
            workspace,
            url: if url == "obs-websocket" {
                DEFAULT_URL.into()
            } else {
                url
            },
            password_file: (password.as_os_str() != "obs-password").then_some(password),
        }
    }

    fn connect(&self) -> Result<Session, String> {
        let rest = self
            .url
            .strip_prefix("ws://")
            .ok_or("the OBS address is ws://127.0.0.1:<port>")?;
        let host = rest.split('/').next().unwrap_or("");
        let address: std::net::SocketAddr = host
            .replace("localhost", "127.0.0.1")
            .parse()
            .map_err(|_| format!("{host} is not an address"))?;
        if !address.ip().is_loopback() {
            return Err("this bridge talks only to OBS on this machine's loopback".into());
        }
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).map_err(|e| {
            format!("OBS is not answering at {address}: {e}. Is OBS open, with Tools > WebSocket Server Settings enabled?")
        })?;
        stream.set_read_timeout(Some(TIMEOUT)).ok();
        stream.set_write_timeout(Some(TIMEOUT)).ok();
        let (socket, _) =
            tungstenite::client(self.url.as_str(), stream).map_err(|e| format!("OBS: {e}"))?;
        let mut session = Session { socket, next: 0 };
        let hello = session.read()?;
        if hello["op"] != 0 {
            return Err("OBS did not say hello; is this obs-websocket 5?".into());
        }
        let mut identify = json!({"rpcVersion": 1, "eventSubscriptions": 0});
        if let Some(auth) = hello["d"]["authentication"].as_object() {
            let file = self.password_file.as_ref().ok_or(
                "OBS asks for a password: give the server --program obs-password=<file holding it>",
            )?;
            let password =
                std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            identify["authentication"] = json!(authentication(
                password.trim_end_matches(['\n', '\r']),
                auth["salt"].as_str().unwrap_or(""),
                auth["challenge"].as_str().unwrap_or(""),
            ));
        }
        session
            .socket
            .send(Message::text(json!({"op": 1, "d": identify}).to_string()))
            .map_err(|e| format!("OBS: {e}"))?;
        let identified = session.read()?;
        if identified["op"] != 2 {
            return Err("OBS did not accept the connection; check the password".into());
        }
        Ok(session)
    }

    fn with_session(&self, work: impl FnOnce(&mut Session) -> Result<Value, String>) -> Outcome {
        match self.connect().and_then(|mut session| {
            let result = work(&mut session);
            let _ = session.socket.close(None);
            result
        }) {
            Ok(value) => Outcome::json(&value),
            Err(why) => Outcome::error(why),
        }
    }

    fn text_arg<'a>(arguments: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| format!("{key} is a name"))
    }

    fn image_file(&self, name: &str) -> Result<PathBuf, String> {
        let path = self.workspace.file(name).map_err(|e| e.to_string())?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if IMAGE_KINDS.contains(&ext.as_str()) {
            Ok(path)
        } else {
            Err(format!("{name} is not an image OBS shows: {IMAGE_KINDS:?}"))
        }
    }

    fn status(&self) -> Outcome {
        self.with_session(|s| {
            let version = s.request("GetVersion", json!({}))?;
            let record = s.request("GetRecordStatus", json!({}))?;
            let stream = s.request("GetStreamStatus", json!({}))?;
            let scene = s.request("GetCurrentProgramScene", json!({}))?;
            let stats = s.request("GetStats", json!({}))?;
            Ok(json!({
                "obs": version["obsVersion"], "websocket": version["obsWebSocketVersion"],
                "scene": scene["currentProgramSceneName"],
                "recording": {"active": record["outputActive"], "paused": record["outputPaused"], "timecode": record["outputTimecode"]},
                "streaming": {"active": stream["outputActive"], "timecode": stream["outputTimecode"]},
                "fps": stats["activeFps"], "cpu_percent": stats["cpuUsage"],
                "skipped_frames": stats["renderSkippedFrames"], "dropped_frames": stats["outputSkippedFrames"],
            }))
        })
    }

    fn scenes(&self) -> Outcome {
        self.with_session(|s| {
            let list = s.request("GetSceneList", json!({}))?;
            let mut scenes = Vec::new();
            for scene in list["scenes"].as_array().into_iter().flatten().rev() {
                let name = scene["sceneName"].as_str().unwrap_or("").to_string();
                let items = s.request("GetSceneItemList", json!({"sceneName": name}))?;
                let items: Vec<Value> = items["sceneItems"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|i| json!({"source": i["sourceName"], "kind": i["inputKind"], "visible": i["sceneItemEnabled"], "id": i["sceneItemId"]}))
                    .collect();
                scenes.push(json!({"scene": name, "items": items}));
            }
            Ok(json!({"current": list["currentProgramSceneName"], "scenes": scenes}))
        })
    }

    fn call_tool(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "obs_switch_scene" => {
                let scene = match Self::text_arg(arguments, "scene") {
                    Ok(scene) => scene.to_string(),
                    Err(why) => return Outcome::error(why),
                };
                self.with_session(|s| {
                    s.request("SetCurrentProgramScene", json!({"sceneName": scene}))?;
                    Ok(json!({"scene": scene}))
                })
            }
            "obs_show" => {
                let (scene, source) = match (
                    Self::text_arg(arguments, "scene"),
                    Self::text_arg(arguments, "source"),
                ) {
                    (Ok(a), Ok(b)) => (a.to_string(), b.to_string()),
                    (Err(why), _) | (_, Err(why)) => return Outcome::error(why),
                };
                let visible = arguments
                    .get("visible")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                self.with_session(|s| {
                    let id = s.request(
                        "GetSceneItemId",
                        json!({"sceneName": scene, "sourceName": source}),
                    )?["sceneItemId"]
                        .clone();
                    s.request(
                        "SetSceneItemEnabled",
                        json!({"sceneName": scene, "sceneItemId": id, "sceneItemEnabled": visible}),
                    )?;
                    Ok(json!({"scene": scene, "source": source, "visible": visible}))
                })
            }
            "obs_set_text" => {
                let input = match Self::text_arg(arguments, "input") {
                    Ok(input) => input.to_string(),
                    Err(why) => return Outcome::error(why),
                };
                let Some(text) = arguments
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|t| t.len() <= 4000)
                else {
                    return Outcome::error("text is up to 4000 characters");
                };
                let text = text.to_string();
                self.with_session(|s| {
                    s.request("SetInputSettings", json!({"inputName": input, "inputSettings": {"text": text}, "overlay": true}))?;
                    Ok(json!({"input": input, "text": text}))
                })
            }
            "obs_set_image" => {
                let input = match Self::text_arg(arguments, "input") {
                    Ok(input) => input.to_string(),
                    Err(why) => return Outcome::error(why),
                };
                let path = match arguments
                    .get("file")
                    .and_then(Value::as_str)
                    .ok_or("file is an image in the workspace".to_string())
                    .and_then(|f| self.image_file(f))
                {
                    Ok(path) => path,
                    Err(why) => return Outcome::error(why),
                };
                self.with_session(|s| {
                    s.request("SetInputSettings", json!({"inputName": input, "inputSettings": {"file": path}, "overlay": true}))?;
                    Ok(json!({"input": input, "file": self.workspace.relative(&path)}))
                })
            }
            "obs_add_overlay" => self.add_overlay(arguments),
            "obs_record" => self.record(arguments),
            "obs_screenshot" => self.screenshot(arguments),
            _ => Outcome::error("unknown tool"),
        }
    }

    fn add_overlay(&self, arguments: &Map<String, Value>) -> Outcome {
        let (scene, name) = match (
            Self::text_arg(arguments, "scene"),
            Self::text_arg(arguments, "name"),
        ) {
            (Ok(a), Ok(b)) => (a.to_string(), b.to_string()),
            (Err(why), _) | (_, Err(why)) => return Outcome::error(why),
        };
        let x = arguments
            .get("x")
            .and_then(Value::as_f64)
            .unwrap_or(40.0)
            .clamp(-10000.0, 10000.0);
        let y = arguments
            .get("y")
            .and_then(Value::as_f64)
            .unwrap_or(40.0)
            .clamp(-10000.0, 10000.0);
        let settings = match (
            arguments.get("text").and_then(Value::as_str),
            arguments.get("file").and_then(Value::as_str),
        ) {
            (Some(text), None) if text.len() <= 4000 => {
                let size = arguments
                    .get("size")
                    .and_then(Value::as_u64)
                    .unwrap_or(48)
                    .clamp(8, 400);
                let colour = arguments
                    .get("color")
                    .and_then(Value::as_str)
                    .unwrap_or("#ffffff");
                let rgb = u32::from_str_radix(colour.trim_start_matches('#'), 16)
                    .ok()
                    .filter(|_| colour.len() == 7);
                let Some(rgb) = rgb else {
                    return Outcome::error("color is #rrggbb");
                };
                // OBS stores colour as ABGR.
                let abgr =
                    0xff00_0000u32 | ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff);
                (
                    "text",
                    json!({"text": text, "font": {"face": "Sans Serif", "size": size, "style": "Bold"}, "color": abgr, "color1": abgr, "color2": abgr, "outline": true, "drop_shadow": true}),
                )
            }
            (None, Some(file)) => match self.image_file(file) {
                Ok(path) => ("image", json!({"file": path})),
                Err(why) => return Outcome::error(why),
            },
            _ => return Outcome::error("an overlay has text or file, not both"),
        };
        self.with_session(|s| {
            let kind = if settings.0 == "image" {
                "image_source".to_string()
            } else {
                // The text source is named differently on each platform.
                let kinds = s.request("GetInputKindList", json!({"unversioned": false}))?;
                let kinds: Vec<&str> = kinds["inputKinds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                ["text_ft2_source_v2", "text_gdiplus_v3", "text_gdiplus_v2", "text_ft2_source"]
                    .into_iter()
                    .find(|k| kinds.contains(k))
                    .ok_or("this OBS has no text source")?
                    .to_string()
            };
            let created = s.request(
                "CreateInput",
                json!({"sceneName": scene, "inputName": name, "inputKind": kind, "inputSettings": settings.1, "sceneItemEnabled": true}),
            )?;
            s.request(
                "SetSceneItemTransform",
                json!({"sceneName": scene, "sceneItemId": created["sceneItemId"], "sceneItemTransform": {"positionX": x, "positionY": y}}),
            )?;
            Ok(json!({"scene": scene, "overlay": name, "kind": kind, "at": [x, y]}))
        })
    }

    fn record(&self, arguments: &Map<String, Value>) -> Outcome {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("");
        let recordings = match self
            .workspace
            .scratch("obs")
            .map(|_| self.workspace.root().join("recordings"))
        {
            Ok(dir) => dir,
            Err(e) => return Outcome::error(e.to_string()),
        };
        self.with_session(|s| match action {
            "start" => {
                std::fs::create_dir_all(&recordings).map_err(|e| e.to_string())?;
                s.request("SetRecordDirectory", json!({"recordDirectory": recordings}))?;
                s.request("StartRecord", json!({}))?;
                Ok(json!({"recording": true, "into": "recordings/"}))
            }
            "stop" => {
                let stopped = s.request("StopRecord", json!({}))?;
                let path = stopped["outputPath"]
                    .as_str()
                    .map(|p| self.workspace.relative(std::path::Path::new(p)));
                Ok(json!({"recording": false, "file": path}))
            }
            "pause" => s
                .request("PauseRecord", json!({}))
                .map(|_| json!({"paused": true})),
            "resume" => s
                .request("ResumeRecord", json!({}))
                .map(|_| json!({"paused": false})),
            _ => Err("action is start, stop, pause or resume".into()),
        })
    }

    fn screenshot(&self, arguments: &Map<String, Value>) -> Outcome {
        let png = match self.workspace.fresh_file("renders", "obs", "png") {
            Ok(png) => png,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let source = arguments
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_string);
        let result = self.connect().and_then(|mut s| {
            let source = match source {
                Some(source) => source,
                None => s.request("GetCurrentProgramScene", json!({}))?["currentProgramSceneName"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            };
            let shot = s.request(
                "GetSourceScreenshot",
                json!({"sourceName": source, "imageFormat": "png", "imageWidth": 1280}),
            )?;
            let _ = s.socket.close(None);
            let data = shot["imageData"].as_str().unwrap_or("");
            let encoded = data.split_once(',').map(|(_, b)| b).unwrap_or(data);
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| "OBS sent a picture that does not decode".to_string())
                .map(|bytes| (source, bytes))
        });
        match result {
            Ok((source, bytes)) => {
                if let Err(e) = std::fs::write(&png, &bytes) {
                    return Outcome::error(e.to_string());
                }
                let outcome = Outcome::json(
                    &json!({"image": self.workspace.relative(&png), "source": source}),
                );
                if bytes.len() <= MAX_IMAGE {
                    outcome.with_image(Image {
                        mime: "image/png",
                        data: bytes,
                    })
                } else {
                    outcome
                }
            }
            Err(why) => {
                let _ = std::fs::remove_file(&png);
                Outcome::error(why)
            }
        }
    }
}

impl Bridge for Obs {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    fn guide(&self) -> &'static str {
        GUIDE
    }

    fn tools(&self) -> Vec<BridgeTool> {
        let name = json!({"type": "string"});
        let tool = |name: &'static str,
                    title: &'static str,
                    description: &'static str,
                    schema: Value,
                    effect: Effect| BridgeTool {
            name,
            title,
            description,
            input_schema: schema,
            grade: PrivacyGrade::Internal,
            effect,
        };
        let none = json!({"type": "object", "properties": {}, "additionalProperties": false});
        vec![
            tool("obs_status", "Is OBS recording?", "OBS's version, the current scene, whether it records or streams, fps, CPU and dropped frames.", none.clone(), Effect::ReadOnly),
            tool("obs_scenes", "Scenes and sources", "Every scene with its sources: name, kind and whether each is visible.", none, Effect::ReadOnly),
            tool("obs_switch_scene", "Switch scene", "Makes a scene the one on air.",
                 json!({"type": "object", "properties": {"scene": name}, "required": ["scene"], "additionalProperties": false}), Effect::Writes),
            tool("obs_show", "Show or hide a source", "Shows or hides a source (an overlay, a camera) in a scene.",
                 json!({"type": "object", "properties": {"scene": name, "source": name, "visible": {"type": "boolean"}}, "required": ["scene", "source"], "additionalProperties": false}), Effect::Writes),
            tool("obs_set_text", "Change an overlay's text", "Sets the text of a text source: a lower third, a score, a caption.",
                 json!({"type": "object", "properties": {"input": name, "text": {"type": "string"}}, "required": ["input", "text"], "additionalProperties": false}), Effect::Writes),
            tool("obs_set_image", "Change an overlay's picture", "Points an image source at a picture in the workspace.",
                 json!({"type": "object", "properties": {"input": name, "file": {"type": "string"}}, "required": ["input", "file"], "additionalProperties": false}), Effect::Writes),
            tool("obs_add_overlay", "Add an overlay", "Adds a new text overlay (bold, outlined, with shadow; size, color #rrggbb) or picture overlay to a scene at x, y pixels.",
                 json!({"type": "object", "properties": {"scene": name, "name": name, "text": {"type": "string"}, "file": {"type": "string"},
                        "x": {"type": "number"}, "y": {"type": "number"}, "size": {"type": "integer"}, "color": {"type": "string"}},
                        "required": ["scene", "name"], "additionalProperties": false}), Effect::Writes),
            tool("obs_record", "Record", "start records into the workspace's recordings/ folder; stop returns the file; pause and resume.",
                 json!({"type": "object", "properties": {"action": {"type": "string", "enum": ["start", "stop", "pause", "resume"]}}, "required": ["action"], "additionalProperties": false}), Effect::Writes),
            tool("obs_screenshot", "What is on air", "A picture of the current scene, or of one source, returned as an image.",
                 json!({"type": "object", "properties": {"source": name}, "additionalProperties": false}), Effect::Writes),
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "obs_status" => self.status(),
            "obs_scenes" => self.scenes(),
            other => self.call_tool(other, arguments),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_matches_the_protocol_example() {
        // The worked example in obs-websocket's protocol documentation.
        assert_eq!(
            authentication(
                "supersecretpassword",
                "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=",
                "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="
            ),
            "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
        );
    }

    #[test]
    fn only_loopback_obs_is_reached() {
        let dir = std::env::temp_dir();
        let mut programs = Programs::default();
        programs.set("obs-websocket", "ws://10.0.0.8:4455");
        let obs = Obs::new(
            Manifest::parse(MANIFEST).unwrap(),
            Workspace::open(&dir).unwrap(),
            &programs,
        );
        assert!(obs.connect().err().unwrap().contains("loopback"));
    }
}
