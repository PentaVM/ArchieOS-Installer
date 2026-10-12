//! A Roku TV speaking ECP over HTTP, with state: power, the running app,
//! the installed channels and the media player. Keys act on that state the
//! way a Roku TV does: with the panel off only PowerOn does anything; Home
//! closes the app; launching an installed channel wakes the TV.
use super::http::{HttpServer, Request, Response};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct TvState {
    pub power_on: bool,
    /// Running channel id; `None` is the home screen.
    pub active_app: Option<String>,
    /// `Some(true)` playing, `Some(false)` paused, `None` no player.
    pub playing: Option<bool>,
    /// Channel id → name.
    pub installed: BTreeMap<String, String>,
}

pub struct RokuTv {
    pub base: String,
    pub state: Arc<Mutex<TvState>>,
    server: HttpServer,
}
impl RokuTv {
    pub async fn start(installed: &[(&str, &str)]) -> Self {
        let state = Arc::new(Mutex::new(TvState {
            power_on: false,
            active_app: None,
            playing: None,
            installed: installed
                .iter()
                .map(|(id, name)| (id.to_string(), name.to_string()))
                .collect(),
        }));
        let s = state.clone();
        let server = HttpServer::start(Arc::new(move |r: &Request| handle(&s, r))).await;
        Self {
            base: server.base.clone(),
            state,
            server,
        }
    }
    pub fn log(&self) -> Vec<String> {
        self.server.log()
    }
}

fn xml(body: String) -> Response {
    Response::new(
        200,
        "text/xml; charset=\"utf-8\"",
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n{body}"),
    )
}

fn handle(state: &Mutex<TvState>, request: &Request) -> Response {
    let mut tv = state.lock().unwrap();
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/query/device-info") => xml(format!(
            "<device-info><udn>uuid:roku:ecp:LOOPBACK</udn>\
             <serial-number>LOOPBACK0001</serial-number><vendor-name>Roku</vendor-name>\
             <model-name>Loopback TV</model-name><is-tv>true</is-tv>\
             <power-mode>{}</power-mode></device-info>",
            if tv.power_on { "PowerOn" } else { "Headless" }
        )),
        ("GET", "/query/active-app") => match (&tv.active_app, tv.power_on) {
            (Some(id), true) => xml(format!(
                "<active-app><app id=\"{id}\" type=\"appl\" version=\"1.0\">{}</app></active-app>",
                tv.installed.get(id).cloned().unwrap_or_default()
            )),
            _ => xml("<active-app><app>Roku</app></active-app>".into()),
        },
        ("GET", "/query/media-player") => match tv.playing {
            Some(true) => xml("<player error=\"false\" state=\"play\"/>".into()),
            Some(false) => xml("<player error=\"false\" state=\"pause\"/>".into()),
            None => xml("<player error=\"false\" state=\"close\"/>".into()),
        },
        ("POST", path) if path.starts_with("/keypress/") => {
            let key = &path["/keypress/".len()..];
            if !tv.power_on && key != "PowerOn" {
                return Response::empty(200);
            }
            match key {
                "PowerOn" => tv.power_on = true,
                "PowerOff" => {
                    tv.power_on = false;
                    tv.active_app = None;
                    tv.playing = None;
                }
                "Home" => {
                    tv.active_app = None;
                    tv.playing = None;
                }
                "Play" if tv.playing.is_some() => {
                    tv.playing = Some(!tv.playing.unwrap_or(false));
                }
                "Pause" if tv.playing.is_some() => tv.playing = Some(false),
                _ => {}
            }
            Response::empty(200)
        }
        ("POST", path) if path.starts_with("/launch/") => {
            let id = &path["/launch/".len()..];
            if !tv.installed.contains_key(id) {
                return Response::empty(404);
            }
            tv.power_on = true;
            tv.active_app = Some(id.to_owned());
            tv.playing = Some(true);
            Response::empty(200)
        }
        (_, path) if path.starts_with("/query/") => Response::empty(405),
        _ => Response::empty(404),
    }
}
