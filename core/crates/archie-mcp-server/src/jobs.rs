//! Long jobs a bridge started (a FreeCAD sweep, a render queue): their progress, and a
//! way to stop them. The jobs run in their own processes and outlive this server.

use crate::toolbox::{Effect, ToolOutcome, ToolSpec};
use archie_bridges::{jobs, Bridge};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub const NAMES: [&str; 3] = ["archie_jobs", "archie_job_status", "archie_job_cancel"];

pub fn specs() -> Vec<ToolSpec> {
    let job = json!({
        "type": "object",
        "properties": {"bridge": {"type": "string"}, "job": {"type": "string"}},
        "required": ["bridge", "job"],
        "additionalProperties": false
    });
    vec![
        ToolSpec {
            name: "archie_jobs",
            title: "Long jobs",
            description: "The long jobs each bridge has run or is running, newest first, with their state \
                          and progress.",
            input_schema: json!({"type": "object", "properties": {"bridge": {"type": "string"}}, "additionalProperties": false}),
            grade: PrivacyGrade::Internal,
            effect: Effect::ReadOnly,
        },
        ToolSpec {
            name: "archie_job_status",
            title: "A job's progress",
            description: "One job's state (running, done, cancelled, failed), how many of its iterations are \
                          done, its latest result and any error.",
            input_schema: job.clone(),
            grade: PrivacyGrade::Internal,
            effect: Effect::ReadOnly,
        },
        ToolSpec {
            name: "archie_job_cancel",
            title: "Stop a job",
            description: "Asks a job to stop after its current iteration. What it made so far stays.",
            input_schema: job,
            grade: PrivacyGrade::Internal,
            effect: Effect::Acts,
        },
    ]
}

fn bridge<'a>(
    bridges: &'a [Arc<dyn Bridge>],
    arguments: &Map<String, Value>,
) -> Result<&'a Arc<dyn Bridge>, String> {
    let id = arguments
        .get("bridge")
        .and_then(Value::as_str)
        .ok_or("which bridge?")?;
    bridges
        .iter()
        .find(|b| b.manifest().id == id)
        .ok_or_else(|| format!("no bridge {id} is served here"))
}

pub fn call(
    bridges: &[Arc<dyn Bridge>],
    name: &str,
    arguments: &Map<String, Value>,
) -> ToolOutcome {
    let job = arguments.get("job").and_then(Value::as_str);
    match name {
        "archie_jobs" => {
            let wanted = arguments.get("bridge").and_then(Value::as_str);
            let all: Map<String, Value> = bridges
                .iter()
                .filter(|b| wanted.is_none_or(|w| w == b.manifest().id))
                .map(|b| (b.manifest().id.clone(), json!(jobs::list(b.workspace()))))
                .collect();
            ToolOutcome::json(&Value::Object(all))
        }
        "archie_job_status" | "archie_job_cancel" => {
            let bridge = match bridge(bridges, arguments) {
                Ok(bridge) => bridge,
                Err(why) => return ToolOutcome::error(why),
            };
            let Some(job) = job else {
                return ToolOutcome::error("which job? archie_jobs lists them");
            };
            if name == "archie_job_cancel" {
                if let Err(e) = jobs::cancel(bridge.workspace(), job) {
                    return ToolOutcome::error(e.to_string());
                }
            }
            match jobs::read_status(bridge.workspace(), job) {
                Ok(status) => ToolOutcome::json(&json!(status)),
                Err(e) => ToolOutcome::error(e.to_string()),
            }
        }
        _ => ToolOutcome::error("unknown tool"),
    }
}
