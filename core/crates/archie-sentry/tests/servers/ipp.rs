//! An IPP/1.1 printer (RFC 8010/8011) that queues jobs: Get-Printer-Attributes,
//! Print-Job, Get-Jobs (`which-jobs` completed/not-completed) and
//! Get-Job-Attributes. A job is checked against `document-format-supported`
//! (0x040a otherwise), its bytes are checked against the declared format
//! (a mismatch aborts the job with `document-format-error`, as printers do),
//! and it advances pending → processing → completed one step each time
//! someone asks about it with Get-Job-Attributes, or all at once with
//! [`Printer::finish_all`]. The printer is `processing` while any job is not.
use super::http::{HttpServer, Request, Response};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

pub const PATH: &str = "/ipp/print";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Pending = 3,
    Processing = 5,
    Aborted = 8,
    Completed = 9,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u32,
    pub name: String,
    pub format: String,
    pub sha256: String,
    pub state: JobState,
    pub reasons: Vec<String>,
}

#[derive(Debug)]
pub struct PrinterState {
    pub uuid: String,
    pub formats: Vec<String>,
    pub accepting: bool,
    pub jobs: Vec<Job>,
    next_id: u32,
}

pub struct Printer {
    /// `ipp://127.0.0.1:<port>/ipp/print`
    pub uri: String,
    pub state: Arc<Mutex<PrinterState>>,
    server: HttpServer,
}
impl Printer {
    pub async fn start(uuid: &str, formats: &[&str]) -> Self {
        let state = Arc::new(Mutex::new(PrinterState {
            uuid: uuid.into(),
            formats: formats.iter().map(|f| f.to_string()).collect(),
            accepting: true,
            jobs: Vec::new(),
            next_id: 100,
        }));
        let s = state.clone();
        let server = HttpServer::start(Arc::new(move |r: &Request| handle(&s, r))).await;
        Self {
            uri: format!("{}{PATH}", server.base.replacen("http://", "ipp://", 1)),
            state,
            server,
        }
    }
    pub fn finish_all(&self) {
        for job in &mut self.state.lock().unwrap().jobs {
            if job.state != JobState::Aborted {
                job.state = JobState::Completed;
            }
        }
    }
    pub fn log(&self) -> Vec<String> {
        self.server.log()
    }
}

struct Attribute {
    group: u8,
    tag: u8,
    name: String,
    value: Vec<u8>,
}

/// Request → (operation, request id, attributes, document data).
fn decode(bytes: &[u8]) -> Option<(u16, u32, Vec<Attribute>, Vec<u8>)> {
    if bytes.len() < 9 || bytes[0] != 1 && bytes[0] != 2 {
        return None;
    }
    let op = u16::from_be_bytes([bytes[2], bytes[3]]);
    let id = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let mut at = 8;
    let mut group = 0;
    let mut last = String::new();
    let mut attributes = Vec::new();
    loop {
        let tag = *bytes.get(at)?;
        at += 1;
        if tag == 3 {
            return Some((op, id, attributes, bytes[at..].to_vec()));
        }
        if tag <= 0x0f {
            group = tag;
            continue;
        }
        let nl = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        at += 2;
        if nl > 0 {
            last = String::from_utf8(bytes.get(at..at + nl)?.to_vec()).ok()?;
        }
        at += nl;
        let vl = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        at += 2;
        let value = bytes.get(at..at + vl)?.to_vec();
        at += vl;
        attributes.push(Attribute {
            group,
            tag,
            name: last.clone(),
            value,
        });
    }
}

fn attr(out: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
    out.push(tag);
    out.extend_from_slice(&(name.len() as u16).to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
}
fn header(status: u16, id: u32) -> Vec<u8> {
    let mut out = vec![1, 1];
    out.extend_from_slice(&status.to_be_bytes());
    out.extend_from_slice(&id.to_be_bytes());
    out.push(1);
    attr(&mut out, 0x47, "attributes-charset", b"utf-8");
    attr(&mut out, 0x48, "attributes-natural-language", b"en");
    out
}
fn reply(mut body: Vec<u8>) -> Response {
    body.push(3);
    Response::new(200, "application/ipp", body)
}
fn job_group(out: &mut Vec<u8>, job: &Job, with_name: bool) {
    out.push(2);
    attr(out, 0x21, "job-id", &(job.id as i32).to_be_bytes());
    if with_name {
        attr(out, 0x42, "job-name", job.name.as_bytes());
    }
    attr(out, 0x23, "job-state", &(job.state as i32).to_be_bytes());
    let reasons = if job.reasons.is_empty() {
        vec!["none".to_owned()]
    } else {
        job.reasons.clone()
    };
    for (i, reason) in reasons.iter().enumerate() {
        attr(
            out,
            0x44,
            if i == 0 { "job-state-reasons" } else { "" },
            reason.as_bytes(),
        );
    }
}

fn magic_matches(format: &str, data: &[u8]) -> bool {
    match format {
        "application/pdf" => data.starts_with(b"%PDF"),
        "image/jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
        "image/pwg-raster" => data.starts_with(b"RaS2"),
        "image/urf" => data.starts_with(b"UNIRAST\0"),
        _ => !data.is_empty(),
    }
}

fn handle(state: &Mutex<PrinterState>, request: &Request) -> Response {
    if request.method != "POST" || request.path != PATH {
        return Response::empty(404);
    }
    if request.header("content-type") != Some("application/ipp") {
        return Response::empty(400);
    }
    let Some((op, id, attributes, data)) = decode(&request.body) else {
        return Response::empty(400);
    };
    let find = |name: &str| attributes.iter().find(|a| a.group == 1 && a.name == name);
    if find("attributes-charset").map(|a| a.value.as_slice()) != Some(b"utf-8".as_slice())
        || find("printer-uri").is_none()
    {
        return reply(header(0x0400, id));
    }
    let mut p = state.lock().unwrap();
    match op {
        // Get-Printer-Attributes
        0x000b => {
            let mut out = header(0, id);
            out.push(4);
            let busy = p
                .jobs
                .iter()
                .any(|j| matches!(j.state, JobState::Pending | JobState::Processing));
            attr(&mut out, 0x42, "printer-name", b"loopback");
            attr(&mut out, 0x45, "printer-uuid", p.uuid.as_bytes());
            attr(
                &mut out,
                0x23,
                "printer-state",
                &(if busy { 4i32 } else { 3 }).to_be_bytes(),
            );
            attr(&mut out, 0x44, "printer-state-reasons", b"none");
            attr(
                &mut out,
                0x22,
                "printer-is-accepting-jobs",
                &[u8::from(p.accepting)],
            );
            for (i, f) in p.formats.iter().enumerate() {
                attr(
                    &mut out,
                    0x49,
                    if i == 0 {
                        "document-format-supported"
                    } else {
                        ""
                    },
                    f.as_bytes(),
                );
            }
            attr(
                &mut out,
                0x41,
                "printer-make-and-model",
                b"Loopback IPP Everywhere",
            );
            reply(out)
        }
        // Print-Job
        0x0002 => {
            let name = find("job-name").map(|a| String::from_utf8_lossy(&a.value).into_owned());
            let format = find("document-format")
                .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                .unwrap_or_else(|| "application/octet-stream".into());
            if !p.accepting {
                return reply(header(0x0506, id)); // server-error-not-accepting-jobs
            }
            if !p.formats.contains(&format) {
                return reply(header(0x040a, id));
            }
            p.next_id += 1;
            let job = Job {
                id: p.next_id,
                name: name.unwrap_or_else(|| format!("job-{}", p.next_id)),
                sha256: format!("{:x}", Sha256::digest(&data)),
                state: if magic_matches(&format, &data) {
                    JobState::Pending
                } else {
                    JobState::Aborted
                },
                reasons: if magic_matches(&format, &data) {
                    vec![]
                } else {
                    vec!["document-format-error".into()]
                },
                format,
            };
            let mut out = header(0, id);
            job_group(&mut out, &job, false);
            p.jobs.push(job);
            reply(out)
        }
        // Get-Jobs
        0x000a => {
            let which = find("which-jobs")
                .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                .unwrap_or_else(|| "not-completed".into());
            let with_name = attributes
                .iter()
                .any(|a| a.name == "requested-attributes" && a.value == b"job-name");
            let mut out = header(0, id);
            for job in &p.jobs {
                let done = matches!(job.state, JobState::Completed | JobState::Aborted);
                if (which == "completed") == done {
                    job_group(&mut out, job, with_name);
                }
            }
            reply(out)
        }
        // Get-Job-Attributes: asking advances the job one step.
        0x0009 => {
            let Some(job_id) = find("job-id").filter(|a| a.value.len() == 4).map(|a| {
                i32::from_be_bytes([a.value[0], a.value[1], a.value[2], a.value[3]]) as u32
            }) else {
                return reply(header(0x0400, id));
            };
            let Some(job) = p.jobs.iter_mut().find(|j| j.id == job_id) else {
                return reply(header(0x0406, id)); // client-error-not-found
            };
            let mut out = header(0, id);
            job_group(&mut out, job, false);
            job.state = match job.state {
                JobState::Pending => JobState::Processing,
                JobState::Processing => JobState::Completed,
                other => other,
            };
            reply(out)
        }
        _ => reply(header(0x0501, id)), // server-error-operation-not-supported
    }
}
