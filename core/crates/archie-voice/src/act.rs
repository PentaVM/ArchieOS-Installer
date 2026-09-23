//! From a proposal to an action — or to a reason, said aloud, why not.
//!
//! Models advise; Cerebra decides; Policy authorises. This is the deciding. A tool the
//! model proposed arrives as a name and some text it says is JSON, and before anything
//! touches the stage that text is read strictly: a known tool, only the arguments that
//! tool takes, of the right kinds and within bounds, aimed at an application the stage
//! policy names and allows that kind of touch in. Whatever fails comes back as a refusal
//! with a sentence the voice can say, because a person who asked out loud deserves to
//! hear why nothing happened rather than wonder.
//!
//! Passing here is not permission. The stage's gate checks the same things again with a
//! lease in hand, at the moment of acting, against the window as it is then. This only
//! decides whether it is worth asking, and says no in words when it is not.

use archie_speech::SpeechToolIntentCandidate;
use archie_stage::{ActionKind, Key, KeyChord, StagePolicy};
use serde_json::{Map, Value};

/// What will be done, once decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Start an application, exactly as its policy entry says.
    Open { app_id: String, argv: Vec<String> },
    /// Find something by what it looks like, and press it.
    Click { app_id: String, target: String },
    Type { app_id: String, text: String },
    Keys { app_id: String, chord: KeyChord },
    Scroll { app_id: String, down: bool, steps: i32 },
}

impl Plan {
    pub fn app_id(&self) -> &str {
        match self {
            Self::Open { app_id, .. }
            | Self::Click { app_id, .. }
            | Self::Type { app_id, .. }
            | Self::Keys { app_id, .. }
            | Self::Scroll { app_id, .. } => app_id,
        }
    }
}

/// Why a proposal will not be carried out, for the log and for the voice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    /// Said to the person, in their language.
    pub spoken: String,
}

fn refuse(code: &'static str, spoken: impl Into<String>) -> Refusal {
    Refusal {
        code,
        spoken: spoken.into(),
    }
}

/// The longest description of something to click.
pub const MAX_TARGET_CHARS: usize = 200;
/// The most scroll steps one proposal may ask for.
pub const MAX_SCROLL_STEPS: i32 = 20;

/// Decides what a proposal would do, or why it will not be done.
///
/// `front` is the application in front — the one with keyboard focus — which is what
/// "type this" and "press enter" mean when nobody names an application.
pub fn decide(
    intent: &SpeechToolIntentCandidate,
    policy: &StagePolicy,
    front: Option<&str>,
) -> Result<Plan, Refusal> {
    match intent.tool_name.as_str() {
        "open_app" => {
            let args = arguments(&intent.raw_arguments, &["app"])?;
            let name = text(&args, "app", 128)?;
            let app = policy.named(name).ok_or_else(|| {
                refuse(
                    "unknown_app",
                    format!("No conozco una aplicación que se llame {name}."),
                )
            })?;
            if app.launch.is_empty() {
                return Err(refuse(
                    "not_openable",
                    format!("No tengo permiso para abrir {}.", app.display_name),
                ));
            }
            Ok(Plan::Open {
                app_id: app.app_id.clone(),
                argv: app.launch.clone(),
            })
        }
        "click" => {
            let args = arguments(&intent.raw_arguments, &["target", "app"])?;
            let target = text(&args, "target", MAX_TARGET_CHARS)?.to_string();
            let app_id = match args.get("app") {
                Some(_) => {
                    let name = text(&args, "app", 128)?;
                    policy
                        .named(name)
                        .map(|app| app.app_id.clone())
                        .ok_or_else(|| {
                            refuse(
                                "unknown_app",
                                format!("No conozco una aplicación que se llame {name}."),
                            )
                        })?
                }
                None => front_app(front)?,
            };
            permitted(policy, &app_id, ActionKind::Click, "hacer clic")?;
            Ok(Plan::Click { app_id, target })
        }
        "type_text" => {
            let args = arguments(&intent.raw_arguments, &["text"])?;
            let words = text(&args, "text", usize::MAX)?.to_string();
            let app_id = front_app(front)?;
            let app = permitted(policy, &app_id, ActionKind::Text, "escribir")?;
            if words.chars().count() > app.max_text_chars {
                return Err(refuse(
                    "text_too_long",
                    format!(
                        "Eso es más largo de lo que puedo escribir en {}.",
                        app.display_name
                    ),
                ));
            }
            Ok(Plan::Type {
                app_id,
                text: words,
            })
        }
        "press_keys" => {
            let args = arguments(&intent.raw_arguments, &["keys"])?;
            let spec = text(&args, "keys", 64)?;
            let chord = chord_of(spec).ok_or_else(|| {
                refuse("unknown_keys", format!("No sé qué teclas son «{spec}»."))
            })?;
            let app_id = front_app(front)?;
            let app = permitted(policy, &app_id, ActionKind::Key, "usar el teclado")?;
            if !chord.modifiers.is_empty() && !app.allow_modified_keys {
                return Err(refuse(
                    "modified_keys_not_allowed",
                    format!(
                        "No tengo permiso para usar combinaciones de teclas en {}.",
                        app.display_name
                    ),
                ));
            }
            if let Key::Named(named) = &chord.key {
                if app.approval_keys.contains(named) {
                    return Err(refuse(
                        "needs_approval",
                        "Esa tecla necesita que me lo apruebes tú.",
                    ));
                }
            }
            Ok(Plan::Keys { app_id, chord })
        }
        "scroll" => {
            let args = arguments(&intent.raw_arguments, &["direction", "amount"])?;
            let down = match text(&args, "direction", 8)? {
                "down" | "abajo" => true,
                "up" | "arriba" => false,
                other => {
                    return Err(refuse(
                        "bad_arguments",
                        format!("No sé desplazarme hacia «{other}»."),
                    ))
                }
            };
            let steps = match args.get("amount") {
                None => 3,
                Some(value) => value
                    .as_i64()
                    .filter(|steps| (1..=i64::from(MAX_SCROLL_STEPS)).contains(steps))
                    .map(|steps| steps as i32)
                    .ok_or_else(|| {
                        refuse("bad_arguments", "No entendí cuánto desplazarme.")
                    })?,
            };
            let app_id = front_app(front)?;
            permitted(policy, &app_id, ActionKind::Scroll, "desplazar la vista")?;
            Ok(Plan::Scroll {
                app_id,
                down,
                steps,
            })
        }
        _ => Err(refuse("unknown_tool", "Eso todavía no sé hacerlo.")),
    }
}

/// The proposal's arguments as an object holding only the keys this tool takes.
fn arguments(raw: &str, known: &[&str]) -> Result<Map<String, Value>, Refusal> {
    let value: Value = serde_json::from_str(raw.trim())
        .map_err(|_| refuse("bad_arguments", "No entendí bien qué hacer."))?;
    let Value::Object(object) = value else {
        return Err(refuse("bad_arguments", "No entendí bien qué hacer."));
    };
    if let Some(stray) = object.keys().find(|key| !known.contains(&key.as_str())) {
        return Err(refuse(
            "bad_arguments",
            format!("No entendí bien qué hacer: sobra «{stray}»."),
        ));
    }
    Ok(object)
}

fn text<'a>(args: &'a Map<String, Value>, key: &str, most: usize) -> Result<&'a str, Refusal> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| refuse("bad_arguments", "Me faltó un dato para hacerlo."))?;
    if value.chars().count() > most {
        return Err(refuse("bad_arguments", "Eso es demasiado largo."));
    }
    Ok(value)
}

fn front_app(front: Option<&str>) -> Result<String, Refusal> {
    front
        .map(str::to_string)
        .ok_or_else(|| refuse("nothing_in_front", "No hay ninguna aplicación al frente para eso."))
}

fn permitted<'a>(
    policy: &'a StagePolicy,
    app_id: &str,
    kind: ActionKind,
    doing: &str,
) -> Result<&'a archie_stage::AppPolicy, Refusal> {
    let app = policy.app(app_id).ok_or_else(|| {
        refuse(
            "app_not_allowed",
            "No tengo permiso para usar la aplicación que está al frente.",
        )
    })?;
    if !app.allow.contains(&kind) {
        return Err(refuse(
            "action_not_allowed",
            format!("No tengo permiso para {doing} en {}.", app.display_name),
        ));
    }
    if app.requires_approval.contains(&kind) {
        return Err(refuse(
            "needs_approval",
            format!("Para {doing} en {} necesito que me lo apruebes tú.", app.display_name),
        ));
    }
    Ok(app)
}

/// Keys as a model tends to write them — "Ctrl + S", "Enter", "control+shift+z" — read
/// as the stage writes them.
fn chord_of(spec: &str) -> Option<KeyChord> {
    let normalised: String = spec
        .to_lowercase()
        .split('+')
        .map(|part| match part.trim() {
            "control" => "ctrl",
            "return" | "intro" => "enter",
            "esc" => "escape",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+");
    KeyChord::parse(&normalised)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: &str = r#"
[[app]]
app_id = "org.mozilla.firefox"
display_name = "Firefox"
allow = ["click", "text", "key", "scroll", "focus"]
max_text_chars = 64
allow_modified_keys = true
approval_keys = ["enter"]
launch = ["firefox", "--new-window"]

[[app]]
app_id = "org.qt-project.qml"
display_name = "Lienzo"
allow = ["stroke", "click"]
"#;

    fn policy() -> StagePolicy {
        StagePolicy::parse(POLICY).unwrap()
    }

    fn intent(tool: &str, arguments: &str) -> SpeechToolIntentCandidate {
        SpeechToolIntentCandidate {
            correlation_id: "t1#0".into(),
            tool_name: tool.into(),
            raw_arguments: arguments.into(),
            source_adapter: "nemotron:test".into(),
            confidence: None,
        }
    }

    const FOX: Option<&str> = Some("org.mozilla.firefox");

    #[test]
    fn open_runs_exactly_what_the_policy_wrote_and_nothing_the_model_said() {
        let plan = decide(&intent("open_app", r#"{"app":"Firefox"}"#), &policy(), None).unwrap();
        assert_eq!(
            plan,
            Plan::Open {
                app_id: "org.mozilla.firefox".into(),
                argv: vec!["firefox".into(), "--new-window".into()],
            }
        );
        // The model cannot smuggle a command in as the name, or as an extra argument.
        let smuggled = decide(
            &intent("open_app", r#"{"app":"firefox; rm -rf ~"}"#),
            &policy(),
            None,
        );
        assert_eq!(smuggled.unwrap_err().code, "unknown_app");
        let extra = decide(
            &intent("open_app", r#"{"app":"firefox","args":"--kiosk evil.com"}"#),
            &policy(),
            None,
        );
        assert_eq!(extra.unwrap_err().code, "bad_arguments");
    }

    #[test]
    fn an_application_without_a_launch_entry_is_not_opened_and_the_person_hears_why() {
        let refused = decide(&intent("open_app", r#"{"app":"lienzo"}"#), &policy(), None)
            .unwrap_err();
        assert_eq!(refused.code, "not_openable");
        assert!(refused.spoken.contains("Lienzo"), "{}", refused.spoken);
        let unknown = decide(&intent("open_app", r#"{"app":"chrome"}"#), &policy(), None)
            .unwrap_err();
        assert!(unknown.spoken.contains("chrome"), "{}", unknown.spoken);
    }

    #[test]
    fn typing_goes_to_the_application_in_front_within_its_bounds() {
        let plan = decide(&intent("type_text", r#"{"text":"clima santiago"}"#), &policy(), FOX)
            .unwrap();
        assert_eq!(plan.app_id(), "org.mozilla.firefox");
        let long = format!(r#"{{"text":"{}"}}"#, "a".repeat(65));
        assert_eq!(
            decide(&intent("type_text", &long), &policy(), FOX)
                .unwrap_err()
                .code,
            "text_too_long"
        );
        assert_eq!(
            decide(&intent("type_text", r#"{"text":"hola"}"#), &policy(), None)
                .unwrap_err()
                .code,
            "nothing_in_front"
        );
        // An application the policy lets Archie draw in, but not type in.
        assert_eq!(
            decide(
                &intent("type_text", r#"{"text":"hola"}"#),
                &policy(),
                Some("org.qt-project.qml")
            )
            .unwrap_err()
            .code,
            "action_not_allowed"
        );
        assert_eq!(
            decide(
                &intent("type_text", r#"{"text":"hola"}"#),
                &policy(),
                Some("org.unknown.app")
            )
            .unwrap_err()
            .code,
            "app_not_allowed"
        );
    }

    #[test]
    fn keys_are_read_the_way_a_model_writes_them_and_approval_keys_wait_for_a_person() {
        let plan = decide(&intent("press_keys", r#"{"keys":"Ctrl + S"}"#), &policy(), FOX)
            .unwrap();
        let Plan::Keys { chord, .. } = plan else {
            panic!("a key plan");
        };
        assert_eq!(Some(chord), KeyChord::parse("ctrl+s"));
        assert_eq!(
            decide(&intent("press_keys", r#"{"keys":"Enter"}"#), &policy(), FOX)
                .unwrap_err()
                .code,
            "needs_approval"
        );
        assert_eq!(
            decide(&intent("press_keys", r#"{"keys":"hyper+q"}"#), &policy(), FOX)
                .unwrap_err()
                .code,
            "unknown_keys"
        );
    }

    #[test]
    fn scroll_is_bounded_and_speaks_either_language() {
        assert_eq!(
            decide(&intent("scroll", r#"{"direction":"abajo"}"#), &policy(), FOX).unwrap(),
            Plan::Scroll {
                app_id: "org.mozilla.firefox".into(),
                down: true,
                steps: 3
            }
        );
        assert_eq!(
            decide(
                &intent("scroll", r#"{"direction":"down","amount":500}"#),
                &policy(),
                FOX
            )
            .unwrap_err()
            .code,
            "bad_arguments"
        );
    }

    #[test]
    fn a_click_can_name_its_application_or_mean_the_one_in_front() {
        let named = decide(
            &intent("click", r#"{"target":"el botón Buscar","app":"firefox"}"#),
            &policy(),
            None,
        )
        .unwrap();
        assert_eq!(named.app_id(), "org.mozilla.firefox");
        let in_front = decide(
            &intent("click", r#"{"target":"el color rojo"}"#),
            &policy(),
            Some("org.qt-project.qml"),
        )
        .unwrap();
        assert_eq!(in_front.app_id(), "org.qt-project.qml");
        let long = format!(r#"{{"target":"{}"}}"#, "x".repeat(MAX_TARGET_CHARS + 1));
        assert!(decide(&intent("click", &long), &policy(), FOX).is_err());
    }

    #[test]
    fn nonsense_is_refused_in_words_and_never_panics() {
        for raw in ["", "null", "[]", "{", r#"{"app": 7}"#, r#"{"app": ""}"#] {
            let refused = decide(&intent("open_app", raw), &policy(), FOX).unwrap_err();
            assert!(!refused.spoken.is_empty(), "{raw:?} gets a sentence");
        }
        assert_eq!(
            decide(&intent("format_disk", "{}"), &policy(), FOX)
                .unwrap_err()
                .code,
            "unknown_tool"
        );
    }
}
