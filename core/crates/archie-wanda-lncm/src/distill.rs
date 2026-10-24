//! Multi-teacher distillation, through the egress gate every other model call uses.
//!
//! Four steps, each resumable (records are appended as they arrive and a rerun skips what is
//! done):
//!
//! 1. **Teach.** Every teacher answers every admitted case, several times. A case is admitted
//!    only when its provenance is owned or public, nothing in it has the shape of a credential,
//!    e-mail, phone, card or RUT, and it shares no long word sequence with an evaluation set.
//!    Calls go through `InferenceClient::chat` at [`PrivacyGrade::Public`], so a model the
//!    registry does not clear for public data is never called.
//! 2. **Draft.** Cheaper models answer plainly, as the runtime's draft model would: those
//!    answers become the `draft` input of examples, so the student learns to turn a mediocre
//!    draft into the best answer.
//! 3. **Judge.** Judges score composed answers on a fixed rubric. A judge never scores a model
//!    of its own family.
//! 4. **Select.** Exact labels win when a simulator knows them. Otherwise a decision needs a
//!    majority across samples, delta lines need to recur across samples, and a composed answer
//!    needs the best mean rubric with no judge rating its safety low.

use crate::data::{read_jsonl, Case, Choice, EvalGuard, Example, Provenance, Target};
use crate::state::parse_delta;
use crate::{bounded_text, clip_bytes, Lang, LncmError, Result, Task};
use archie_model_runtime::{
    provider_model_id, ChatMessage, ChatRequest, InferenceClient, MessageRole, ProviderEndpoint,
};
use archie_registry::ModelRegistry;
use archie_types::{ModelDescriptor, ModelId, PrivacyGrade};
use archie_vault::Veil;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_RAW_BYTES: usize = 32 * 1024;

fn one() -> usize {
    1
}
fn temperature() -> f32 {
    0.7
}
fn max_tokens() -> u32 {
    4096
}
fn concurrency() -> usize {
    4
}
fn timeout() -> u64 {
    180
}
fn half() -> f32 {
    0.5
}
fn six() -> u8 {
    6
}
fn two() -> usize {
    2
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelUse {
    /// The model's id in the registry (`models/…/*.toml`), e.g. `nebius.nemotron-3-ultra`.
    pub model: String,
    #[serde(default = "one")]
    pub samples: usize,
    #[serde(default = "temperature")]
    pub temperature: f32,
    #[serde(default = "max_tokens")]
    pub max_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistillConfig {
    pub models_dir: PathBuf,
    #[serde(default = "concurrency")]
    pub concurrency: usize,
    #[serde(default = "timeout")]
    pub timeout_seconds: u64,
    pub teachers: Vec<ModelUse>,
    #[serde(default)]
    pub judges: Vec<ModelUse>,
    #[serde(default)]
    pub drafts: Vec<ModelUse>,
    /// Stop after this many cases.
    #[serde(default)]
    pub max_cases: Option<usize>,
    /// Stop starting calls once this many completion tokens were spent, across all roles.
    #[serde(default)]
    pub max_completion_tokens: Option<u64>,
    /// Hard spending limit in US dollars, at the registry's prices, across every run that
    /// shares the ledger. A call starts only if its worst case still fits.
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Where every call's tokens and cost are appended. Required with `max_cost_usd`.
    #[serde(default)]
    pub ledger: Option<PathBuf>,
    /// Share of samples that must agree on a decision or a delta line.
    #[serde(default = "half")]
    pub min_agreement: f32,
    /// Lowest safety score any judge may give an answer that is kept.
    #[serde(default = "six")]
    pub min_safety: u8,
    /// Evaluation-only files (FANToM, OpenToM, …). Cases that overlap them are refused.
    #[serde(default)]
    pub eval_only: Vec<PathBuf>,
    #[serde(default = "two")]
    pub max_eval_overlap: usize,
    /// When a case has an exact label, use it and do not ask teachers to decide it.
    #[serde(default = "yes")]
    pub trust_gold: bool,
}

impl DistillConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = crate::codec::read_bounded(path, 1 << 20)?;
        let mut config: Self =
            toml::from_str(&text).map_err(|e| LncmError::Config(e.to_string()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        if config.models_dir.is_relative() {
            config.models_dir = base.join(&config.models_dir);
        }
        if let Some(ledger) = &mut config.ledger {
            if ledger.is_relative() {
                *ledger = base.join(&*ledger);
            }
        }
        for p in &mut config.eval_only {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let uses = self.teachers.iter().chain(&self.judges).chain(&self.drafts);
        let bad_use = |u: &ModelUse| {
            !(1..=16).contains(&u.samples)
                || !(0.0..=2.0).contains(&u.temperature)
                || !(16..=32_768).contains(&u.max_tokens)
                || ModelId::parse(u.model.clone()).is_err()
        };
        if self.teachers.is_empty()
            || !(1..=64).contains(&self.concurrency)
            || !(5..=3600).contains(&self.timeout_seconds)
            || !(0.0..=1.0).contains(&self.min_agreement)
            || self.min_safety > 10
            || self
                .max_cost_usd
                .is_some_and(|c| !(0.0..=100_000.0).contains(&c))
            || (self.max_cost_usd.is_some() && self.ledger.is_none())
            || uses.into_iter().any(bad_use)
        {
            return Err(LncmError::Config("distillation config out of range".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Teacher,
    Draft,
    Judge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parsed {
    #[serde(default)]
    pub choice: Option<Choice>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub delta: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rubric {
    pub tom: u8,
    pub evidence: u8,
    pub uncertainty: u8,
    pub actionable: u8,
    pub safety: u8,
}

impl Rubric {
    pub fn total(&self) -> u32 {
        u32::from(self.tom)
            + u32::from(self.evidence)
            + u32::from(self.uncertainty)
            + u32::from(self.actionable)
            + u32::from(self.safety)
    }
}

/// One model answer, as recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub case_id: String,
    pub role: Role,
    pub model: String,
    pub family: String,
    pub index: usize,
    /// For a judge: the key of the sample it scored.
    #[serde(default)]
    pub judged: Option<String>,
    pub text: String,
    #[serde(default)]
    pub parsed: Option<Parsed>,
    #[serde(default)]
    pub rubric: Option<Rubric>,
    pub completion_tokens: u64,
    pub seconds: f32,
    #[serde(default)]
    pub prompt_tokens: u64,
    /// Millionths of a US dollar, at the registry's prices.
    #[serde(default)]
    pub cost_micros: u64,
}

/// One call's spend, as appended to the ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerEntry {
    pub model: String,
    pub role: Role,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_micros: u64,
}

/// Total spend recorded in a ledger, in millionths of a dollar.
pub fn ledger_total(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let entries: Vec<LedgerEntry> = read_jsonl(path, 50_000_000)?;
    Ok(entries.iter().map(|e| e.cost_micros).sum())
}

/// Cost in millionths of a dollar of a call, from prices in millionths per million tokens.
pub fn cost_micros(
    prompt_tokens: u64,
    completion_tokens: u64,
    input_price: u64,
    output_price: u64,
) -> u64 {
    let total = u128::from(prompt_tokens) * u128::from(input_price)
        + u128::from(completion_tokens) * u128::from(output_price);
    total.div_ceil(1_000_000).min(u128::from(u64::MAX)) as u64
}

impl Sample {
    pub fn key(&self) -> String {
        sample_key(
            &self.case_id,
            self.role,
            &self.model,
            self.index,
            self.judged.as_deref(),
        )
    }
}

fn sample_key(case: &str, role: Role, model: &str, index: usize, judged: Option<&str>) -> String {
    format!("{case}|{role:?}|{model}|{index}|{}", judged.unwrap_or(""))
}

/// Why a case is not sent to any model, if it is not.
pub fn admit_case(
    case: &Case,
    guard: &EvalGuard,
    max_overlap: usize,
) -> std::result::Result<(), &'static str> {
    if case.validate().is_err() {
        return Err("invalid_or_private_provenance");
    }
    let texts = case.texts();
    let veiled = Veil::new().cover(&texts);
    if !veiled.unveil.is_empty() || !veiled.residue().is_empty() {
        return Err("sensitive_shape");
    }
    if !guard.is_empty() && guard.overlap(&texts.join("\n")) > max_overlap {
        return Err("overlaps_evaluation_set");
    }
    Ok(())
}

struct Words {
    teacher: &'static str,
    decide: &'static str,
    compose: &'static str,
    update: &'static str,
    draft: &'static str,
    judge: &'static str,
    state: &'static str,
    chunk: &'static str,
    question: &'static str,
    options: &'static str,
    answer: &'static str,
}

fn words(lang: Lang) -> Words {
    match lang {
        Lang::Es => Words {
            teacher: "Eres un experto en cognición social y psicología del comportamiento. Ayudas a una persona a entender situaciones de sus conversaciones de grupo. Reglas: (1) Sigue con precisión quién vio y oyó qué: quien no estaba cuando se dijo algo no lo sabe, salvo que se lo contaran. (2) Lo que infieras de una persona es una hipótesis, nunca un hecho: no diagnostiques ni atribuyas intenciones como certezas. (3) Apóyate en la evidencia del contexto y cita los turnos [n] cuando ayude. (4) Expresa la incertidumbre y di qué la resolvería. (5) Da una recomendación concreta y respetuosa; nada manipulador. (6) Si hay señales de peligro, recomienda pedir ayuda. Responde solo con JSON, sin Markdown.",
            decide: "Formato: {\"choice\": <número de la opción empezando en 0, o -1 si con lo que se sabe no se puede saber>, \"confidence\": <de 0 a 1>}",
            compose: "Formato: {\"plan\": {\"stance\": \"actitud\", \"key\": \"el dato clave\", \"action\": \"qué hacer\"}, \"answer\": \"respuesta para decir en voz alta, breve (máximo cuatro frases), en español\"}",
            update: "Lee el fragmento y devuelve qué cambia en el estado. Formato: {\"delta\": [\"...\"]}. Cada línea es una de: fact <tema> = <valor> @<turno> (un mensaje afirma o cambia un dato); told <persona> <tema> = <valor> @<turno> (alguien le contó un dato a una persona ausente); expect <persona> +|- <clave>: <enunciado> (el mensaje es evidencia a favor o en contra de un patrón de conducta; clave en minúsculas con guiones); affect <persona> <emoción> <0-100>. Personas con espacios entre comillas. Usa solo personas y turnos del fragmento. Si nada cambia: {\"delta\": [\"none\"]}.",
            draft: "Responde a la pregunta usando solo el contexto. Sé breve.",
            judge: "Evalúas respuestas de un asistente sobre situaciones sociales. Puntúa de 0 a 10: tom (sigue bien quién sabe y cree qué), evidence (se apoya en el contexto), uncertainty (reconoce lo que no se sabe), actionable (recomendación útil y concreta), safety (sin diagnósticos ni manipulación, trata lo inferido como hipótesis, pide ayuda ante peligro). Responde solo JSON: {\"tom\": n, \"evidence\": n, \"uncertainty\": n, \"actionable\": n, \"safety\": n}",
            state: "Estado conocido",
            chunk: "Conversación",
            question: "Pregunta",
            options: "Opciones",
            answer: "Respuesta a evaluar",
        },
        Lang::En => Words {
            teacher: "You are an expert in social cognition and behavioural psychology. You help a person understand situations in their group conversations. Rules: (1) Track exactly who saw and heard what: someone who was not there when something was said does not know it, unless they were told. (2) What you infer about a person is a hypothesis, never a fact: do not diagnose or present intentions as certain. (3) Rely on the evidence in the context and cite turns [n] when it helps. (4) State the uncertainty and what would resolve it. (5) Give a concrete, respectful recommendation; nothing manipulative. (6) If there are signs of danger, recommend getting help. Answer with JSON only, no Markdown.",
            decide: "Format: {\"choice\": <option number starting at 0, or -1 if it cannot be known from what is known>, \"confidence\": <0 to 1>}",
            compose: "Format: {\"plan\": {\"stance\": \"attitude\", \"key\": \"the key fact\", \"action\": \"what to do\"}, \"answer\": \"answer to be spoken aloud, short (at most four sentences), in English\"}",
            update: "Read the excerpt and return what changes in the state. Format: {\"delta\": [\"...\"]}. Each line is one of: fact <topic> = <value> @<turn> (a message states or changes a fact); told <person> <topic> = <value> @<turn> (someone told an absent person a fact); expect <person> +|- <key>: <statement> (the message is evidence for or against a behaviour pattern; lower-case key with hyphens); affect <person> <emotion> <0-100>. Quote people with spaces in their name. Use only people and turns from the excerpt. If nothing changes: {\"delta\": [\"none\"]}.",
            draft: "Answer the question using only the context. Be brief.",
            judge: "You evaluate an assistant's answers about social situations. Score 0 to 10: tom (tracks who knows and believes what), evidence (grounded in the context), uncertainty (acknowledges what is unknown), actionable (useful, concrete recommendation), safety (no diagnosis or manipulation, treats inferences as hypotheses, recommends help when there is danger). Answer JSON only: {\"tom\": n, \"evidence\": n, \"uncertainty\": n, \"actionable\": n, \"safety\": n}",
            state: "Known state",
            chunk: "Conversation",
            question: "Question",
            options: "Options",
            answer: "Answer to evaluate",
        },
    }
}

fn context_block(case: &Case, w: &Words) -> String {
    let mut parts = Vec::new();
    if let Some(s) = &case.state {
        parts.push(format!("{}:\n{s}", w.state));
    }
    if let Some(c) = &case.chunk {
        parts.push(format!("{}:\n{c}", w.chunk));
    }
    if let Some(q) = &case.question {
        parts.push(format!("{}: {q}", w.question));
    }
    if !case.options.is_empty() {
        let list = case
            .options
            .iter()
            .enumerate()
            .map(|(i, o)| format!("{i}. {o}"))
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("{}:\n{list}", w.options));
    }
    parts.join("\n\n")
}

/// System and user prompts for a role on a case.
pub fn prompts(case: &Case, role: Role, judged: Option<&Parsed>) -> (String, String) {
    let w = words(case.lang);
    let context = context_block(case, &w);
    match role {
        Role::Teacher => {
            let format = match case.task {
                Task::Decide => w.decide,
                Task::Update => w.update,
                Task::Compose | Task::Gate => w.compose,
            };
            (format!("{}\n{format}", w.teacher), context)
        }
        Role::Draft => (w.draft.to_string(), context),
        Role::Judge => {
            let answer = judged
                .map(|p| {
                    let mut text = String::new();
                    if let Some(plan) = &p.plan {
                        text.push_str(plan);
                        text.push('\n');
                    }
                    text.push_str(p.answer.as_deref().unwrap_or(""));
                    text
                })
                .unwrap_or_default();
            (
                w.judge.to_string(),
                format!("{context}\n\n{}:\n{answer}", w.answer),
            )
        }
    }
}

/// Removes a reasoning block. A block that never closed means the answer was cut off.
pub fn strip_think(text: &str) -> &str {
    if let Some(at) = text.rfind("</think>") {
        return text[at + "</think>".len()..].trim();
    }
    if text.trim_start().starts_with("<think>") {
        return "";
    }
    text.trim()
}

fn json_object(text: &str) -> Option<serde_json::Value> {
    let text = strip_think(text);
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

/// Parses a teacher's answer for `case`. `None` when it does not follow the format.
pub fn parse_answer(case: &Case, text: &str) -> Option<Parsed> {
    let v = json_object(text)?;
    let mut parsed = Parsed {
        choice: None,
        confidence: v
            .get("confidence")
            .and_then(|c| c.as_f64())
            .map(|c| c.clamp(0.0, 1.0) as f32),
        plan: None,
        answer: None,
        delta: None,
    };
    match case.task {
        Task::Decide => {
            let choice = v.get("choice")?.as_i64()?;
            parsed.choice = match choice {
                -1 => Some(Choice::Abstain),
                c if c >= 0 && (c as usize) < case.options.len() => {
                    Some(Choice::Option(c as usize))
                }
                _ => return None,
            };
        }
        Task::Compose | Task::Gate => {
            let answer = v.get("answer")?.as_str()?.trim().to_string();
            if !bounded_text(&answer, 2000) {
                return None;
            }
            parsed.answer = Some(answer);
            parsed.plan = match v.get("plan") {
                Some(serde_json::Value::Object(map)) => {
                    let field = |k: &str| {
                        map.get(k)
                            .and_then(|x| x.as_str())
                            .map(|s| clip_bytes(s.trim(), 300).to_string())
                    };
                    match (field("stance"), field("key"), field("action")) {
                        (Some(s), Some(k), Some(a))
                            if !s.is_empty() && !k.is_empty() && !a.is_empty() =>
                        {
                            Some(format!("{s} | {k} | {a}"))
                        }
                        _ => None,
                    }
                }
                Some(serde_json::Value::String(s)) if bounded_text(s, 600) => {
                    Some(s.trim().to_string())
                }
                _ => None,
            };
        }
        Task::Update => {
            let lines: Vec<String> = v
                .get("delta")?
                .as_array()?
                .iter()
                .filter_map(|l| l.as_str())
                .map(|l| l.trim().to_string())
                .collect();
            let (valid, _) = parse_delta(&lines.join("\n"));
            if valid.is_empty() {
                return None;
            }
            parsed.delta = Some(valid.iter().map(ToString::to_string).collect());
        }
    }
    Some(parsed)
}

pub fn parse_rubric(text: &str) -> Option<Rubric> {
    let v = json_object(text)?;
    let score = |k: &str| v.get(k)?.as_u64().filter(|n| *n <= 10).map(|n| n as u8);
    Some(Rubric {
        tom: score("tom")?,
        evidence: score("evidence")?,
        uncertainty: score("uncertainty")?,
        actionable: score("actionable")?,
        safety: score("safety")?,
    })
}

#[derive(Clone)]
struct Callable {
    id: String,
    family: String,
    descriptor: ModelDescriptor,
    provider_model: String,
    input_price: u64,
    output_price: u64,
}

fn resolve(registry: &ModelRegistry, id: &str) -> Result<Callable> {
    let model = ModelId::parse(id.to_string()).map_err(|e| LncmError::Config(e.to_string()))?;
    let descriptor = registry
        .get(&model)
        .ok_or_else(|| LncmError::Config(format!("{id} is not in the model registry")))?
        .clone();
    let endpoint =
        ProviderEndpoint::for_model(&descriptor).map_err(|e| LncmError::Model(e.to_string()))?;
    if !endpoint.is_configured() {
        return Err(LncmError::Config(format!(
            "{id}: its credential is not in the environment"
        )));
    }
    let provider_model = provider_model_id(&descriptor)
        .map_err(|e| LncmError::Model(e.to_string()))?
        .to_string();
    let family = provider_model
        .split_once('/')
        .map(|(f, _)| f.to_lowercase())
        .unwrap_or_else(|| id.to_lowercase());
    Ok(Callable {
        id: id.to_string(),
        family,
        input_price: descriptor.cost.input_micros_per_mtok,
        output_price: descriptor.cost.output_micros_per_mtok,
        descriptor,
        provider_model,
    })
}

async fn ask(
    model: &Callable,
    system: &str,
    user: &str,
    temperature: f32,
    max_tokens: u32,
    timeout: Duration,
) -> Result<(String, u64, u64)> {
    let endpoint = ProviderEndpoint::for_model(&model.descriptor)
        .and_then(|e| e.with_limits(timeout, 8 * 1024 * 1024))
        .map_err(|e| LncmError::Model(e.to_string()))?;
    let client = InferenceClient::new(endpoint).map_err(|e| LncmError::Model(e.to_string()))?;
    let request = ChatRequest {
        model: model.provider_model.clone(),
        messages: vec![
            ChatMessage {
                role: MessageRole::System,
                content: system.to_string(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: user.to_string(),
            },
        ],
        max_tokens: Some(max_tokens),
        temperature: Some(temperature),
        stream: false,
        reply_schema: None,
    };
    let response = client
        .chat(&model.descriptor, PrivacyGrade::Public, &request)
        .await
        .map_err(|e| LncmError::Model(e.to_string()))?;
    let text = response
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();
    let (prompt, completion) = response
        .usage
        .map_or((0, 0), |u| (u.prompt_tokens, u.completion_tokens));
    Ok((text, prompt, completion))
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    pub requested: usize,
    pub already_done: usize,
    pub completed: usize,
    pub unparsed: usize,
    pub failed: usize,
    pub refused: BTreeMap<String, usize>,
    pub not_started_budget: usize,
    pub completion_tokens: u64,
    /// Spent by this run, and recorded in the ledger before it, in millionths of a dollar.
    pub cost_micros: u64,
    pub ledger_before_micros: u64,
}

struct Job {
    case: usize,
    role: Role,
    model: Callable,
    index: usize,
    judged: Option<String>,
    judged_parsed: Option<Parsed>,
    temperature: f32,
    max_tokens: u32,
}

pub struct Runner {
    config: DistillConfig,
    registry: ModelRegistry,
    guard: EvalGuard,
}

impl Runner {
    pub fn new(config: DistillConfig) -> Result<Self> {
        let registry = ModelRegistry::load_from_dir(&config.models_dir)
            .map_err(|e| LncmError::Config(e.to_string()))?;
        let mut guard = EvalGuard::new(8);
        guard.add_files(&config.eval_only)?;
        Ok(Self {
            config,
            registry,
            guard,
        })
    }

    fn admitted<'a>(&self, cases: &'a [Case], report: &mut RunReport) -> Vec<(usize, &'a Case)> {
        let mut out = Vec::new();
        for (i, case) in cases.iter().enumerate() {
            if self.config.max_cases.is_some_and(|m| out.len() >= m) {
                break;
            }
            match admit_case(case, &self.guard, self.config.max_eval_overlap) {
                Ok(()) => out.push((i, case)),
                Err(why) => *report.refused.entry(why.to_string()).or_default() += 1,
            }
        }
        out
    }

    fn done(out: &Path) -> Result<HashSet<String>> {
        if !out.exists() {
            return Ok(HashSet::new());
        }
        let samples: Vec<Sample> = read_jsonl(out, 50_000_000)?;
        Ok(samples.iter().map(Sample::key).collect())
    }

    /// Teachers answer every admitted case.
    pub async fn teach(&self, cases: &[Case], out: &Path) -> Result<RunReport> {
        self.run_role(cases, out, Role::Teacher, &self.config.teachers.clone())
            .await
    }

    /// Draft models answer plainly.
    pub async fn draft(&self, cases: &[Case], out: &Path) -> Result<RunReport> {
        self.run_role(cases, out, Role::Draft, &self.config.drafts.clone())
            .await
    }

    async fn run_role(
        &self,
        cases: &[Case],
        out: &Path,
        role: Role,
        uses: &[ModelUse],
    ) -> Result<RunReport> {
        let mut report = RunReport::default();
        let admitted = self.admitted(cases, &mut report);
        let done = Self::done(out)?;
        let mut jobs = Vec::new();
        for u in uses {
            let model = resolve(&self.registry, &u.model)?;
            for &(index, case) in &admitted {
                let skip_gold = role == Role::Teacher
                    && self.config.trust_gold
                    && case.task == Task::Decide
                    && case.gold.as_ref().is_some_and(|g| g.choice.is_some());
                if skip_gold || (role == Role::Draft && case.task == Task::Update) {
                    continue;
                }
                for sample in 0..u.samples {
                    report.requested += 1;
                    if done.contains(&sample_key(&case.id, role, &model.id, sample, None)) {
                        report.already_done += 1;
                        continue;
                    }
                    jobs.push(Job {
                        case: index,
                        role,
                        model: model.clone(),
                        index: sample,
                        judged: None,
                        judged_parsed: None,
                        temperature: if role == Role::Draft {
                            0.0
                        } else {
                            u.temperature
                        },
                        max_tokens: u.max_tokens,
                    });
                }
            }
        }
        self.execute(cases, jobs, out, report).await
    }

    /// Judges score parsed composed answers of other families.
    pub async fn judge(
        &self,
        cases: &[Case],
        samples_path: &Path,
        out: &Path,
    ) -> Result<RunReport> {
        let mut report = RunReport::default();
        let admitted: BTreeMap<&str, usize> = self
            .admitted(cases, &mut report)
            .into_iter()
            .map(|(i, c)| (c.id.as_str(), i))
            .collect();
        let samples: Vec<Sample> = read_jsonl(samples_path, 50_000_000)?;
        let done = Self::done(out)?;
        let mut jobs = Vec::new();
        for u in &self.config.judges {
            let judge = resolve(&self.registry, &u.model)?;
            for s in samples.iter().filter(|s| s.role == Role::Teacher) {
                let (Some(&case), Some(parsed)) = (admitted.get(s.case_id.as_str()), &s.parsed)
                else {
                    continue;
                };
                if cases[case].task != Task::Compose
                    || parsed.answer.is_none()
                    || s.family == judge.family
                {
                    continue;
                }
                report.requested += 1;
                let key = s.key();
                if done.contains(&sample_key(
                    &s.case_id,
                    Role::Judge,
                    &judge.id,
                    0,
                    Some(&key),
                )) {
                    report.already_done += 1;
                    continue;
                }
                jobs.push(Job {
                    case,
                    role: Role::Judge,
                    model: judge.clone(),
                    index: 0,
                    judged: Some(key),
                    judged_parsed: Some(parsed.clone()),
                    temperature: 0.0,
                    max_tokens: u.max_tokens,
                });
            }
        }
        self.execute(cases, jobs, out, report).await
    }

    async fn execute(
        &self,
        cases: &[Case],
        jobs: Vec<Job>,
        out: &Path,
        mut report: RunReport,
    ) -> Result<RunReport> {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.concurrency));
        let writer = Arc::new(tokio::sync::Mutex::new(out.to_path_buf()));
        let spent = Arc::new(AtomicU64::new(0));
        let budget = self.config.max_completion_tokens;
        let before = match &self.config.ledger {
            Some(path) => ledger_total(path)?,
            None => 0,
        };
        report.ledger_before_micros = before;
        let cost_limit = self
            .config
            .max_cost_usd
            .map(|usd| (usd * 1_000_000.0) as u64);
        // Spent so far (ledger included) plus the worst case of every call in flight.
        let committed = Arc::new(AtomicU64::new(before));
        let ledger = Arc::new(tokio::sync::Mutex::new(self.config.ledger.clone()));
        let timeout = Duration::from_secs(self.config.timeout_seconds);
        let cases: Arc<Vec<Case>> = Arc::new(cases.to_vec());
        let mut tasks = tokio::task::JoinSet::new();
        for job in jobs {
            if budget.is_some_and(|b| spent.load(Ordering::Relaxed) >= b) {
                report.not_started_budget += 1;
                continue;
            }
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| LncmError::Model("closed".into()))?;
            if budget.is_some_and(|b| spent.load(Ordering::Relaxed) >= b) {
                report.not_started_budget += 1;
                continue;
            }
            let (system, user) = prompts(&cases[job.case], job.role, job.judged_parsed.as_ref());
            // Worst case: a generous token estimate of the prompt and every output token allowed.
            let worst = cost_micros(
                (system.len() + user.len()) as u64 / 2 + 64,
                u64::from(job.max_tokens),
                job.model.input_price,
                job.model.output_price,
            );
            if let Some(limit) = cost_limit {
                let now = committed.load(Ordering::SeqCst);
                if now.saturating_add(worst) > limit {
                    report.not_started_budget += 1;
                    continue;
                }
                committed.fetch_add(worst, Ordering::SeqCst);
            }
            let (cases, writer, spent, committed, ledger) = (
                cases.clone(),
                writer.clone(),
                spent.clone(),
                committed.clone(),
                ledger.clone(),
            );
            tasks.spawn(async move {
                let _permit = permit;
                let case = &cases[job.case];
                let started = Instant::now();
                let mut last_error = String::new();
                for attempt in 0..3u32 {
                    if attempt > 0 {
                        tokio::time::sleep(Duration::from_secs(2u64 << attempt)).await;
                    }
                    match ask(
                        &job.model,
                        &system,
                        &user,
                        job.temperature,
                        job.max_tokens,
                        timeout,
                    )
                    .await
                    {
                        Ok((text, prompt_tokens, tokens)) => {
                            spent.fetch_add(tokens, Ordering::Relaxed);
                            let cost = cost_micros(
                                prompt_tokens,
                                tokens,
                                job.model.input_price,
                                job.model.output_price,
                            );
                            if cost_limit.is_some() {
                                // Swap the reservation for what was really spent.
                                committed.fetch_add(cost, Ordering::SeqCst);
                                committed.fetch_sub(worst, Ordering::SeqCst);
                            }
                            if let Some(path) = ledger.lock().await.as_ref() {
                                crate::data::append_jsonl(
                                    path,
                                    &LedgerEntry {
                                        model: job.model.id.clone(),
                                        role: job.role,
                                        prompt_tokens,
                                        completion_tokens: tokens,
                                        cost_micros: cost,
                                    },
                                )?;
                            }
                            let (parsed, rubric) = match job.role {
                                Role::Teacher => (parse_answer(case, &text), None),
                                Role::Draft => {
                                    let answer =
                                        clip_bytes(strip_think(&text), 4000).trim().to_string();
                                    let parsed = (!answer.is_empty()).then_some(Parsed {
                                        choice: None,
                                        confidence: None,
                                        plan: None,
                                        answer: Some(answer),
                                        delta: None,
                                    });
                                    (parsed, None)
                                }
                                Role::Judge => (None, parse_rubric(&text)),
                            };
                            let ok = parsed.is_some() || rubric.is_some();
                            let sample = Sample {
                                case_id: case.id.clone(),
                                role: job.role,
                                model: job.model.id.clone(),
                                family: job.model.family.clone(),
                                index: job.index,
                                judged: job.judged.clone(),
                                text: clip_bytes(&text, MAX_RAW_BYTES).to_string(),
                                parsed,
                                rubric,
                                completion_tokens: tokens,
                                seconds: started.elapsed().as_secs_f32(),
                                prompt_tokens,
                                cost_micros: cost,
                            };
                            let path = writer.lock().await;
                            crate::data::append_jsonl(&path, &sample)?;
                            return Ok::<(bool, u64, u64), LncmError>((ok, tokens, cost));
                        }
                        Err(e) => last_error = e.to_string(),
                    }
                }
                // Nothing was spent that the provider reported; release the reservation.
                if cost_limit.is_some() {
                    committed.fetch_sub(worst, Ordering::SeqCst);
                }
                Err(LncmError::Model(last_error))
            });
        }
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok((ok, tokens, cost))) => {
                    report.completed += 1;
                    report.completion_tokens += tokens;
                    report.cost_micros += cost;
                    if !ok {
                        report.unparsed += 1;
                    }
                }
                Ok(Err(LncmError::Io(e))) => return Err(LncmError::Io(e)),
                _ => report.failed += 1,
            }
        }
        Ok(report)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectReport {
    pub cases: usize,
    pub from_gold: usize,
    pub from_teachers: usize,
    pub with_drafts: usize,
    pub dropped: BTreeMap<String, usize>,
}

/// Turns cases and their samples into training examples.
pub fn select(
    config: &DistillConfig,
    cases: &[Case],
    samples: &[Sample],
) -> (Vec<Example>, SelectReport) {
    let mut by_case: BTreeMap<&str, Vec<&Sample>> = BTreeMap::new();
    for s in samples {
        by_case.entry(s.case_id.as_str()).or_default().push(s);
    }
    let mut report = SelectReport::default();
    let mut examples = Vec::new();
    for case in cases {
        report.cases += 1;
        let empty = Vec::new();
        let mine = by_case.get(case.id.as_str()).unwrap_or(&empty);
        let teachers: Vec<&&Sample> = mine
            .iter()
            .filter(|s| s.role == Role::Teacher && s.parsed.is_some())
            .collect();
        let gold = case.gold.clone().filter(|_| config.trust_gold);
        let chosen: Option<(Target, Option<String>)> = match case.task {
            Task::Decide => match gold.filter(|g| g.choice.is_some()) {
                Some(g) => Some((g, None)),
                None => majority(
                    teachers.iter().filter_map(|s| s.parsed.as_ref()?.choice),
                    config.min_agreement,
                )
                .map(|c| {
                    (
                        Target {
                            choice: Some(c),
                            ..Default::default()
                        },
                        Some(voters(&teachers)),
                    )
                }),
            },
            Task::Update => match gold.filter(|g| g.text.is_some()) {
                Some(mut g) => {
                    g.change = Some(g.text.as_deref().is_some_and(|t| t.trim() != "none"));
                    Some((g, None))
                }
                None => vote_lines(&teachers, config.min_agreement).map(|lines| {
                    let changed = !(lines.len() == 1 && lines[0] == "none");
                    (
                        Target {
                            change: Some(changed),
                            text: Some(lines.join("\n")),
                            ..Default::default()
                        },
                        Some(voters(&teachers)),
                    )
                }),
            },
            Task::Compose | Task::Gate => best_composed(config, &teachers, mine)
                .or_else(|| gold.filter(|g| g.text.is_some()).map(|g| (g, None))),
        };
        let Some((target, teacher)) = chosen else {
            *report
                .dropped
                .entry(format!("{:?}", case.task).to_lowercase())
                .or_default() += 1;
            continue;
        };
        let provenance = match &teacher {
            Some(t) => {
                report.from_teachers += 1;
                Provenance::Distilled {
                    teacher: t.clone(),
                    case_id: case.id.clone(),
                }
            }
            None => {
                report.from_gold += 1;
                case.provenance.clone()
            }
        };
        let mut base = case.as_example(target);
        base.id = format!("{}:selected", case.id);
        base.provenance = provenance;
        for (i, draft) in mine
            .iter()
            .filter(|s| s.role == Role::Draft)
            .filter_map(|s| s.parsed.as_ref()?.answer.clone().map(|a| (s, a)))
            .enumerate()
        {
            let mut with_draft = base.clone();
            with_draft.id = format!("{}:draft:{}:{i}", case.id, draft.0.model);
            with_draft.draft = Some(draft.1);
            if with_draft.validate().is_ok() {
                examples.push(with_draft);
                report.with_drafts += 1;
            }
        }
        if base.validate().is_ok() {
            examples.push(base);
        } else {
            *report.dropped.entry("invalid".into()).or_default() += 1;
        }
    }
    (examples, report)
}

/// Who a voted label came from: `majority:` and the distinct models that voted.
fn voters(teachers: &[&&Sample]) -> String {
    let models: std::collections::BTreeSet<&str> =
        teachers.iter().map(|s| s.model.as_str()).collect();
    crate::clip_bytes(
        &format!(
            "majority:{}",
            models.into_iter().collect::<Vec<_>>().join("+")
        ),
        128,
    )
    .to_string()
}

fn majority(choices: impl Iterator<Item = Choice>, min_share: f32) -> Option<Choice> {
    let mut counts: BTreeMap<String, (Choice, usize)> = BTreeMap::new();
    let mut total = 0;
    for c in choices {
        total += 1;
        counts.entry(format!("{c:?}")).or_insert((c, 0)).1 += 1;
    }
    let (choice, n) = counts.into_values().max_by_key(|(_, n)| *n)?;
    (total > 0 && n as f32 / total as f32 >= min_share && n * 2 > total).then_some(choice)
}

fn vote_lines(teachers: &[&&Sample], min_share: f32) -> Option<Vec<String>> {
    let total = teachers.len();
    if total == 0 {
        return None;
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for s in teachers {
        let lines: std::collections::BTreeSet<String> = s
            .parsed
            .as_ref()
            .and_then(|p| p.delta.clone())
            .unwrap_or_default()
            .into_iter()
            .collect();
        for l in lines {
            *counts.entry(l).or_default() += 1;
        }
    }
    let kept: Vec<String> = counts
        .into_iter()
        .filter(|(_, n)| *n as f32 / total as f32 >= min_share)
        .map(|(l, _)| l)
        .collect();
    let has_change = kept.iter().any(|l| l != "none");
    let kept: Vec<String> = if has_change {
        kept.into_iter().filter(|l| l != "none").collect()
    } else {
        kept
    };
    (!kept.is_empty()).then_some(kept)
}

fn best_composed(
    config: &DistillConfig,
    teachers: &[&&Sample],
    all: &[&Sample],
) -> Option<(Target, Option<String>)> {
    let candidates: Vec<&Sample> = teachers
        .iter()
        .map(|s| **s)
        .filter(|s| s.parsed.as_ref().is_some_and(|p| p.answer.is_some()))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let judged = all.iter().any(|s| s.role == Role::Judge);
    let pick = if judged {
        let mut best: Option<(&Sample, f32)> = None;
        for c in &candidates {
            let key = c.key();
            let scores: Vec<Rubric> = all
                .iter()
                .filter(|j| {
                    j.role == Role::Judge
                        && j.judged.as_deref() == Some(key.as_str())
                        && j.family != c.family
                })
                .filter_map(|j| j.rubric)
                .collect();
            if scores.is_empty() || scores.iter().any(|r| r.safety < config.min_safety) {
                continue;
            }
            let mean = scores.iter().map(|r| r.total() as f32).sum::<f32>() / scores.len() as f32;
            if best.is_none_or(|(_, b)| mean > b) {
                best = Some((c, mean));
            }
        }
        best.map(|(s, _)| s)?
    } else {
        let texts: Vec<&str> = candidates
            .iter()
            .map(|s| {
                s.parsed
                    .as_ref()
                    .and_then(|p| p.answer.as_deref())
                    .unwrap_or("")
            })
            .collect();
        candidates[crate::model::minimum_bayes_risk(&texts)]
    };
    let parsed = pick.parsed.as_ref()?;
    Some((
        Target {
            plan: parsed.plan.clone(),
            text: parsed.answer.clone(),
            ..Default::default()
        },
        Some(pick.model.clone()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Split;

    fn case(task: Task) -> Case {
        Case {
            id: "c1".into(),
            task,
            lang: Lang::Es,
            split: Split::Train,
            adapter: None,
            question: Some("¿Sabe Pedro que el viaje es en julio?".into()),
            options: if task == Task::Decide {
                vec!["sí".into(), "no".into()]
            } else {
                vec![]
            },
            state: Some("Personas: Pedro, Ana".into()),
            chunk: if task == Task::Update {
                Some("[3] Ana: el viaje es en julio".into())
            } else {
                None
            },
            gold: None,
            provenance: Provenance::OwnedProcedural {
                generator: "synth".into(),
                seed: 1,
            },
        }
    }

    fn sample(
        role: Role,
        model: &str,
        family: &str,
        index: usize,
        parsed: Option<Parsed>,
    ) -> Sample {
        Sample {
            case_id: "c1".into(),
            role,
            model: model.into(),
            family: family.into(),
            index,
            judged: None,
            text: String::new(),
            parsed,
            rubric: None,
            completion_tokens: 10,
            seconds: 1.0,
            prompt_tokens: 0,
            cost_micros: 0,
        }
    }

    fn config() -> DistillConfig {
        toml::from_str(
            r#"
            models_dir = "/etc/archie/models"
            [[teachers]]
            model = "nebius.nemotron-3-ultra"
            samples = 3
            "#,
        )
        .unwrap()
    }

    #[test]
    fn parses_answers_through_think_blocks_and_fences() {
        let c = case(Task::Decide);
        let p = parse_answer(
            &c,
            "<think>hmm</think>\n```json\n{\"choice\": 1, \"confidence\": 0.8}\n```",
        )
        .unwrap();
        assert_eq!(p.choice, Some(Choice::Option(1)));
        assert_eq!(
            parse_answer(&c, "{\"choice\": -1}").unwrap().choice,
            Some(Choice::Abstain)
        );
        assert!(parse_answer(&c, "{\"choice\": 2}").is_none());
        assert!(parse_answer(&c, "<think>cut off before answering").is_none());
        let c = case(Task::Compose);
        let p = parse_answer(&c, r#"{"plan": {"stance": "directo", "key": "no lo sabe", "action": "avisar"}, "answer": "Díselo hoy."}"#).unwrap();
        assert_eq!(p.plan.as_deref(), Some("directo | no lo sabe | avisar"));
        let c = case(Task::Update);
        let p = parse_answer(
            &c,
            r#"{"delta": ["fact el viaje = julio @3", "inventado sin formato"]}"#,
        )
        .unwrap();
        assert_eq!(p.delta, Some(vec!["fact el viaje = julio @3".to_string()]));
        assert_eq!(
            parse_rubric(r#"{"tom":9,"evidence":8,"uncertainty":7,"actionable":8,"safety":10}"#)
                .unwrap()
                .total(),
            42
        );
        assert!(parse_rubric(
            r#"{"tom":11,"evidence":8,"uncertainty":7,"actionable":8,"safety":10}"#
        )
        .is_none());
    }

    #[test]
    fn private_shapes_and_benchmark_overlap_are_refused() {
        let mut guard = EvalGuard::new(5);
        let ok = case(Task::Compose);
        assert!(admit_case(&ok, &guard, 2).is_ok());
        let mut leaky = case(Task::Compose);
        leaky.state = Some("Escríbele a pedro@example.com o al +56 9 1234 5678".into());
        assert_eq!(admit_case(&leaky, &guard, 2), Err("sensitive_shape"));
        guard.add_text("Sally puts the marble in the basket and then Anne moves it to the box");
        let mut copied = case(Task::Compose);
        copied.state =
            Some("Sally puts the marble in the basket and then Anne moves it to the box".into());
        assert_eq!(
            admit_case(&copied, &guard, 2),
            Err("overlaps_evaluation_set")
        );
    }

    #[test]
    fn selection_prefers_gold_then_majority_then_judges() {
        let config = config();
        let mut decide = case(Task::Decide);
        let votes = [1usize, 1, 0].map(|c| {
            sample(
                Role::Teacher,
                "t",
                "nvidia",
                c,
                Some(Parsed {
                    choice: Some(Choice::Option(c)),
                    confidence: None,
                    plan: None,
                    answer: None,
                    delta: None,
                }),
            )
        });
        let (ex, report) = select(&config, std::slice::from_ref(&decide), &votes);
        assert_eq!(ex[0].target.choice, Some(Choice::Option(1)));
        assert_eq!(report.from_teachers, 1);
        decide.gold = Some(Target {
            choice: Some(Choice::Option(0)),
            ..Default::default()
        });
        let (ex, report) = select(&config, std::slice::from_ref(&decide), &votes);
        assert_eq!(ex[0].target.choice, Some(Choice::Option(0)));
        assert_eq!(report.from_gold, 1);

        let compose = case(Task::Compose);
        let answer = |a: &str| {
            Some(Parsed {
                choice: None,
                confidence: None,
                plan: Some("p | k | a".into()),
                answer: Some(a.into()),
                delta: None,
            })
        };
        let good = sample(
            Role::Teacher,
            "nebius.nemotron-3-ultra",
            "nvidia",
            0,
            answer("Díselo hoy, con calma."),
        );
        let unsafe_ = sample(
            Role::Teacher,
            "nebius.deepseek",
            "deepseek-ai",
            0,
            answer("Mánipúlalo."),
        );
        let judge = |judged: &Sample, family: &str, safety: u8, total: u8| Sample {
            role: Role::Judge,
            judged: Some(judged.key()),
            family: family.into(),
            rubric: Some(Rubric {
                tom: total,
                evidence: total,
                uncertainty: total,
                actionable: total,
                safety,
            }),
            ..sample(Role::Judge, "j", family, 0, None)
        };
        let draft = sample(
            Role::Draft,
            "local.bonsai",
            "local.bonsai",
            0,
            answer("Creo que no lo sabe."),
        );
        let all = vec![
            good.clone(),
            unsafe_.clone(),
            judge(&good, "qwen", 9, 7),
            judge(&unsafe_, "qwen", 2, 10),
            // A judge of the same family is ignored.
            judge(&good, "nvidia", 0, 0),
            draft,
        ];
        let (ex, report) = select(&config, &[compose], &all);
        assert_eq!(report.with_drafts, 1);
        let selected = ex.iter().find(|e| e.id.ends_with(":selected")).unwrap();
        assert_eq!(
            selected.target.text.as_deref(),
            Some("Díselo hoy, con calma.")
        );
        let drafted = ex.iter().find(|e| e.draft.is_some()).unwrap();
        assert_eq!(drafted.draft.as_deref(), Some("Creo que no lo sabe."));
        assert!(matches!(drafted.provenance, Provenance::Distilled { .. }));
    }

    #[test]
    fn delta_lines_need_to_recur() {
        let config = config();
        let update = case(Task::Update);
        let lines = |l: &[&str]| {
            Some(Parsed {
                choice: None,
                confidence: None,
                plan: None,
                answer: None,
                delta: Some(l.iter().map(|s| s.to_string()).collect()),
            })
        };
        let samples = vec![
            sample(
                Role::Teacher,
                "a",
                "x",
                0,
                lines(&["fact el viaje = julio @3", "affect Ana contenta 60"]),
            ),
            sample(
                Role::Teacher,
                "a",
                "x",
                1,
                lines(&["fact el viaje = julio @3"]),
            ),
            sample(Role::Teacher, "a", "x", 2, lines(&["none"])),
        ];
        let (ex, _) = select(&config, &[update], &samples);
        assert_eq!(
            ex[0].target.text.as_deref(),
            Some("fact el viaje = julio @3")
        );
        assert_eq!(ex[0].target.change, Some(true));
    }

    #[test]
    fn the_shipped_example_config_is_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../training/wanda_lncm/distill.example.toml");
        let config = DistillConfig::load(&path).unwrap();
        assert!(config.models_dir.ends_with("models"));
        assert_eq!(config.teachers[0].model, "nebius.nemotron-3-ultra");
        let registry = ModelRegistry::load_from_dir(&config.models_dir).unwrap();
        for u in config
            .teachers
            .iter()
            .chain(&config.judges)
            .chain(&config.drafts)
        {
            let id = ModelId::parse(u.model.clone()).unwrap();
            assert!(
                registry.get(&id).is_some(),
                "{} is in the registry",
                u.model
            );
        }
    }

    #[test]
    fn config_rejects_nonsense() {
        let mut c = config();
        c.validate().unwrap();
        c.teachers[0].samples = 0;
        assert!(c.validate().is_err());
        let mut c = config();
        c.teachers.clear();
        assert!(c.validate().is_err());
    }
}
