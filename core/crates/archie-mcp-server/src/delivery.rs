//! Delivery tools: a bridge's results sent to the person's own computer, stage by stage,
//! through the desk the workstation paired with this node (see
//! [`archie_bridges::delivery`]).

use crate::toolbox::{Effect, ToolOutcome, ToolSpec};
use archie_bridges::delivery::{Deliverer, Outbox};
use archie_bridges::Bridge;
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub const NAMES: [&str; 3] = [
    "archie_deliver",
    "archie_delivery_status",
    "archie_delivery_flush",
];

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "archie_deliver",
            title: "Deliver results to the person's computer",
            description: "Sends files a bridge made — exports, renders, documents — to the person's \
                          workstation, into ~/Archie/<project>/stages/<stage>/ and latest/, where their \
                          own software opens them (a STEP in Fusion 360, a PSD in Photoshop, a DOCX in \
                          Word). Deliver every meaningful stage of a long job, with a short note on what \
                          changed, and the last one as stage \"final\". open asks the desk to open the \
                          files, which it does only for kinds the person allowed. When the workstation \
                          is away the files wait and go with the next delivery.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "bridge": {"type": "string", "description": "The bridge whose folder the files are in, such as kicad."},
                    "files": {"type": "array", "items": {"type": "string"}, "minItems": 1,
                              "description": "Paths relative to that bridge's folder."},
                    "project": {"type": "string", "description": "The folder on the desk, such as motor-bracket."},
                    "stage": {"type": "string", "description": "Such as iter-0042 or final. Default final."},
                    "note": {"type": "string", "description": "One line on this stage: what changed, what it scored."},
                    "open": {"type": "boolean", "description": "Open the files on the workstation."}
                },
                "required": ["bridge", "files", "project"],
                "additionalProperties": false
            }),
            grade: PrivacyGrade::Internal,
            effect: Effect::Acts,
        },
        ToolSpec {
            name: "archie_delivery_status",
            title: "Is the person's computer connected?",
            description: "Whether the workstation's desk answers through its tunnel, and which files wait \
                          in each bridge's outbox.",
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            grade: PrivacyGrade::Internal,
            effect: Effect::ReadOnly,
        },
        ToolSpec {
            name: "archie_delivery_flush",
            title: "Send what waits",
            description: "Sends the files waiting in every bridge's outbox, now that the desk is back.",
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            grade: PrivacyGrade::Internal,
            effect: Effect::Acts,
        },
    ]
}

pub fn call(
    deliverer: &Deliverer,
    bridges: &[Arc<dyn Bridge>],
    name: &str,
    arguments: &Map<String, Value>,
) -> ToolOutcome {
    match name {
        "archie_deliver" => {
            let Some(id) = arguments.get("bridge").and_then(Value::as_str) else {
                return ToolOutcome::error("archie_deliver needs bridge");
            };
            let Some(bridge) = bridges.iter().find(|b| b.manifest().id == id) else {
                return ToolOutcome::error(format!("no bridge {id} is served here"));
            };
            let files: Vec<String> = match arguments.get("files").and_then(Value::as_array) {
                Some(files) if !files.is_empty() => {
                    match files
                        .iter()
                        .map(|f| f.as_str().map(str::to_string))
                        .collect()
                    {
                        Some(files) => files,
                        None => return ToolOutcome::error("files are paths, as strings"),
                    }
                }
                _ => return ToolOutcome::error("archie_deliver needs files"),
            };
            let Some(project) = arguments.get("project").and_then(Value::as_str) else {
                return ToolOutcome::error("archie_deliver needs project");
            };
            let stage = arguments
                .get("stage")
                .and_then(Value::as_str)
                .unwrap_or("final");
            let note = arguments.get("note").and_then(Value::as_str).unwrap_or("");
            let open = arguments
                .get("open")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match deliverer.deliver(bridge.workspace(), &files, project, stage, note, open) {
                Ok(report) => ToolOutcome::json(&json!(report)),
                Err(error) => ToolOutcome::error(error.to_string()),
            }
        }
        "archie_delivery_status" => {
            let desk = match deliverer.health() {
                Ok(version) => json!({"reachable": true, "version": version}),
                Err(error) => json!({"reachable": false, "why": error.to_string()}),
            };
            let waiting: Map<String, Value> = bridges
                .iter()
                .map(|bridge| {
                    let pending = Outbox::new(bridge.workspace())
                        .map(|outbox| outbox.pending())
                        .unwrap_or_default();
                    (bridge.manifest().id.clone(), json!(pending))
                })
                .collect();
            ToolOutcome::json(
                &json!({"desk": deliverer.desk().to_string(), "status": desk, "waiting": waiting}),
            )
        }
        "archie_delivery_flush" => {
            let mut sent = Map::new();
            for bridge in bridges {
                match deliverer.flush(bridge.workspace()) {
                    Ok(report) => {
                        sent.insert(bridge.manifest().id.clone(), json!(report));
                    }
                    Err(error) => return ToolOutcome::error(error.to_string()),
                }
            }
            ToolOutcome::json(&Value::Object(sent))
        }
        _ => ToolOutcome::error("unknown tool"),
    }
}
