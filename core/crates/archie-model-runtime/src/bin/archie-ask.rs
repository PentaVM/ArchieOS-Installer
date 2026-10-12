//! `archie-ask` — ask the node's reasoning model about text that may be private.
//!
//! ```text
//! archie-ask [--grade sensitive] [--role response|reasoning] [--know person="Camila Rojas"]...
//!            [--system TEXT] [--models /etc/archie/models] < question
//! ```
//!
//! The router chooses the model. One on this machine is given the text as it is. A cloud
//! model is given it veiled: keys, addresses, numbers that identify and the names given
//! with `--know` leave as stand-ins, and the real values are put back into the answer
//! here. The answer goes to standard output; which model answered and what was covered,
//! to standard error.

use archie_model_runtime::routed::{Question, Routed};
use archie_types::{ModelRole, PrivacyGrade};
use archie_vault::{EntityKind, Veil};
use clap::{Parser, ValueEnum};
use std::io::Read;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Grade {
    Public,
    Internal,
    Sensitive,
    Restricted,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Role {
    Response,
    Reasoning,
}

#[derive(Parser, Debug)]
#[command(about = "Ask the node's reasoning model; what must not leave the machine does not")]
struct Args {
    /// How private the question is. Restricted text is answered only by a model on this machine.
    #[arg(long, value_enum, default_value = "sensitive")]
    grade: Grade,
    #[arg(long, value_enum, default_value = "response")]
    role: Role,
    /// A name to keep here, as KIND=VALUE: person, company, project, account, location or contract.
    #[arg(long = "know", value_name = "KIND=VALUE")]
    known: Vec<String>,
    #[arg(
        long,
        default_value = "You are Archie. Answer what is asked, and nothing else."
    )]
    system: String,
    #[arg(long, default_value = "/etc/archie/models")]
    models: String,
    #[arg(long, default_value_t = 2000)]
    max_tokens: u32,
    #[arg(long, default_value_t = 120)]
    timeout_seconds: u64,
}

fn entity(text: &str) -> Result<(EntityKind, String), String> {
    let (kind, value) = text
        .split_once('=')
        .ok_or_else(|| format!("--know {text}: write it as KIND=VALUE"))?;
    let kind = match kind.to_lowercase().as_str() {
        "person" => EntityKind::Person,
        "company" => EntityKind::Company,
        "project" => EntityKind::Project,
        "account" => EntityKind::Account,
        "location" => EntityKind::Location,
        "contract" => EntityKind::Contract,
        other => return Err(format!("--know: {other} is not a kind; use person, company, project, account, location or contract")),
    };
    Ok((kind, value.to_string()))
}

async fn run(args: Args) -> Result<(), String> {
    let mut question = String::new();
    std::io::stdin()
        .read_to_string(&mut question)
        .map_err(|e| e.to_string())?;
    if question.trim().is_empty() {
        return Err("give the question on standard input".into());
    }
    let mut veil = Veil::new();
    for known in &args.known {
        let (kind, value) = entity(known)?;
        veil = veil.knowing(kind, value);
    }
    let routed = Routed::from_dir(&args.models).map_err(|e| format!("{}: {e}", args.models))?;
    let answer = routed
        .ask_private(
            &Question {
                role: match args.role {
                    Role::Response => ModelRole::CognitiveResponse,
                    Role::Reasoning => ModelRole::ReasoningWorker,
                },
                privacy: match args.grade {
                    Grade::Public => PrivacyGrade::Public,
                    Grade::Internal => PrivacyGrade::Internal,
                    Grade::Sensitive => PrivacyGrade::Sensitive,
                    Grade::Restricted => PrivacyGrade::Restricted,
                },
                system: &args.system,
                user: question.trim(),
                image: None,
                max_tokens: args.max_tokens,
                timeout: Duration::from_secs(args.timeout_seconds),
            },
            &veil,
        )
        .await
        .map_err(|e| e.to_string())?;
    println!("{}", answer.answered.text.trim_end());
    let covered: Vec<String> = answer
        .covered
        .iter()
        .map(|(kind, count)| format!("{count} {}", kind.to_lowercase()))
        .collect();
    eprintln!(
        "archie-ask: {} in {:.1} s; {}",
        answer.answered.model,
        answer.answered.seconds,
        match (answer.left_the_machine, covered.is_empty()) {
            (false, _) => "answered on this machine, nothing left it".to_string(),
            (true, true) => "sent to the cloud; nothing in it needed covering".to_string(),
            (true, false) => format!("sent to the cloud with {} kept here", covered.join(", ")),
        }
    );
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("archie-ask: {error}");
            ExitCode::FAILURE
        }
    }
}
