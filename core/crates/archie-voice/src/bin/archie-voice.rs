//! `archie-voice` — a turn of conversation, typed instead of spoken, against the real model.
//!
//! ```text
//! archie-voice ask "<what the person said>" --grade public
//!              [--model nvidia/Nemotron-3_5-Lightning] [--budget full|sentence]
//!              [--language es-CL]
//!
//! archie-voice do  "<what the person said>" --grade public
//!              --socket <stage control socket> --key-file <stage key> --policy <apps.toml>
//!              [--stage-display <wayland display>] [--app-log <file>] [--steps 6]
//!              [--ground-model <vision model> [--ground-endpoint <local NIM url>]]
//!              [--model …] [--budget …] [--language …]
//! ```
//!
//! `ask` is the cognitive half on its own: what was heard goes in, and out come the pieces
//! a voice would say and the tools the model proposes, each stamped with how long after the
//! question it arrived. The time to the first piece is the silence a person would hear
//! after they stop talking, so it is the number this tool exists to show.
//!
//! `do` goes on to act. Every tool the model proposes is decided on — read strictly, and
//! checked against the stage policy — and then either carried out through the stage gate
//! or refused. What came of it goes back to the model, which takes the next step or says
//! what it has to say, for at most `--steps` steps. A click needs a grounding model to find
//! what was named; without `--ground-model` clicks are refused and the model is told so.
//!
//! `--grade` has no default. What a person says to their computer is theirs, and a cloud
//! model whose retention nobody has verified may only be given what they graded public; the
//! model client refuses anything else before it leaves the machine. The credential comes
//! from `NEBIUS_TOKEN_FACTORY_KEY` and is never printed.

use archie_model_runtime::{InferenceClient, ProviderEndpoint};
use archie_speech::{CognitiveEvent, CognitiveModelAdapter, CognitiveRequest, SpeechBudget};
use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost,
};
use archie_voice::{tools, NebiusSource, NemotronCognitive};
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    // Read before any thread exists: the local offset cannot be trusted once one does.
    let offset = time::UtcOffset::current_local_offset().ok();
    match run(std::env::args().skip(1).collect(), offset) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("archie-voice: {message}");
            ExitCode::FAILURE
        }
    }
}

fn take(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = args.iter().position(|arg| arg == flag)?;
    if at + 1 >= args.len() {
        return None;
    }
    let value = args.remove(at + 1);
    args.remove(at);
    Some(value)
}

/// What both commands share: who is asked, how, and about what.
struct Asking {
    heard: String,
    model: String,
    language: String,
    budget: SpeechBudget,
    grade: PrivacyGrade,
    runtime: tokio::runtime::Runtime,
    cognitive: NemotronCognitive<NebiusSource>,
    clock: String,
}

fn run(mut args: Vec<String>, offset: Option<time::UtcOffset>) -> Result<ExitCode, String> {
    const USAGE: &str = "usage: archie-voice ask|do \"<what was said>\" --grade public";
    let command = match args.first().map(String::as_str) {
        Some("ask") | Some("do") => args.remove(0),
        _ => return Err(USAGE.into()),
    };
    // `do` takes its stage flags out before the shared ones are read.
    let stage = if command == "do" {
        Some(stage_flags(&mut args)?)
    } else {
        None
    };
    let asking = asking(args, offset)?;
    match stage {
        None => ask(asking),
        Some(flags) => act(asking, flags),
    }
}

fn asking(mut args: Vec<String>, offset: Option<time::UtcOffset>) -> Result<Asking, String> {
    let model = take(&mut args, "--model").unwrap_or_else(|| "nvidia/Nemotron-3_5-Lightning".into());
    let language = take(&mut args, "--language").unwrap_or_else(|| "es-CL".into());
    let budget = match take(&mut args, "--budget").as_deref() {
        None | Some("full") => SpeechBudget::Full,
        Some("sentence") => SpeechBudget::Sentence,
        Some(other) => return Err(format!("--budget is full or sentence, not {other}")),
    };
    let grade = match take(&mut args, "--grade").as_deref() {
        Some("public") => PrivacyGrade::Public,
        Some("internal") => PrivacyGrade::Internal,
        Some(other) => return Err(format!("--grade is public or internal, not {other}")),
        None => {
            return Err("say how private this is with --grade; it has no default, because what \
                        a person says to their computer is theirs"
                .into())
        }
    };
    if args.len() != 1 {
        return Err(format!("say one thing, in quotes; got {args:?}"));
    }
    let heard = args.remove(0);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let endpoint = ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1")
        .map_err(|error| error.to_string())?;
    let client = InferenceClient::new(endpoint).map_err(|error| error.to_string())?;
    let source = NebiusSource::new(
        client,
        descriptor(&model, "voice", true, PrivacyGrade::Public)?,
        grade,
        runtime.handle().clone(),
    );
    let cognitive = NemotronCognitive::new(
        source,
        model.clone(),
        vec!["es".into(), "en".into()],
        tools::os_control(),
    );
    Ok(Asking {
        heard,
        model,
        language,
        budget,
        grade,
        runtime,
        cognitive,
        clock: clock(offset),
    })
}

fn ask(mut asking: Asking) -> Result<ExitCode, String> {
    asking.cognitive.set_context(asking.clock.clone());
    let started = Instant::now();
    asking
        .cognitive
        .begin(CognitiveRequest {
            turn_id: "turn-1".into(),
            transcript: asking.heard.clone(),
            language: asking.language.clone(),
            budget: asking.budget,
        })
        .map_err(|error| error.to_string())?;
    println!("heard: {}", asking.heard);
    let step = listen(&mut asking.cognitive, started);
    if step.failed {
        return Ok(ExitCode::from(4));
    }
    for intent in &step.intents {
        println!("        tool  {}({})   proposed, not done", intent.tool_name, intent.raw_arguments);
    }
    match step.first_piece {
        Some(ms) => println!("first words after {ms} ms, with {}", asking.model),
        None => println!("no words: the model only proposed tools, with {}", asking.model),
    }
    Ok(ExitCode::SUCCESS)
}

/// What one step of the model produced.
struct Step {
    intents: Vec<archie_speech::SpeechToolIntentCandidate>,
    first_piece: Option<u128>,
    failed: bool,
}

/// Reads one step to its end, printing what a voice would say as it arrives.
fn listen(cognitive: &mut NemotronCognitive<NebiusSource>, started: Instant) -> Step {
    let mut step = Step {
        intents: Vec::new(),
        first_piece: None,
        failed: false,
    };
    loop {
        match cognitive.poll() {
            Some(CognitiveEvent::Segment(segment)) => {
                let at = started.elapsed().as_millis();
                step.first_piece.get_or_insert(at);
                println!("{at:>6} ms  say   {}", segment.text);
            }
            Some(CognitiveEvent::ToolIntent(intent)) => {
                println!(
                    "{:>6} ms  tool  {}({})",
                    started.elapsed().as_millis(),
                    intent.tool_name,
                    intent.raw_arguments
                );
                step.intents.push(intent);
            }
            Some(CognitiveEvent::Completed { .. }) => {
                println!("{:>6} ms  done", started.elapsed().as_millis());
                return step;
            }
            Some(CognitiveEvent::Failed { reason, .. }) => {
                println!("{:>6} ms  failed: {reason}", started.elapsed().as_millis());
                step.failed = true;
                return step;
            }
            None => {
                if cognitive.running_turn().is_none() {
                    return step;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// Where the stage is, and how to act on it.
struct StageFlags {
    socket: String,
    key_file: String,
    policy: String,
    display: Option<String>,
    steps: usize,
    ground_model: Option<String>,
    ground_endpoint: Option<String>,
    app_log: Option<String>,
}

fn stage_flags(args: &mut Vec<String>) -> Result<StageFlags, String> {
    let steps = match take(args, "--steps") {
        None => 6,
        Some(steps) => steps
            .parse::<usize>()
            .ok()
            .filter(|steps| (1..=8).contains(steps))
            .ok_or("--steps is between 1 and 8")?,
    };
    Ok(StageFlags {
        socket: take(args, "--socket").ok_or("do needs --socket")?,
        key_file: take(args, "--key-file").ok_or("do needs --key-file")?,
        policy: take(args, "--policy").ok_or("do needs --policy")?,
        display: take(args, "--stage-display"),
        steps,
        ground_model: take(args, "--ground-model"),
        ground_endpoint: take(args, "--ground-endpoint"),
        app_log: take(args, "--app-log"),
    })
}

#[cfg(not(unix))]
fn act(_asking: Asking, _flags: StageFlags) -> Result<ExitCode, String> {
    Err("do acts on a stage, which listens on a Unix socket; run it where the stage runs".into())
}

#[cfg(unix)]
fn act(mut asking: Asking, flags: StageFlags) -> Result<ExitCode, String> {
    use archie_iris::LocateAnythingClient;
    use archie_policy::{PolicyEngine, Principal};
    use archie_sight::Sight;
    use archie_stage::{learn, Operator, StageClient, StagePolicy, StageWorld};
    use archie_types::AgentId;
    use archie_voice::{decide, Grounding, Hands, Launch};
    use std::path::Path;
    use std::sync::Arc;
    use tokio::net::UnixStream;

    let policy = StagePolicy::load(&flags.policy).map_err(|error| error.to_string())?;
    let key = load_key(&flags.key_file)?;
    let snapshot_dir = Path::new(&flags.socket)
        .parent()
        .ok_or("--socket has no directory")?
        .join("snapshots");
    let (client, world) = asking.runtime.block_on(async {
        let stream = UnixStream::connect(&flags.socket)
            .await
            .map_err(|error| format!("{}: {error}", flags.socket))?;
        let (read, write) = stream.into_split();
        let mut client = StageClient::new(read, write);
        let mut world = StageWorld::default();
        learn(&mut client, &mut world, Duration::from_millis(400))
            .await
            .map_err(|error| error.to_string())?;
        Ok::<_, String>((client, world))
    })?;
    let principal = Principal::Agent {
        agent: AgentId::new(),
    };
    let operator = Operator::new(key, policy.clone(), PolicyEngine::default());
    let mut sight = Sight::new(operator, client, world, snapshot_dir, principal.clone());
    // What applications declare about their controls is the first way to find one: exact,
    // live, and it needs neither a capture nor a model.
    match asking.runtime.block_on(archie_a11y::Accessibility::connect()) {
        Ok(bus) => sight = sight.asking(Box::new(bus)),
        Err(error) => eprintln!("archie-voice: nothing to ask about controls ({error})"),
    }
    let mut hands = Hands::new(
        sight,
        principal,
        asking.grade,
        Launch {
            wayland_display: flags.display.clone(),
            log: flags.app_log.clone().map(Into::into),
        },
    );
    if let Some(model) = &flags.ground_model {
        let cloud = flags.ground_endpoint.is_none();
        let endpoint = match &flags.ground_endpoint {
            None => ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1"),
            Some(url) => ProviderEndpoint::nvidia_nim(url, Locality::Local),
        }
        .map_err(|error| error.to_string())?;
        let descriptor = descriptor(model, "look", cloud, asking.grade)?;
        let locality = descriptor.locality;
        let client = LocateAnythingClient::new(endpoint, descriptor).map_err(|error| error.to_string())?;
        hands = hands.seeing(Grounding {
            client: Arc::new(client),
            locality,
        });
    }

    let openable: Vec<String> = policy
        .apps()
        .filter(|app| !app.launch.is_empty())
        .map(|app| app.display_name.clone())
        .collect();
    let context = |front: Option<String>| {
        let mut context = asking.clock.clone();
        context.push_str(&match front {
            Some(app) => format!("\nAplicación al frente: {app}."),
            None => "\nNo hay ninguna aplicación al frente.".to_string(),
        });
        if !openable.is_empty() {
            context.push_str(&format!("\nAplicaciones que puedes abrir: {}.", openable.join(", ")));
        }
        context
    };

    println!("heard: {}", asking.heard);
    let started = Instant::now();
    asking.cognitive.set_context(context(hands.front()));
    asking
        .cognitive
        .begin(CognitiveRequest {
            turn_id: "turn-1".into(),
            transcript: asking.heard.clone(),
            language: asking.language.clone(),
            budget: asking.budget,
        })
        .map_err(|error| error.to_string())?;
    let mut performed = 0usize;
    let mut refused = 0usize;
    for number in 1..=flags.steps {
        let step = listen(&mut asking.cognitive, started);
        if step.failed {
            return Ok(ExitCode::from(4));
        }
        if step.intents.is_empty() {
            println!(
                "finished in {} ms after {number} step(s): {performed} done, {refused} not done",
                started.elapsed().as_millis()
            );
            return Ok(ExitCode::SUCCESS);
        }
        let mut results = Vec::new();
        for intent in &step.intents {
            let front = hands.front();
            let outcome = match decide(intent, &policy, front.as_deref()) {
                Ok(plan) => {
                    let at = Instant::now();
                    let done = asking.runtime.block_on(hands.perform(&plan));
                    println!(
                        "{:>6} ms  {}  {:?} in {} ms: {}",
                        started.elapsed().as_millis(),
                        if done.done { "did " } else { "not " },
                        plan_name(&plan),
                        at.elapsed().as_millis(),
                        done.report
                    );
                    if done.done {
                        performed += 1;
                    } else {
                        refused += 1;
                    }
                    done.report
                }
                Err(refusal) => {
                    refused += 1;
                    println!(
                        "{:>6} ms  refused ({}): {}",
                        started.elapsed().as_millis(),
                        refusal.code,
                        refusal.spoken
                    );
                    format!("No se hizo: {}", refusal.spoken)
                }
            };
            results.push((intent.correlation_id.clone(), outcome));
        }
        if number == flags.steps {
            break;
        }
        asking.cognitive.set_context(context(hands.front()));
        asking
            .cognitive
            .follow_up(format!("turn-1.{number}"), &results, asking.budget)
            .map_err(|error| error.to_string())?;
    }
    println!(
        "stopped after {} steps: {performed} done, {refused} not done",
        flags.steps
    );
    Ok(ExitCode::from(5))
}

#[cfg(unix)]
fn plan_name(plan: &archie_voice::Plan) -> String {
    use archie_voice::Plan;
    match plan {
        Plan::Open { app_id, .. } => format!("open {app_id}"),
        Plan::Click { app_id, target } => format!("click {target:?} in {app_id}"),
        Plan::Type { app_id, text } => format!("type {} chars in {app_id}", text.chars().count()),
        Plan::Keys { app_id, chord } => format!("keys {} in {app_id}", chord.key.name()),
        Plan::Scroll { app_id, down, steps } => {
            format!("scroll {} {steps} in {app_id}", if *down { "down" } else { "up" })
        }
    }
}

#[cfg(unix)]
fn load_key(path: &str) -> Result<archie_stage::CommandKey, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let bytes: [u8; 32] = hex::decode(text.trim())
        .map_err(|_| format!("{path}: not a hex key"))?
        .try_into()
        .map_err(|_| format!("{path}: a stage key is 32 bytes"))?;
    Ok(archie_stage::CommandKey::from_bytes(&bytes))
}

/// The clock, as a fact the model is given rather than a thing it guesses.
fn clock(offset: Option<time::UtcOffset>) -> String {
    let now = time::OffsetDateTime::now_utc();
    let (local, zone) = match offset {
        Some(offset) => (now.to_offset(offset), format!("UTC{offset}")),
        None => (now, "UTC (no se pudo leer la zona local)".to_string()),
    };
    format!(
        "Hora local: {:02}:{:02}, {} {} de {} de {}, zona {zone}.",
        local.hour(),
        local.minute(),
        weekday(local.weekday()),
        local.day(),
        month(local.month()),
        local.year()
    )
}

fn weekday(day: time::Weekday) -> &'static str {
    use time::Weekday::*;
    match day {
        Monday => "lunes",
        Tuesday => "martes",
        Wednesday => "miércoles",
        Thursday => "jueves",
        Friday => "viernes",
        Saturday => "sábado",
        Sunday => "domingo",
    }
}

fn month(month: time::Month) -> &'static str {
    use time::Month::*;
    match month {
        January => "enero",
        February => "febrero",
        March => "marzo",
        April => "abril",
        May => "mayo",
        June => "junio",
        July => "julio",
        August => "agosto",
        September => "septiembre",
        October => "octubre",
        November => "noviembre",
        December => "diciembre",
    }
}

/// A model as the router knows it. In the cloud, with retention nobody has verified, it
/// may see what was graded public and nothing more, whatever the command line asked for.
fn descriptor(
    provider_model: &str,
    role: &str,
    cloud: bool,
    grade: PrivacyGrade,
) -> Result<ModelDescriptor, String> {
    let mut labels = BTreeMap::new();
    labels.insert("provider-model".to_string(), provider_model.to_string());
    let sanitized: String = provider_model
        .chars()
        .take(96)
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    Ok(ModelDescriptor {
        id: ModelId::parse(format!("{role}.model-{sanitized}"))
            .map_err(|_| "the model name cannot become an Archie model id".to_string())?,
        display_name: provider_model.to_string(),
        provider: if cloud {
            ProviderKind::NebiusTokenFactory
        } else {
            ProviderKind::NvidiaNim
        },
        locality: if cloud { Locality::Cloud } else { Locality::Local },
        node: None,
        context_window: 32_000,
        // A grounding model that reasons before it answers needs room for one box.
        max_output_tokens: if role == "look" { 512 } else { 400 },
        capabilities: vec![Capability::known("compute.inference")],
        max_privacy_grade: if cloud { PrivacyGrade::Public } else { grade },
        retention: if cloud {
            DataRetention::Unknown
        } else {
            DataRetention::ZeroVerified
        },
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: None,
        quality_tier: 50,
        required_accelerator_bytes: None,
        labels,
    })
}
