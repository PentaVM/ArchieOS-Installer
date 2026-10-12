//! `archie-print` — the operator prints a document or some text on a LAN printer.
//!
//! ```text
//! archie-print --printer ipp://192.168.1.232/ipp/print [--expect-uuid urn:uuid:…] FILE
//! archie-print --printer ipp://192.168.1.232/ipp/print --text "Hola"
//! ```
//!
//! It asks the printer what it is and what it accepts, shows what would be sent, and
//! sends it only when the person at the terminal types `print`. Then it follows the job
//! until the printer says how it ended, and prints a receipt as JSON. A PDF, JPEG, PWG
//! or URF file is sent as it is when the printer takes that format; text is drawn into
//! pages first. This is the operator's tool: it is not reachable from a model.
use archie_sentry::adapters::ipp::print::{format_of, JobState};
use archie_sentry::adapters::ipp::{IppClient, PrinterState};
use archie_sentry::page;
use sha2::{Digest, Sha256};
use std::io::{BufRead, Write};
use std::time::Duration;

const FOLLOW_FOR: Duration = Duration::from_secs(180);

fn take(arguments: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = arguments.iter().position(|a| a == flag)?;
    (at + 1 < arguments.len()).then(|| {
        arguments.remove(at);
        arguments.remove(at)
    })
}

async fn run() -> Result<bool, String> {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    let printer = take(&mut arguments, "--printer");
    let expected = take(&mut arguments, "--expect-uuid").map(|uuid| uuid.to_ascii_lowercase());
    let text = take(&mut arguments, "--text");
    let (Some(printer), true) = (printer, text.is_some() != (arguments.len() == 1)) else {
        return Err("usage: archie-print --printer ipp://ADDRESS/ipp/print [--expect-uuid UUID] (FILE | --text TEXT)".into());
    };
    let client = IppClient::new(&printer).map_err(|_| {
        "the printer must be an ipp:// or ipps:// address on the local network, with a path"
            .to_string()
    })?;
    let snapshot = client
        .get_printer_attributes(1)
        .await
        .map_err(|e| format!("the printer did not answer ({e})"))?;
    if expected.is_some() && snapshot.uuid != expected {
        return Err(format!(
            "this is not the expected printer: it says it is {}",
            snapshot.uuid.as_deref().unwrap_or("nothing")
        ));
    }
    if snapshot.state == PrinterState::Stopped || snapshot.accepting_jobs == Some(false) {
        return Err(format!(
            "the printer is not taking jobs: {:?}, {}",
            snapshot.state,
            snapshot.reasons.join(", ")
        ));
    }
    let accepts = |format: &str| snapshot.document_formats.iter().any(|f| f == format);

    let (name, format, documents): (String, &str, Vec<Vec<u8>>) = match (&text, arguments.first()) {
        (None, Some(path)) => {
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("document")
                .to_string();
            match format_of(&bytes) {
                Some(format) if accepts(format) => (name, format, vec![bytes]),
                Some(format) => {
                    return Err(format!(
                        "{path} is {format}, and this printer takes only {}",
                        snapshot.document_formats.join(", ")
                    ))
                }
                None => match String::from_utf8(bytes) {
                    Ok(written) => (
                        name,
                        "image/jpeg",
                        page::pages(&written).map_err(|e| e.to_string())?,
                    ),
                    Err(_) => {
                        return Err(format!(
                            "{path} is neither text nor a PDF, JPEG, PWG or URF document"
                        ))
                    }
                },
            }
        }
        (Some(written), _) => (
            "Archie".to_string(),
            "image/jpeg",
            page::pages(written).map_err(|e| e.to_string())?,
        ),
        (None, None) => unreachable!("checked above"),
    };
    if !accepts(format) {
        return Err(format!(
            "this printer does not take {format}; it takes {}",
            snapshot.document_formats.join(", ")
        ));
    }
    let mut digest = Sha256::new();
    documents
        .iter()
        .for_each(|document| digest.update(document));
    let digest = format!("{:x}", digest.finalize());

    eprintln!(
        "printer:  {} ({})",
        snapshot.name.as_deref().unwrap_or("unnamed"),
        snapshot.uuid.as_deref().unwrap_or("no UUID")
    );
    eprintln!(
        "document: {name}, {} page job(s) as {format}, sha256 {digest}",
        documents.len()
    );
    eprint!("Type print to send it: ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    if answer.trim() != "print" {
        return Err("not sent".into());
    }

    let mut request_id = 1;
    let mut jobs = Vec::new();
    let mut all_completed = true;
    for (index, document) in documents.iter().enumerate() {
        request_id += 1;
        let job_name = if documents.len() == 1 {
            name.clone()
        } else {
            format!("{name} ({}/{})", index + 1, documents.len())
        };
        let mut job = client
            .print_job(request_id, &job_name, format, document)
            .await
            .map_err(|e| {
                format!(
                    "job {} was not accepted ({e}); it is not sent again",
                    index + 1
                )
            })?;
        let started = std::time::Instant::now();
        while !job.state.ended() && started.elapsed() < FOLLOW_FOR {
            tokio::time::sleep(Duration::from_secs(2)).await;
            request_id += 1;
            match client.job(request_id, job.job_id).await {
                Ok(now) => job = now,
                // Printers forget a job soon after it ends; the last state read stands.
                Err(_) => break,
            }
        }
        all_completed &= job.state == JobState::Completed;
        jobs.push(job);
    }
    let receipt = serde_json::json!({
        "printer_uuid": snapshot.uuid,
        "printer_name": snapshot.name,
        "document": name,
        "format": format,
        "sha256": digest,
        "jobs": jobs,
        "completed": all_completed,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).expect("a receipt serializes")
    );
    Ok(all_completed)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("archie-print: the printer did not report every job completed");
            std::process::ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("archie-print: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
