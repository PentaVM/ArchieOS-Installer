//! A Home Assistant REST API with entities it owns: bearer authentication,
//! `/api/config`, `/api/states/<entity>` and the services the adapter calls,
//! each changing the entity and stamping `last_reported` with this server's
//! clock, as Home Assistant does.
use super::http::{HttpServer, Request, Response};
use archie_types::Timestamp;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct Entity {
    pub state: String,
    pub attributes: Value,
    pub last_reported_ms: i64,
}

#[derive(Debug)]
pub struct HaState {
    pub token: String,
    pub temperature_unit: String,
    pub entities: BTreeMap<String, Entity>,
    /// `domain/service` of every service call, in order.
    pub calls: Vec<String>,
    last_ms: i64,
}

pub struct HomeAssistantServer {
    pub base: String,
    pub state: Arc<Mutex<HaState>>,
    server: HttpServer,
}
impl HomeAssistantServer {
    pub async fn start(token: &str, entities: &[(&str, &str, Value)]) -> Self {
        let now = Timestamp::now().0;
        let state = Arc::new(Mutex::new(HaState {
            token: token.into(),
            temperature_unit: "°C".into(),
            entities: entities
                .iter()
                .map(|(id, state, attributes)| {
                    (
                        id.to_string(),
                        Entity {
                            state: state.to_string(),
                            attributes: attributes.clone(),
                            last_reported_ms: now,
                        },
                    )
                })
                .collect(),
            calls: Vec::new(),
            last_ms: now,
        }));
        let s = state.clone();
        let server = HttpServer::start(Arc::new(move |r: &Request| handle(&s, r))).await;
        Self {
            base: format!("{}/", server.base),
            state,
            server,
        }
    }
    pub fn log(&self) -> Vec<String> {
        self.server.log()
    }
}

fn rfc3339(ms: i64) -> String {
    Timestamp(ms).to_rfc3339()
}

fn state_json(id: &str, entity: &Entity) -> Value {
    json!({
        "entity_id": id,
        "state": entity.state,
        "attributes": entity.attributes,
        "last_changed": rfc3339(entity.last_reported_ms),
        "last_updated": rfc3339(entity.last_reported_ms),
        "last_reported": rfc3339(entity.last_reported_ms),
    })
}

fn handle(state: &Mutex<HaState>, request: &Request) -> Response {
    let mut ha = state.lock().unwrap();
    if request.header("authorization") != Some(&format!("Bearer {}", ha.token)) {
        return Response::new(401, "text/plain", "401: Unauthorized");
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/config") => Response::json(
            200,
            &json!({"unit_system": {"temperature": ha.temperature_unit, "length": "km"},
                    "version": "loopback"}),
        ),
        ("GET", path) if path.starts_with("/api/states/") => {
            let id = &path["/api/states/".len()..];
            match ha.entities.get(id) {
                Some(entity) => Response::json(200, &state_json(id, entity)),
                None => Response::json(404, &json!({"message": "Entity not found."})),
            }
        }
        ("POST", path) if path.starts_with("/api/services/") => {
            let Some((domain, service)) = path["/api/services/".len()..].split_once('/') else {
                return Response::empty(404);
            };
            let Ok(data) = serde_json::from_slice::<Value>(&request.body) else {
                return Response::json(400, &json!({"message": "Invalid JSON specified."}));
            };
            let Some(id) = data["entity_id"].as_str().map(str::to_owned) else {
                return Response::json(400, &json!({"message": "entity_id required"}));
            };
            ha.calls.push(format!("{domain}/{service}"));
            // Each report gets its own millisecond, as distinct reports do.
            let now = loop {
                let now = Timestamp::now().0;
                if now > ha.last_ms {
                    break now;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            };
            ha.last_ms = now;
            let Some(entity) = ha.entities.get_mut(&id) else {
                return Response::json(400, &json!({"message": "Entity not found."}));
            };
            let entity_domain = id.split('.').next().unwrap_or("");
            let changed = match (domain, service) {
                ("homeassistant", "update_entity") => true,
                (d, _) if d != entity_domain => {
                    return Response::json(400, &json!({"message": "Service not valid for entity"}))
                }
                ("switch", "turn_on") => set(entity, "on"),
                ("switch", "turn_off") => set(entity, "off"),
                ("light", "turn_off") => {
                    entity.attributes["brightness"] = Value::Null;
                    set(entity, "off")
                }
                ("light", "turn_on") => {
                    if let Some(pct) = data["brightness_pct"].as_u64() {
                        entity.attributes["brightness"] = json!((pct.min(100) * 255 + 50) / 100);
                    }
                    set(entity, "on")
                }
                ("media_player", "media_play") => set(entity, "playing"),
                ("media_player", "media_pause") => set(entity, "paused"),
                ("climate", "set_temperature") => match data["temperature"].as_f64() {
                    Some(t) if (5.0..=35.0).contains(&t) => {
                        entity.attributes["temperature"] = json!(t);
                        true
                    }
                    _ => return Response::json(400, &json!({"message": "out of range"})),
                },
                _ => return Response::json(400, &json!({"message": "Service not found."})),
            };
            if changed {
                entity.last_reported_ms = now;
            }
            let snapshot = state_json(&id, entity);
            Response::json(200, &json!([snapshot]))
        }
        _ => Response::json(404, &json!({"message": "Not found"})),
    }
}

fn set(entity: &mut Entity, state: &str) -> bool {
    entity.state = state.into();
    true
}
