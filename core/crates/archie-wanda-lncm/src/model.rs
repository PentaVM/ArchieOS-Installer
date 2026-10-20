//! A trained LNCM, as the runtime uses it.
//!
//! A checkpoint is a directory:
//!
//! ```text
//! config.toml            the model's shape
//! codec.json             the text codec
//! vq.safetensors         tokenizer weights
//! generator.safetensors  generator and head weights
//! lora/<name>.safetensors adapters, one per context domain
//! manifest.json          SHA-256 of every file, what trained it, how it measured
//! ```
//!
//! Opening a checkpoint checks every file against the manifest, and an adapter against the
//! generator it was trained on: weights that do not match what was measured are never served.

use crate::codec::{Codec, PromptParts};
use crate::config::LncmConfig;
use crate::generator::{Generator, PromptCache, Sampling};
use crate::nn;
use crate::vq::VqVae;
use crate::{Lang, LncmError, Result, Task};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use rand::SeedableRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const MANIFEST_FORMAT: u16 = 1;
pub const CONFIG_FILE: &str = "config.toml";
pub const CODEC_FILE: &str = "codec.json";
pub const VQ_FILE: &str = "vq.safetensors";
pub const GENERATOR_FILE: &str = "generator.safetensors";
pub const MANIFEST_FILE: &str = "manifest.json";
/// The largest weights file a checkpoint may hold.
const MAX_WEIGHTS_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageRecord {
    pub stage: String,
    pub steps: u64,
    /// SHA-256 of the training data file(s), so a measurement can be traced to its data.
    pub data_sha256: Vec<String>,
    pub metrics: BTreeMap<String, f64>,
    pub finished_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterRecord {
    pub file: String,
    pub sha256: String,
    /// SHA-256 of the generator weights the adapter was trained on.
    pub base_sha256: String,
    pub steps: u64,
    pub metrics: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u16,
    /// Relative path → SHA-256, for the config, codec, tokenizer and generator.
    pub files: BTreeMap<String, String>,
    pub stages: Vec<StageRecord>,
    pub adapters: BTreeMap<String, AdapterRecord>,
}

impl Manifest {
    pub fn new() -> Self {
        Self {
            format: MANIFEST_FORMAT,
            files: BTreeMap::new(),
            stages: Vec::new(),
            adapters: BTreeMap::new(),
        }
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(MANIFEST_FILE);
        if !path.exists() {
            return Ok(Self::new());
        }
        let manifest: Self =
            serde_json::from_str(&crate::codec::read_bounded(&path, 4 * 1024 * 1024)?)?;
        if manifest.format != MANIFEST_FORMAT {
            return Err(LncmError::Checkpoint("unknown manifest format".into()));
        }
        Ok(manifest)
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        write_atomic(
            &dir.join(MANIFEST_FILE),
            serde_json::to_string_pretty(self)?.as_bytes(),
        )
    }

    /// Records the current digest of `file` (relative to `dir`).
    pub fn record(&mut self, dir: &Path, file: &str) -> Result<String> {
        let digest = file_sha256(&dir.join(file))?;
        self.files.insert(file.to_string(), digest.clone());
        Ok(digest)
    }

    pub fn verify(&self, dir: &Path, file: &str) -> Result<String> {
        let expected = self
            .files
            .get(file)
            .ok_or_else(|| LncmError::Checkpoint(format!("{file} is not in the manifest")))?;
        let actual = file_sha256(&dir.join(file))?;
        if &actual != expected {
            return Err(LncmError::Checkpoint(format!(
                "{file} does not match its digest"
            )));
        }
        Ok(actual)
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self::new()
    }
}

pub fn file_sha256(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|_| LncmError::Checkpoint(format!("cannot open {}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Writes through a temporary file and a rename, so a reader never sees half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Loads a safetensors file of at most [`MAX_WEIGHTS_BYTES`].
pub fn load_tensors(path: &Path, device: &Device) -> Result<HashMap<String, Tensor>> {
    let size = std::fs::metadata(path)
        .map_err(|_| LncmError::Checkpoint(format!("cannot read {}", path.display())))?
        .len();
    if size > MAX_WEIGHTS_BYTES {
        return Err(LncmError::Checkpoint("weights file too large".into()));
    }
    Ok(candle_core::safetensors::load(path, device)?)
}

/// What state 1 decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    /// One probability per option given, in order.
    pub probabilities: Vec<f32>,
    /// Probability that the question cannot be answered from what is known.
    pub abstain: f32,
    /// The chosen option, or `None` when "cannot know" is the most likely answer.
    pub choice: Option<usize>,
    /// Probability of what was chosen (an option or abstaining).
    pub confidence: f32,
}

/// State 1's answer to a yes/no question, ready to be spoken.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuickAnswer {
    pub text: String,
    /// `Some(true)` yes, `Some(false)` no, `None` cannot know.
    pub answer: Option<bool>,
    pub confidence: f32,
}

/// State 2's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Composition {
    pub plan: Option<String>,
    pub answer: String,
    /// Names the guard corrected to the person in the state they were closest to.
    pub corrected_names: Vec<(String, String)>,
    /// Capitalised words, not at the start of a sentence, that are no one in the state and
    /// appear nowhere in the context. A caller may prefer the draft when this is not empty.
    pub unverified_names: Vec<String>,
    pub seconds: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct ComposeOptions {
    pub sampling: Sampling,
    /// More than one draws that many candidates and keeps the one that agrees most with the
    /// others (minimum Bayes risk over character trigrams).
    pub candidates: usize,
    pub seed: u64,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        Self {
            sampling: Sampling::GREEDY,
            candidates: 1,
            seed: 0,
        }
    }
}

/// Gate pass output.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Gate {
    /// Probability that the question is for Wanda.
    pub social: f32,
    /// Probability that it is a yes/no question.
    pub yes_no: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateOutput {
    pub change: f32,
    /// Delta lines, when the change head passed the threshold.
    pub delta: Option<String>,
}

#[derive(Clone)]
pub struct Lncm {
    config: LncmConfig,
    codec: Codec,
    vq: VqVae,
    generator: Generator,
    device: Device,
    dir: Option<PathBuf>,
    generator_sha256: String,
    adapter: Option<String>,
}

impl Lncm {
    /// Opens a checkpoint, verifying every file against the manifest.
    pub fn load(dir: &Path, device: &Device) -> Result<Self> {
        let manifest = Manifest::load(dir)?;
        for file in [CONFIG_FILE, CODEC_FILE, VQ_FILE, GENERATOR_FILE] {
            manifest.verify(dir, file)?;
        }
        let config = LncmConfig::from_toml(&crate::codec::read_bounded(
            &dir.join(CONFIG_FILE),
            1 << 20,
        )?)?;
        let codec = Codec::load(&dir.join(CODEC_FILE))?;
        if codec.vocab_size() != config.vocab_size {
            return Err(LncmError::Checkpoint(
                "codec and config disagree on the vocabulary".into(),
            ));
        }
        let mut tensors = load_tensors(&dir.join(VQ_FILE), device)?;
        tensors.extend(load_tensors(&dir.join(GENERATOR_FILE), device)?);
        let vb = VarBuilder::from_tensors(tensors, DType::F32, device);
        let vq = VqVae::new(&vb, &config)?;
        let generator = Generator::new(&vb, &config)?;
        Ok(Self {
            generator_sha256: manifest.files[GENERATOR_FILE].clone(),
            config,
            codec,
            vq,
            generator,
            device: device.clone(),
            dir: Some(dir.to_path_buf()),
            adapter: None,
        })
    }

    /// Assembles a model from parts already in memory (training and tests).
    pub fn from_parts(
        config: LncmConfig,
        codec: Codec,
        vq: VqVae,
        generator: Generator,
        device: Device,
    ) -> Self {
        Self {
            config,
            codec,
            vq,
            generator,
            device,
            dir: None,
            generator_sha256: String::new(),
            adapter: None,
        }
    }

    /// The same model with the named adapter attached, checked against the manifest and
    /// against the generator it was trained on.
    pub fn with_adapter(&self, name: &str) -> Result<Self> {
        if !adapter_name_valid(name) {
            return Err(LncmError::Input("invalid_adapter_name"));
        }
        let dir = self
            .dir
            .as_ref()
            .ok_or_else(|| LncmError::Checkpoint("model has no checkpoint directory".into()))?;
        let manifest = Manifest::load(dir)?;
        let record = manifest
            .adapters
            .get(name)
            .ok_or_else(|| LncmError::Checkpoint(format!("no adapter {name}")))?;
        if record.base_sha256 != self.generator_sha256 {
            return Err(LncmError::Checkpoint(format!(
                "adapter {name} was trained on other generator weights"
            )));
        }
        let path = dir.join(&record.file);
        if file_sha256(&path)? != record.sha256 {
            return Err(LncmError::Checkpoint(format!(
                "adapter {name} does not match its digest"
            )));
        }
        let vb =
            VarBuilder::from_tensors(load_tensors(&path, &self.device)?, DType::F32, &self.device);
        let mut out = self.clone();
        out.generator.detach_lora();
        out.generator.attach_lora(&vb, &self.config.lora)?;
        out.adapter = Some(name.to_string());
        Ok(out)
    }

    pub fn config(&self) -> &LncmConfig {
        &self.config
    }
    pub fn codec(&self) -> &Codec {
        &self.codec
    }
    pub fn device(&self) -> &Device {
        &self.device
    }
    pub fn adapter(&self) -> Option<&str> {
        self.adapter.as_deref()
    }
    pub fn parts(&self) -> (&VqVae, &Generator) {
        (&self.vq, &self.generator)
    }

    fn prompt(&self, parts: &PromptParts<'_>) -> Result<crate::codec::Prompt> {
        if parts.options.len() > self.config.max_options {
            return Err(LncmError::Input("too_many_options"));
        }
        self.codec
            .build_prompt(parts, self.config.prompt_len, self.config.option_len)
    }

    fn cache(&self, ids: &[u32]) -> Result<PromptCache> {
        Ok(self.generator.encode_prompt(&self.vq, ids, &self.device)?)
    }

    /// Is the question for Wanda, and is it a yes/no question.
    pub fn gate(&self, lang: Lang, question: &str) -> Result<Gate> {
        let parts = PromptParts {
            question: Some(question),
            ..PromptParts::new(Task::Gate, lang)
        };
        let prompt = self.prompt(&parts)?;
        let heads = self
            .generator
            .cached_heads(&self.cache(&prompt.ids)?, &[])?;
        let social = sigmoid(heads.gate.to_vec1::<f32>()?[0]);
        let kind = candle_nn::ops::softmax(&heads.kind, 1)?.to_vec2::<f32>()?;
        Ok(Gate {
            social,
            yes_no: kind[0][1],
        })
    }

    /// State 1 over explicit options.
    pub fn decide(
        &self,
        lang: Lang,
        question: &str,
        state: Option<&str>,
        draft: Option<&str>,
        options: &[String],
    ) -> Result<Decision> {
        if options.is_empty() || options.iter().any(|o| o.trim().is_empty()) {
            return Err(LncmError::Input("options_required"));
        }
        let parts = PromptParts {
            question: Some(question),
            options,
            state,
            draft,
            ..PromptParts::new(Task::Decide, lang)
        };
        let prompt = self.prompt(&parts)?;
        let heads = self
            .generator
            .cached_heads(&self.cache(&prompt.ids)?, &prompt.option_spans)?;
        let logits = (heads.decision / f64::from(self.config.decision_temperature))?;
        let probs = candle_nn::ops::softmax(&logits, 1)?
            .to_vec2::<f32>()?
            .remove(0);
        Ok(decision_from(
            &probs,
            options.len(),
            self.config.max_options,
        ))
    }

    /// State 1 for a question without options: answered only when it is a yes/no question.
    pub fn quick(
        &self,
        lang: Lang,
        question: &str,
        state: Option<&str>,
        draft: Option<&str>,
    ) -> Result<Option<QuickAnswer>> {
        if self.gate(lang, question)?.yes_no < 0.5 {
            return Ok(None);
        }
        let options = vec![lang.yes().to_string(), lang.no().to_string()];
        let decision = self.decide(lang, question, state, draft, &options)?;
        let (answer, text) = match decision.choice {
            Some(0) => (Some(true), capitalise(lang.yes())),
            Some(_) => (Some(false), capitalise(lang.no())),
            None => (
                None,
                match lang {
                    Lang::Es => "Con lo que sé no puedo saberlo.".to_string(),
                    Lang::En => "From what I know, I can't tell.".to_string(),
                },
            ),
        };
        Ok(Some(QuickAnswer {
            text,
            answer,
            confidence: decision.confidence,
        }))
    }

    /// State 2: plan and words. `roster` is the people in the state, for the name guard.
    pub fn compose(
        &self,
        lang: Lang,
        question: &str,
        state: Option<&str>,
        draft: Option<&str>,
        roster: &[String],
        options: ComposeOptions,
    ) -> Result<Composition> {
        let started = Instant::now();
        let parts = PromptParts {
            question: Some(question),
            state,
            draft,
            ..PromptParts::new(Task::Compose, lang)
        };
        let prompt = self.prompt(&parts)?;
        let cache = self.cache(&prompt.ids)?;
        let mut rng = rand::rngs::StdRng::seed_from_u64(options.seed);
        let count = options.candidates.clamp(1, 16);
        let mut candidates = Vec::with_capacity(count);
        for i in 0..count {
            let sampling = if i == 0 && count > 1 {
                Sampling::GREEDY
            } else {
                options.sampling
            };
            let ids = self
                .generator
                .generate(&self.vq, &cache, sampling, &mut rng)?;
            candidates.push(self.codec.decode_target(&ids));
        }
        let best = minimum_bayes_risk(
            &candidates
                .iter()
                .map(|(_, a)| a.as_str())
                .collect::<Vec<_>>(),
        );
        let (plan, answer) = candidates.swap_remove(best);
        let plan = plan.map(|p| clean(&p)).filter(|p| !p.is_empty());
        let answer = clean(&answer);
        let context = [question, state.unwrap_or(""), draft.unwrap_or("")].join("\n");
        let guarded = guard_names(&answer, roster, &context);
        Ok(Composition {
            plan,
            answer: guarded.text,
            corrected_names: guarded.corrected,
            unverified_names: guarded.unverified,
            seconds: started.elapsed().as_secs_f32(),
        })
    }

    /// For the reader: does this chunk change the state, and if so, the delta lines.
    pub fn update(
        &self,
        lang: Lang,
        state: &str,
        chunk: &str,
        threshold: f32,
    ) -> Result<UpdateOutput> {
        let parts = PromptParts {
            state: Some(state),
            chunk: Some(chunk),
            ..PromptParts::new(Task::Update, lang)
        };
        let prompt = self.prompt(&parts)?;
        let cache = self.cache(&prompt.ids)?;
        let heads = self.generator.cached_heads(&cache, &[])?;
        let change = sigmoid(heads.change.to_vec1::<f32>()?[0]);
        if change < threshold {
            return Ok(UpdateOutput {
                change,
                delta: None,
            });
        }
        let mut rng = rand::rngs::StdRng::seed_from_u64(0);
        let ids = self
            .generator
            .generate(&self.vq, &cache, Sampling::GREEDY, &mut rng)?;
        let (_, body) = self.codec.decode_target(&ids);
        Ok(UpdateOutput {
            change,
            delta: Some(clean(&body)),
        })
    }

    /// Reconstructs a text through the tokenizer alone, for checking the tokenizer.
    pub fn round_trip(&self, text: &str) -> Result<String> {
        let ids = self
            .codec
            .encode_target(None, text, self.config.target_len)?;
        let tensor = nn::ids_tensor(&[ids], &self.device)?;
        let z = self.vq.encode(&tensor, nn::Ctx::EVAL)?;
        let q = self.vq.quantize(&z, self.config.levels.len())?;
        let ids = self.vq.tokens(&q.full)?;
        Ok(self.codec.decode_target(&ids).1)
    }
}

/// Generated text without control characters (a model can emit any byte), trimmed.
pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn adapter_name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub(crate) fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// Turns probabilities over `[options…, padding…, abstain]` into a decision.
pub(crate) fn decision_from(probs: &[f32], options: usize, max_options: usize) -> Decision {
    let probabilities = probs[..options].to_vec();
    let abstain = probs[max_options];
    let (best, best_p) =
        probabilities
            .iter()
            .copied()
            .enumerate()
            .fold(
                (0, f32::NEG_INFINITY),
                |acc, (i, p)| if p > acc.1 { (i, p) } else { acc },
            );
    if abstain >= best_p {
        Decision {
            probabilities,
            abstain,
            choice: None,
            confidence: abstain,
        }
    } else {
        Decision {
            probabilities,
            abstain,
            choice: Some(best),
            confidence: best_p,
        }
    }
}

/// Character trigram F1 between two texts.
pub fn char_ngram_f1(a: &str, b: &str) -> f32 {
    fn grams(s: &str) -> HashMap<[char; 3], u32> {
        let chars: Vec<char> = s.to_lowercase().chars().collect();
        let mut out = HashMap::new();
        for w in chars.windows(3) {
            *out.entry([w[0], w[1], w[2]]).or_default() += 1;
        }
        out
    }
    let (ga, gb) = (grams(a), grams(b));
    let total_a: u32 = ga.values().sum();
    let total_b: u32 = gb.values().sum();
    if total_a == 0 || total_b == 0 {
        return if a.trim() == b.trim() { 1.0 } else { 0.0 };
    }
    let overlap: u32 = ga
        .iter()
        .map(|(g, &n)| n.min(*gb.get(g).unwrap_or(&0)))
        .sum();
    if overlap == 0 {
        return 0.0;
    }
    let p = overlap as f32 / total_a as f32;
    let r = overlap as f32 / total_b as f32;
    2.0 * p * r / (p + r)
}

/// Index of the candidate with the highest mean agreement with the others.
pub fn minimum_bayes_risk(candidates: &[&str]) -> usize {
    if candidates.len() <= 1 {
        return 0;
    }
    let mut best = (0, f32::NEG_INFINITY);
    for (i, a) in candidates.iter().enumerate() {
        let score: f32 = candidates
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, b)| char_ngram_f1(a, b))
            .sum();
        if score > best.1 {
            best = (i, score);
        }
    }
    best.0
}

pub struct Guarded {
    pub text: String,
    pub corrected: Vec<(String, String)>,
    pub unverified: Vec<String>,
}

/// Checks the capitalised words of a generated answer against the people it may name.
///
/// A word that is a person in the roster, or appears in the context, stands. A word one edit
/// (two for longer words; a swap of neighbours counts as one) from exactly one person's name
/// is that name misspelt and is corrected. Any other capitalised word that does not start a
/// sentence is reported as unverified.
pub fn guard_names(text: &str, roster: &[String], context: &str) -> Guarded {
    let names: BTreeSet<String> = roster
        .iter()
        .flat_map(|n| n.split_whitespace().map(str::to_string))
        .filter(|n| n.chars().next().is_some_and(char::is_uppercase))
        .collect();
    let context_words: BTreeSet<&str> = context
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = String::with_capacity(text.len());
    let mut corrected = Vec::new();
    let mut unverified: Vec<String> = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if !c.is_alphanumeric() {
            out.push(c);
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && chars[j].1.is_alphanumeric() {
            j += 1;
        }
        let end = chars.get(j).map_or(text.len(), |&(at, _)| at);
        let word = &text[start..end];
        i = j;
        let capital = word.chars().next().is_some_and(char::is_uppercase);
        if !capital || names.contains(word) || context_words.contains(word) {
            out.push_str(word);
            continue;
        }
        let lower = word.to_lowercase();
        let budget = if word.chars().count() <= 5 { 1 } else { 2 };
        let close: Vec<&String> = names
            .iter()
            .filter(|n| edit_distance(&n.to_lowercase(), &lower) <= budget)
            .collect();
        if close.len() == 1 {
            corrected.push((word.to_string(), close[0].clone()));
            out.push_str(close[0]);
            continue;
        }
        if !sentence_initial(&text[..start]) && !unverified.iter().any(|w| w == word) {
            unverified.push(word.to_string());
        }
        out.push_str(word);
    }
    Guarded {
        text: out,
        corrected,
        unverified,
    }
}

/// Whether a word that follows `before` starts a sentence.
fn sentence_initial(before: &str) -> bool {
    for c in before.chars().rev() {
        if c == '\n' {
            return true;
        }
        if c.is_whitespace() || matches!(c, '"' | '\'' | '«' | '“' | '(' | '—' | '-') {
            continue;
        }
        return matches!(c, '.' | '!' | '?' | '¡' | '¿' | ':' | '…');
    }
    true
}

/// Edit distance where inserting, deleting, replacing or swapping two neighbours each cost one.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_prefers_abstain_on_ties_and_reports_confidence() {
        let d = decision_from(&[0.2, 0.1, 0.0, 0.0, 0.7], 2, 4);
        assert_eq!(d.choice, None);
        assert!((d.confidence - 0.7).abs() < 1e-6);
        let d = decision_from(&[0.6, 0.1, 0.0, 0.0, 0.3], 2, 4);
        assert_eq!(d.choice, Some(0));
        assert_eq!(d.probabilities.len(), 2);
    }

    #[test]
    fn names_are_corrected_or_flagged() {
        let roster = vec!["Pedro".to_string(), "Ana María".to_string()];
        let g = guard_names(
            "Dile a Pedor que María ya lo sabe y que Rodrigo no.",
            &roster,
            "¿Sabe Pedro lo del viaje?",
        );
        assert_eq!(
            g.text,
            "Dile a Pedro que María ya lo sabe y que Rodrigo no."
        );
        assert_eq!(
            g.corrected,
            vec![("Pedor".to_string(), "Pedro".to_string())]
        );
        assert_eq!(g.unverified, vec!["Rodrigo".to_string()]);
        // Sentence-initial words are ordinary words, not names.
        let g = guard_names("Díselo hoy. Mejor así.", &roster, "");
        assert!(g.unverified.is_empty());
        assert_eq!(g.text, "Díselo hoy. Mejor así.");
    }

    #[test]
    fn mbr_picks_the_consensus() {
        let c = [
            "Pedro no lo sabe todavía.",
            "Pedro no lo sabe aún.",
            "Llueve en Madrid.",
        ];
        assert_ne!(minimum_bayes_risk(&c), 2);
        assert!(char_ngram_f1("hola", "hola") > 0.99);
        assert_eq!(char_ngram_f1("abc", "xyz"), 0.0);
    }

    #[test]
    fn generated_text_is_cleaned() {
        assert_eq!(clean(" hola\u{3}\u{1e} qué\ntal\t "), "hola qué\ntal");
    }

    #[test]
    fn adapter_names_are_slugs() {
        assert!(adapter_name_valid("personal-owner"));
        assert!(!adapter_name_valid("../x"));
        assert!(!adapter_name_valid("Personal"));
        assert!(!adapter_name_valid(""));
    }

    #[test]
    fn edit_distance_counts_a_swap_as_one() {
        assert_eq!(edit_distance("pedro", "pedor"), 1);
        assert_eq!(edit_distance("ana", "ana"), 0);
        assert_eq!(edit_distance("", "ab"), 2);
        assert_eq!(edit_distance("marta", "maria"), 1);
    }
}
