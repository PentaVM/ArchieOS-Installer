//! `archie-voice` — one turn of conversation, typed instead of spoken, against the real model.
//!
//! ```text
//! archie-voice ask "<what the person said>" --grade public
//!              [--model nvidia/Nemotron-3_5-Lightning] [--budget full|sentence]
//!              [--language es-CL]
//! ```
//!
//! This is the cognitive half of the voice on its own: what was heard goes in, and out
//! come the pieces a voice would say and the tools the model proposes, each stamped with
//! how long after the question it arrived. The time to the first piece is the silence a
//! person would hear after they stop talking, so it is the number this tool exists to show.
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

fn run(mut args: Vec<String>, offset: Option<time::UtcOffset>) -> Result<ExitCode, String> {
    if args.first().map(String::as_str) != Some("ask") {
        return Err("usage: archie-voice ask \"<what was said>\" --grade public".into());
    }
    args.remove(0);
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
    let source = NebiusSource::new(client, descriptor(&model)?, grade, runtime.handle().clone());
    let mut cognitive = NemotronCognitive::new(
        source,
        model.clone(),
        vec!["es".into(), "en".into()],
        tools::os_control(),
    );
    cognitive.set_context(clock(offset));

    let started = Instant::now();
    cognitive
        .begin(CognitiveRequest {
            turn_id: "turn-1".into(),
            transcript: heard.clone(),
            language,
            budget,
        })
        .map_err(|error| error.to_string())?;

    println!("heard: {heard}");
    let mut first_piece: Option<u128> = None;
    let mut said = Vec::new();
    loop {
        match cognitive.poll() {
            Some(CognitiveEvent::Segment(segment)) => {
                let at = started.elapsed().as_millis();
                first_piece.get_or_insert(at);
                println!("{at:>6} ms  say   {}", segment.text);
                said.push(segment.text);
            }
            Some(CognitiveEvent::ToolIntent(intent)) => {
                println!(
                    "{:>6} ms  tool  {}({})   proposed, not done",
                    started.elapsed().as_millis(),
                    intent.tool_name,
                    intent.raw_arguments
                );
            }
            Some(CognitiveEvent::Completed { .. }) => {
                println!("{:>6} ms  done", started.elapsed().as_millis());
                break;
            }
            Some(CognitiveEvent::Failed { reason, .. }) => {
                println!("{:>6} ms  failed: {reason}", started.elapsed().as_millis());
                return Ok(ExitCode::from(4));
            }
            None => {
                if cognitive.running_turn().is_none() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    match first_piece {
        Some(ms) => println!("first words after {ms} ms, with {model}"),
        None => println!("no words: the model only proposed tools, with {model}"),
    }
    Ok(ExitCode::SUCCESS)
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

/// A Nebius model as the router knows it. Unverified retention in the cloud, so it may see
/// what was graded public and nothing more, whatever the command line asked for.
fn descriptor(provider_model: &str) -> Result<ModelDescriptor, String> {
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
        id: ModelId::parse(format!("voice.model-{sanitized}"))
            .map_err(|_| "the model name cannot become an Archie model id".to_string())?,
        display_name: provider_model.to_string(),
        provider: ProviderKind::NebiusTokenFactory,
        locality: Locality::Cloud,
        node: None,
        context_window: 32_000,
        max_output_tokens: 400,
        capabilities: vec![Capability::known("compute.inference")],
        max_privacy_grade: PrivacyGrade::Public,
        retention: DataRetention::Unknown,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: None,
        quality_tier: 50,
        required_accelerator_bytes: None,
        labels,
    })
}
