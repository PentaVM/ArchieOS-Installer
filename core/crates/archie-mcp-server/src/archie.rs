//! The tools: the node's state through archied's control socket, and the Archie Stage.

use crate::toolbox::{Effect, ToolOutcome, ToolSpec, Toolbox};
use archie_core::control::{request_over, ControlRequest, ControlResponse};
use archie_core::endpoint;
use archie_policy::Principal;
use archie_stage::{KeyChord, Operator, PointerButton, StageAction, StageText};
use archie_state::StatePaths;
use archie_types::{AgentId, PrivacyGrade};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::time::Duration;

const DAEMON_TIMEOUT: Duration = Duration::from_secs(10);

/// The fields of `archie status` an IDE agent gets. An allowlist, so a field added to the
/// status later stays on the node until someone decides otherwise. Left out today: the
/// hostname, the network interfaces and the machine's configured location — what a
/// machine is called, what it is connected to and where it is are not an IDE's business.
const STATUS_FIELDS: [&str; 22] = [
    "node_id",
    "profile",
    "archieos_version",
    "os",
    "kernel",
    "architecture",
    "uptime_ms",
    "cpu_model",
    "logical_cores",
    "physical_cores",
    "memory_total_bytes",
    "memory_available_bytes",
    "accelerators",
    "storage",
    "capabilities",
    "peers",
    "reachable_peers",
    "modules_loaded",
    "modules_installed",
    "models_registered",
    "active_tasks",
    "health",
];

pub struct ArchieToolbox {
    paths: StatePaths,
    stage_socket: Option<PathBuf>,
    /// Present only when the operator started the server with stage actions allowed.
    operator: Option<Operator>,
    /// The IDE agent as the node's policy sees it: an agent of its own, never a person.
    #[cfg_attr(not(unix), allow(dead_code))]
    principal: Principal,
}

impl ArchieToolbox {
    pub fn new(
        paths: StatePaths,
        stage_socket: Option<PathBuf>,
        operator: Option<Operator>,
    ) -> Self {
        Self {
            paths,
            stage_socket,
            operator: operator.filter(|_| cfg!(unix)),
            principal: Principal::Agent {
                agent: AgentId::new(),
            },
        }
    }

    async fn daemon(&self, request: ControlRequest) -> Result<ControlResponse, String> {
        let mut stream = endpoint::connect(&self.paths)
            .await
            .map_err(|e| e.to_string())?;
        match tokio::time::timeout(DAEMON_TIMEOUT, request_over(&mut stream, &request)).await {
            Err(_) => Err(format!("archied did not answer within {DAEMON_TIMEOUT:?}")),
            Ok(Err(e)) => Err(e.to_string()),
            Ok(Ok(ControlResponse::Error { message })) => Err(message),
            Ok(Ok(response)) => Ok(response),
        }
    }

    async fn daemon_tool(&self, name: &str) -> ToolOutcome {
        let request = match name {
            "archie_ping" => ControlRequest::Ping,
            "archie_status" => ControlRequest::Status,
            "archie_modules" => ControlRequest::Modules,
            "archie_providers" => ControlRequest::Providers,
            "archie_models" => ControlRequest::Models,
            _ => return ToolOutcome::error("unknown tool"),
        };
        let response = match self.daemon(request).await {
            Ok(response) => response,
            Err(message) => return ToolOutcome::error(message),
        };
        ToolOutcome::json(&match response {
            // Whether it runs, and which version. The node id stays: it is an identifier.
            ControlResponse::Pong { version, .. } => json!({"running": true, "version": version}),
            ControlResponse::Status(status) => {
                let full = serde_json::to_value(&*status).unwrap_or(Value::Null);
                let kept: Map<String, Value> = STATUS_FIELDS
                    .iter()
                    .filter_map(|field| {
                        full.get(*field)
                            .map(|value| ((*field).to_string(), value.clone()))
                    })
                    .collect();
                Value::Object(kept)
            }
            ControlResponse::Modules {
                modules,
                load_order,
            } => {
                json!({"modules": modules, "load_order": load_order})
            }
            ControlResponse::Providers { providers, active } => {
                json!({"providers": providers, "active": active})
            }
            ControlResponse::Models { models } => json!({"models": models}),
            _ => return ToolOutcome::error("archied answered with something else"),
        })
    }

    #[cfg(unix)]
    async fn connect_stage(
        &self,
    ) -> Result<
        (
            archie_stage::StageClient<
                tokio::net::unix::OwnedReadHalf,
                tokio::net::unix::OwnedWriteHalf,
            >,
            archie_stage::StageWorld,
        ),
        String,
    > {
        let socket = self
            .stage_socket
            .as_ref()
            .ok_or("no stage socket is configured")?;
        let stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(|e| format!("the stage is not reachable at {}: {e}", socket.display()))?;
        let (read, write) = stream.into_split();
        let mut client = archie_stage::StageClient::new(read, write);
        let mut world = archie_stage::StageWorld::default();
        archie_stage::learn(&mut client, &mut world, Duration::from_millis(300))
            .await
            .map_err(|e| e.to_string())?;
        Ok((client, world))
    }

    #[cfg(unix)]
    async fn stage_surfaces(&self) -> ToolOutcome {
        match self.connect_stage().await {
            Err(message) => ToolOutcome::error(message),
            Ok((_client, world)) => ToolOutcome::json(&json!({
                "authority": world.authority(),
                "focus": world.focus(),
                "surfaces": world
                    .surfaces()
                    .map(|s| json!({
                        "surface": s.surface,
                        "app_id": s.app_id,
                        "x": s.x,
                        "y": s.y,
                        "width": s.width,
                        "height": s.height,
                    }))
                    .collect::<Vec<_>>(),
            })),
        }
    }

    #[cfg(unix)]
    async fn stage_act(&mut self, arguments: &Map<String, Value>) -> ToolOutcome {
        use archie_stage::{ActOutcome, ActRequest};

        let (app_id, surface, action) = match parse_act(arguments) {
            Ok(parsed) => parsed,
            Err(message) => return ToolOutcome::error(message),
        };
        let (mut client, mut world) = match self.connect_stage().await {
            Ok(connected) => connected,
            Err(message) => return ToolOutcome::error(message),
        };
        let request = ActRequest {
            app_id,
            surface,
            action,
            // The text came from the IDE's model, which has already seen it.
            text_grade: PrivacyGrade::Public,
            principal: self.principal.clone(),
            task_id: None,
            user_present: false,
            approved: false,
            justification: "requested by an IDE agent over MCP".into(),
        };
        let Some(operator) = self.operator.as_mut() else {
            return ToolOutcome::error("stage actions are not enabled on this server");
        };
        match operator
            .act(&mut client, &mut world, &request, Duration::from_secs(30))
            .await
        {
            Ok(outcome) if outcome.executed() => ToolOutcome::json(&json!({"outcome": "executed"})),
            Ok(ActOutcome::Receipt { sequence: 0, outcome, .. }) => ToolOutcome::error(format!(
                "the stage could not verify the command ({outcome}): this server's key is not the compositor's"
            )),
            Ok(ActOutcome::Receipt { outcome, .. }) => ToolOutcome::error(match outcome.as_str() {
                "operator_has_control" | "preempted_by_operator" => {
                    "a person holds the stage; the agent waits until they have stopped".to_string()
                }
                "stage_paused" => "the stage is paused; only a person can resume it".to_string(),
                "surface_changed" => "the window moved or changed; read stage_surfaces again".to_string(),
                other => format!("the stage did not run it: {other}"),
            }),
            Ok(ActOutcome::NeedsApproval { reason }) => ToolOutcome::error(format!(
                "this needs a person's approval at the machine ({reason}); this server cannot give it"
            )),
            Ok(ActOutcome::Refused { code, message }) => ToolOutcome::error(format!("refused ({code}): {message}")),
            Ok(ActOutcome::NoSurface) => {
                ToolOutcome::error("that application has no surface on the stage; see stage_surfaces")
            }
            Err(error) => ToolOutcome::error(error.to_string()),
        }
    }
}

fn no_arguments() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

fn act_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_id": {"type": "string", "description": "The application's id, as stage_surfaces reports it."},
            "surface": {"type": "integer", "minimum": 1, "description": "A surface id, when the application has several."},
            "action": {
                "type": "string",
                "enum": ["click", "right_click", "double_click", "move", "drag", "scroll", "type", "key", "focus"]
            },
            "x": {"type": "integer", "minimum": 0, "description": "Surface-local, in pixels."},
            "y": {"type": "integer", "minimum": 0, "description": "Surface-local, in pixels."},
            "to_x": {"type": "integer", "minimum": 0, "description": "Where a drag ends."},
            "to_y": {"type": "integer", "minimum": 0, "description": "Where a drag ends."},
            "steps": {"type": "integer", "minimum": -60, "maximum": 60, "description": "Wheel steps; positive scrolls down."},
            "text": {"type": "string", "maxLength": 2000, "description": "Plain text to type. A newline is not text: press enter with action key."},
            "key": {"type": "string", "description": "A key and its modifiers, such as enter, tab, ctrl+s or ctrl+shift+z."}
        },
        "required": ["app_id", "action"],
        "additionalProperties": false
    })
}

/// A stage action from tool arguments. Nothing about whether it may run: that is policy's.
pub fn parse_act(
    arguments: &Map<String, Value>,
) -> Result<(String, Option<u32>, StageAction), String> {
    const KNOWN: [&str; 10] = [
        "app_id", "surface", "action", "x", "y", "to_x", "to_y", "steps", "text", "key",
    ];
    if let Some(unknown) = arguments.keys().find(|key| !KNOWN.contains(&key.as_str())) {
        return Err(format!("unexpected argument {unknown:?}"));
    }
    let text = |key: &str| -> Result<String, String> {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("{key} is required, as a string"))
    };
    let int = |key: &str| -> Result<i32, String> {
        arguments
            .get(key)
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| format!("{key} is required, as an integer"))
    };
    let app_id = text("app_id")?;
    let surface = match arguments.get("surface") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or("surface is a positive integer")?,
        ),
    };
    let action = match text("action")?.as_str() {
        "click" => StageAction::Click {
            x: int("x")?,
            y: int("y")?,
            button: PointerButton::Left,
        },
        "right_click" => StageAction::Click {
            x: int("x")?,
            y: int("y")?,
            button: PointerButton::Right,
        },
        "double_click" => StageAction::DoubleClick {
            x: int("x")?,
            y: int("y")?,
            button: PointerButton::Left,
        },
        "move" => StageAction::Move {
            x: int("x")?,
            y: int("y")?,
        },
        "drag" => StageAction::Drag {
            x: int("x")?,
            y: int("y")?,
            to_x: int("to_x")?,
            to_y: int("to_y")?,
            button: PointerButton::Left,
        },
        "scroll" => StageAction::Scroll {
            x: int("x")?,
            y: int("y")?,
            steps_x: 0,
            steps_y: int("steps")?,
        },
        "type" => StageAction::Text {
            text: StageText::new(text("text")?),
        },
        "key" => StageAction::Key {
            chord: KeyChord::parse(&text("key")?).ok_or("key is a key and its modifiers, such as enter or ctrl+s")?,
        },
        "focus" => StageAction::Focus,
        _ => return Err("action is one of click, right_click, double_click, move, drag, scroll, type, key, focus".into()),
    };
    Ok((app_id, surface, action))
}

impl Toolbox for ArchieToolbox {
    fn tools(&self) -> Vec<ToolSpec> {
        let read_only = |name: &'static str,
                         title: &'static str,
                         description: &'static str,
                         grade: PrivacyGrade| ToolSpec {
            name,
            title,
            description,
            input_schema: no_arguments(),
            grade,
            effect: Effect::ReadOnly,
        };
        let mut tools = vec![
            read_only(
                "archie_ping",
                "Is Archie running?",
                "Whether the node's daemon, archied, answers, and its version.",
                PrivacyGrade::Public,
            ),
            read_only(
                "archie_status",
                "Node status",
                "The node's health, ArchieOS version, OS and kernel, CPU, memory, accelerators, storage, \
                 capabilities, peer and module counts, registered models and active tasks. Leaves out the \
                 hostname, network interfaces and the machine's location.",
                PrivacyGrade::Internal,
            ),
            read_only(
                "archie_modules",
                "Modules",
                "Installed modules, the state of each and why it is in that state, their dependencies, and \
                 the load order.",
                PrivacyGrade::Internal,
            ),
            read_only(
                "archie_providers",
                "Data providers",
                "Installed data-provider manifests: the capability each satisfies, whether it runs and why, \
                 and its documented blind spots.",
                PrivacyGrade::Internal,
            ),
            read_only(
                "archie_models",
                "Models",
                "Registered models: provider, where they run, availability, the most sensitive privacy \
                 grade each is cleared for, and context window.",
                PrivacyGrade::Internal,
            ),
        ];
        if cfg!(unix) && self.stage_socket.is_some() {
            tools.push(read_only(
                "stage_surfaces",
                "Applications on the stage",
                "The applications open on the Archie Stage, each with its surface id, position and size; who \
                 holds the stage (autonomous, operator or paused); and which surface has keyboard focus. \
                 Coordinates for stage_act are relative to a surface.",
                PrivacyGrade::Internal,
            ));
            if self.operator.is_some() {
                tools.push(ToolSpec {
                    name: "stage_act",
                    title: "Operate an application",
                    description: "Clicks, drags, scrolls, types or presses keys in an application on the Archie \
                                  Stage, with the pointer moving visibly on screen. The node's stage policy \
                                  decides which applications and actions are allowed; anything it holds for a \
                                  person's approval is refused here; and a person touching the mouse or \
                                  keyboard takes the stage back at once. Read stage_surfaces first: \
                                  coordinates are surface-local, and a window that has moved refuses the action.",
                    input_schema: act_schema(),
                    grade: PrivacyGrade::Internal,
                    effect: Effect::Acts,
                });
            }
        }
        tools
    }

    async fn call(&mut self, name: &str, arguments: Map<String, Value>) -> ToolOutcome {
        match name {
            #[cfg(unix)]
            "stage_surfaces" => self.stage_surfaces().await,
            #[cfg(unix)]
            "stage_act" => self.stage_act(&arguments).await,
            _ => {
                if !arguments.is_empty() {
                    return ToolOutcome::error("this tool takes no arguments");
                }
                self.daemon_tool(name).await
            }
        }
    }
}
