//! Text to token ids and back.
//!
//! Two codecs share one id layout: the 256 byte values first, then learned merges (byte-pair
//! encoding, trained here in Rust over owned text), then sixteen special tokens. The byte codec
//! has no merges and is exact for any input; the BPE codec makes prompts about three times
//! shorter, which is most of the model's latency.
//!
//! Special tokens are never parsed out of text. A message that contains `<eos>` or `<task:...>`
//! is encoded as the bytes it is, so nothing a person writes can steer the prompt's structure.

use crate::{clip_bytes, Lang, LncmError, Result, Task};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::Path;

pub const BYTE_TOKENS: u32 = 256;
pub const SPECIAL_COUNT: u32 = 16;
/// Upper bound on learned merges. A vocabulary past this would only make the embedding table
/// and the reconstruction head larger than the model they feed.
pub const MAX_MERGES: usize = 65_536;
/// A pre-token longer than this is split, which bounds the cost of encoding any input.
const MAX_PIECE_BYTES: usize = 48;
const CODEC_FORMAT: &str = "archie-wanda-lncm-codec-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Special {
    Pad = 0,
    Bos,
    Eos,
    Sep,
    TaskGate,
    TaskUpdate,
    TaskDecide,
    TaskCompose,
    Question,
    State,
    Draft,
    Chunk,
    Option,
    Plan,
    LangEs,
    LangEn,
}

impl Special {
    pub fn for_task(task: Task) -> Self {
        match task {
            Task::Gate => Special::TaskGate,
            Task::Update => Special::TaskUpdate,
            Task::Decide => Special::TaskDecide,
            Task::Compose => Special::TaskCompose,
        }
    }
    pub fn for_lang(lang: Lang) -> Self {
        match lang {
            Lang::Es => Special::LangEs,
            Lang::En => Special::LangEn,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Codec {
    merges: Vec<(u32, u32)>,
    ranks: HashMap<(u32, u32), u32>,
    pieces: Vec<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodecFile {
    format: String,
    merges: Vec<[u32; 2]>,
}

impl Codec {
    /// One token per byte. Exact, language-neutral, and three times longer than BPE.
    pub fn bytes() -> Self {
        Self::from_merges(Vec::new()).expect("no merges is always valid")
    }

    fn from_merges(merges: Vec<(u32, u32)>) -> Result<Self> {
        if merges.len() > MAX_MERGES {
            return Err(LncmError::Config("too many merges".into()));
        }
        let mut pieces: Vec<Vec<u8>> = (0..BYTE_TOKENS).map(|b| vec![b as u8]).collect();
        let mut ranks = HashMap::with_capacity(merges.len());
        for (rank, &(a, b)) in merges.iter().enumerate() {
            let next = BYTE_TOKENS + rank as u32;
            if a >= next || b >= next || ranks.insert((a, b), rank as u32).is_some() {
                return Err(LncmError::Config(
                    "merge refers to a later token or repeats".into(),
                ));
            }
            let mut piece = pieces[a as usize].clone();
            piece.extend_from_slice(&pieces[b as usize]);
            pieces.push(piece);
        }
        Ok(Self {
            merges,
            ranks,
            pieces,
        })
    }

    /// Learns byte-pair merges until the vocabulary (bytes + merges + specials) reaches
    /// `vocab_size` or no pair occurs `min_frequency` times. Deterministic for the same text.
    pub fn train_bpe<'a>(
        texts: impl IntoIterator<Item = &'a str>,
        vocab_size: usize,
        min_frequency: u64,
    ) -> Result<Self> {
        let floor = (BYTE_TOKENS + SPECIAL_COUNT) as usize;
        if vocab_size < floor || vocab_size - floor > MAX_MERGES {
            return Err(LncmError::Config(format!(
                "BPE vocabulary must be between {floor} and {}",
                floor + MAX_MERGES
            )));
        }
        let target_merges = vocab_size - floor;
        let mut counts: HashMap<Vec<u8>, u64> = HashMap::new();
        for text in texts {
            for piece in pre_tokenize(text) {
                *counts.entry(piece.to_vec()).or_default() += 1;
            }
        }
        let mut entries: Vec<(Vec<u8>, u64)> = counts.into_iter().collect();
        entries.sort();
        let mut words: Vec<(Vec<u32>, i64)> = entries
            .into_iter()
            .map(|(bytes, n)| (bytes.into_iter().map(u32::from).collect(), n as i64))
            .collect();
        let mut pair_counts: HashMap<(u32, u32), i64> = HashMap::new();
        let mut holders: HashMap<(u32, u32), HashSet<usize>> = HashMap::new();
        for (index, (symbols, freq)) in words.iter().enumerate() {
            for pair in symbols.windows(2) {
                let pair = (pair[0], pair[1]);
                *pair_counts.entry(pair).or_default() += freq;
                holders.entry(pair).or_default().insert(index);
            }
        }
        let mut heap: BinaryHeap<(i64, Reverse<(u32, u32)>)> = pair_counts
            .iter()
            .map(|(&pair, &count)| (count, Reverse(pair)))
            .collect();
        let mut merges = Vec::with_capacity(target_merges);
        while merges.len() < target_merges {
            let Some((count, Reverse(pair))) = heap.pop() else {
                break;
            };
            if pair_counts.get(&pair).copied() != Some(count) {
                continue; // a stale entry: the pair's count changed after it was pushed
            }
            if count < min_frequency.max(1) as i64 {
                break;
            }
            let new = BYTE_TOKENS + merges.len() as u32;
            merges.push(pair);
            let mut affected: Vec<usize> = holders
                .remove(&pair)
                .unwrap_or_default()
                .into_iter()
                .collect();
            affected.sort_unstable();
            let mut touched: HashSet<(u32, u32)> = HashSet::new();
            for index in affected {
                let (symbols, freq) = &mut words[index];
                for p in symbols.windows(2) {
                    let p = (p[0], p[1]);
                    if let Some(c) = pair_counts.get_mut(&p) {
                        *c -= *freq;
                    }
                    touched.insert(p);
                }
                *symbols = merge_pair(symbols, pair, new);
                for p in symbols.windows(2) {
                    let p = (p[0], p[1]);
                    *pair_counts.entry(p).or_default() += *freq;
                    holders.entry(p).or_default().insert(index);
                    touched.insert(p);
                }
            }
            pair_counts.remove(&pair);
            let mut touched: Vec<_> = touched.into_iter().collect();
            touched.sort_unstable();
            for p in touched {
                match pair_counts.get(&p).copied() {
                    Some(c) if c > 0 => heap.push((c, Reverse(p))),
                    Some(_) => {
                        pair_counts.remove(&p);
                    }
                    None => {}
                }
            }
        }
        Self::from_merges(merges)
    }

    pub fn merges(&self) -> usize {
        self.merges.len()
    }
    /// The first special token's id.
    pub fn base(&self) -> u32 {
        BYTE_TOKENS + self.merges.len() as u32
    }
    pub fn vocab_size(&self) -> usize {
        (self.base() + SPECIAL_COUNT) as usize
    }
    pub fn special(&self, special: Special) -> u32 {
        self.base() + special as u32
    }
    pub fn is_special(&self, id: u32) -> bool {
        id >= self.base() && id < self.base() + SPECIAL_COUNT
    }

    /// Encodes text. Never produces a special token.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let mut out = Vec::with_capacity(text.len() / 2 + 1);
        for piece in pre_tokenize(text) {
            self.encode_piece(piece, &mut out);
        }
        out
    }

    fn encode_piece(&self, piece: &[u8], out: &mut Vec<u32>) {
        let mut symbols: Vec<u32> = piece.iter().map(|&b| u32::from(b)).collect();
        if self.ranks.is_empty() {
            out.extend(symbols);
            return;
        }
        loop {
            let best = symbols
                .windows(2)
                .filter_map(|w| self.ranks.get(&(w[0], w[1])).map(|&r| (r, (w[0], w[1]))))
                .min();
            let Some((rank, pair)) = best else { break };
            symbols = merge_pair(&symbols, pair, BYTE_TOKENS + rank);
        }
        out.extend(symbols);
    }

    /// Decodes ids up to the first end-of-sequence, skipping every other special token.
    /// Invalid UTF-8 (a model can emit half a character) is dropped, not replaced.
    pub fn decode(&self, ids: &[u32]) -> String {
        let eos = self.special(Special::Eos);
        let mut bytes = Vec::new();
        for &id in ids {
            if id == eos {
                break;
            }
            if let Some(piece) = self.pieces.get(id as usize) {
                bytes.extend_from_slice(piece);
            }
        }
        String::from_utf8_lossy(&bytes).replace('\u{FFFD}', "")
    }

    /// A generation target: `[<plan> plan <sep>] body <eos> <pad>...`, exactly `len` ids. The
    /// body is cut on a token boundary so the end marker always fits.
    pub fn encode_target(&self, plan: Option<&str>, body: &str, len: usize) -> Result<Vec<u32>> {
        if len < 4 {
            return Err(LncmError::Config("target length below 4".into()));
        }
        let mut ids = Vec::with_capacity(len);
        if let Some(plan) = plan.filter(|p| !p.trim().is_empty()) {
            ids.push(self.special(Special::Plan));
            let mut plan_ids = self.encode(plan);
            plan_ids.truncate(len / 4);
            ids.extend(plan_ids);
            ids.push(self.special(Special::Sep));
        }
        let room = len.saturating_sub(ids.len() + 1);
        let mut body_ids = self.encode(body);
        body_ids.truncate(room);
        ids.extend(body_ids);
        ids.push(self.special(Special::Eos));
        ids.resize(len, self.special(Special::Pad));
        Ok(ids)
    }

    /// Splits a decoded target into its plan (if any) and its body.
    pub fn decode_target(&self, ids: &[u32]) -> (Option<String>, String) {
        let plan = self.special(Special::Plan);
        let sep = self.special(Special::Sep);
        let eos = self.special(Special::Eos);
        let end = ids.iter().position(|&id| id == eos).unwrap_or(ids.len());
        let ids = &ids[..end];
        if ids.first() == Some(&plan) {
            if let Some(split) = ids.iter().position(|&id| id == sep) {
                let plan_text = self.decode(&ids[1..split]).trim().to_string();
                let body = self.decode(&ids[split + 1..]).trim().to_string();
                return ((!plan_text.is_empty()).then_some(plan_text), body);
            }
            return (None, self.decode(&ids[1..]).trim().to_string());
        }
        (None, self.decode(ids).trim().to_string())
    }

    /// Builds one prompt. Layout:
    /// `<bos> <task> <lang> [<question> q] [<option> o]* [<state> s] [<chunk> c] [<draft> d] <eos>`.
    ///
    /// The question and the options are kept whole up to their caps; what room is left is
    /// shared between state, chunk and draft so that a short one gives its share to the others.
    /// Each is cut at its end: the state is rendered most relevant first and a chunk in order.
    pub fn build_prompt(
        &self,
        parts: &PromptParts<'_>,
        max_len: usize,
        option_len: usize,
    ) -> Result<Prompt> {
        let mut ids = vec![
            self.special(Special::Bos),
            self.special(Special::for_task(parts.task)),
            self.special(Special::for_lang(parts.lang)),
        ];
        let mut option_spans = Vec::with_capacity(parts.options.len());
        if let Some(question) = parts.question {
            ids.push(self.special(Special::Question));
            let mut q = self.encode(question);
            q.truncate((max_len / 4).max(16));
            ids.extend(q);
        }
        for option in parts.options {
            let start = ids.len();
            ids.push(self.special(Special::Option));
            let mut o = self.encode(option);
            o.truncate(option_len.saturating_sub(1).max(1));
            ids.extend(o);
            option_spans.push((start, ids.len()));
        }
        let rest: Vec<(Special, Vec<u32>)> = [
            (Special::State, parts.state),
            (Special::Chunk, parts.chunk),
            (Special::Draft, parts.draft),
        ]
        .into_iter()
        .filter_map(|(marker, text)| {
            text.filter(|t| !t.trim().is_empty())
                .map(|t| (marker, self.encode(t)))
        })
        .collect();
        let fixed = ids.len() + 1 + rest.len();
        if fixed > max_len {
            return Err(LncmError::Input("prompt_budget_too_small"));
        }
        // A chunk is read whole when it fits: delta lines name its turns, so a cut chunk would
        // teach the model to cite turns it cannot see. State and draft share what is left.
        let budget = max_len - fixed;
        let chunk_need = rest
            .iter()
            .find(|(m, _)| *m == Special::Chunk)
            .map_or(0, |(_, t)| t.len().min(budget));
        let others: Vec<usize> = rest
            .iter()
            .filter(|(m, _)| *m != Special::Chunk)
            .map(|(_, t)| t.len())
            .collect();
        let mut other_shares = water_fill(&others, budget - chunk_need).into_iter();
        let shares: Vec<usize> = rest
            .iter()
            .map(|(m, _)| {
                if *m == Special::Chunk {
                    chunk_need
                } else {
                    other_shares.next().unwrap_or(0)
                }
            })
            .collect();
        for ((marker, mut text), share) in rest.into_iter().zip(shares) {
            ids.push(self.special(marker));
            text.truncate(share);
            ids.extend(text);
        }
        ids.push(self.special(Special::Eos));
        Ok(Prompt { ids, option_spans })
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&CodecFile {
            format: CODEC_FORMAT.into(),
            merges: self.merges.iter().map(|&(a, b)| [a, b]).collect(),
        })?)
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let file: CodecFile = serde_json::from_str(text)?;
        if file.format != CODEC_FORMAT {
            return Err(LncmError::Checkpoint("unknown codec format".into()));
        }
        Self::from_merges(file.merges.into_iter().map(|[a, b]| (a, b)).collect())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = read_bounded(path, 16 * 1024 * 1024)?;
        Self::from_json(&text)
    }
}

/// What a prompt is built from. Every field is optional except the task and language.
#[derive(Debug, Clone, Copy)]
pub struct PromptParts<'a> {
    pub task: Task,
    pub lang: Lang,
    pub question: Option<&'a str>,
    pub options: &'a [String],
    pub state: Option<&'a str>,
    pub chunk: Option<&'a str>,
    pub draft: Option<&'a str>,
}

impl<'a> PromptParts<'a> {
    pub fn new(task: Task, lang: Lang) -> Self {
        Self {
            task,
            lang,
            question: None,
            options: &[],
            state: None,
            chunk: None,
            draft: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub ids: Vec<u32>,
    /// `[start, end)` of each option, marker included, so a span is never empty.
    pub option_spans: Vec<(usize, usize)>,
}

/// Splits `budget` among requests so that none gets more than it asked for and what a small
/// request leaves goes to the larger ones. Order is preserved.
fn water_fill(needs: &[usize], budget: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..needs.len()).collect();
    order.sort_by_key(|&i| (needs[i], i));
    let mut out = vec![0; needs.len()];
    let mut left = budget;
    for (served, &i) in order.iter().enumerate() {
        let remaining = needs.len() - served;
        let share = (left / remaining).min(needs[i]);
        out[i] = share;
        left -= share;
    }
    out
}

fn merge_pair(symbols: &[u32], pair: (u32, u32), new: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(symbols.len());
    let mut i = 0;
    while i < symbols.len() {
        if i + 1 < symbols.len() && symbols[i] == pair.0 && symbols[i + 1] == pair.1 {
            out.push(new);
            i += 2;
        } else {
            out.push(symbols[i]);
            i += 1;
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Other,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() {
        Class::Word
    } else {
        Class::Other
    }
}

/// Splits text into pre-tokens: runs of letters and digits, runs of other symbols, each
/// carrying the whitespace before it. Merges never cross a pre-token, so a name keeps the
/// same tokens wherever it appears after a space.
pub(crate) fn pre_tokenize(text: &str) -> Vec<&[u8]> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut previous: Option<Class> = None;
    for (index, c) in text.char_indices() {
        let current = class(c);
        // Whitespace attaches to what follows it; anything else breaks at a change of class.
        let boundary = match previous {
            None | Some(Class::Space) => false,
            Some(prev) => current == Class::Space || current != prev,
        };
        if boundary && index > start {
            push_piece(&bytes[start..index], &mut out);
            start = index;
        }
        previous = Some(current);
    }
    if start < bytes.len() {
        push_piece(&bytes[start..], &mut out);
    }
    out
}

fn push_piece<'a>(mut piece: &'a [u8], out: &mut Vec<&'a [u8]>) {
    while piece.len() > MAX_PIECE_BYTES {
        // Cut on a UTF-8 boundary: never before a continuation byte.
        let mut cut = MAX_PIECE_BYTES;
        while cut > 1 && (piece[cut] & 0b1100_0000) == 0b1000_0000 {
            cut -= 1;
        }
        out.push(&piece[..cut]);
        piece = &piece[cut..];
    }
    if !piece.is_empty() {
        out.push(piece);
    }
}

pub(crate) fn read_bounded(path: &Path, max: u64) -> Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(max + 1).read_to_string(&mut text)?;
    if text.len() as u64 > max {
        return Err(LncmError::Input("file_too_large"));
    }
    Ok(text)
}

/// Keeps text within `max_bytes` for a prompt segment without splitting a character.
pub fn clip_segment(text: &str, max_bytes: usize) -> &str {
    clip_bytes(text, max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORPUS: &[&str] = &[
        "Pedro no sabe que el viaje es en julio.",
        "Pedro llega tarde otra vez.",
        "Pedro dice que el viaje le parece bien.",
        "Does Alice know the trip is in July?",
        "Alice was not in the chat when the trip moved.",
        "Ñandú, café y acción: ¡hola!",
    ];

    #[test]
    fn byte_codec_round_trips_any_text() {
        let codec = Codec::bytes();
        for text in CORPUS {
            assert_eq!(codec.decode(&codec.encode(text)), *text);
        }
        assert_eq!(codec.vocab_size(), 272);
    }

    #[test]
    fn bpe_round_trips_compresses_and_is_deterministic() {
        let codec = Codec::train_bpe(CORPUS.iter().copied().cycle().take(60), 400, 2).unwrap();
        assert!(codec.merges() > 10);
        let again = Codec::train_bpe(CORPUS.iter().copied().cycle().take(60), 400, 2).unwrap();
        assert_eq!(codec.to_json().unwrap(), again.to_json().unwrap());
        for text in CORPUS
            .iter()
            .chain(["texto nunca visto, con ñ y 😀"].iter())
        {
            let ids = codec.encode(text);
            assert_eq!(codec.decode(&ids), *text);
            assert!(ids.iter().all(|&id| !codec.is_special(id)));
        }
        let text = "Pedro no sabe que el viaje es en julio.";
        assert!(codec.encode(text).len() * 2 < text.len());
        let restored = Codec::from_json(&codec.to_json().unwrap()).unwrap();
        assert_eq!(restored.encode(text), codec.encode(text));
    }

    #[test]
    fn special_markers_in_text_stay_text() {
        let codec = Codec::bytes();
        let ids = codec.encode("<eos><task:compose>");
        assert!(ids.iter().all(|&id| !codec.is_special(id)));
        assert_eq!(codec.decode(&ids), "<eos><task:compose>");
    }

    #[test]
    fn targets_carry_plan_and_body_and_always_end() {
        let codec = Codec::bytes();
        let ids = codec
            .encode_target(
                Some("calma | Pedro no lo sabe | díselo"),
                "Díselo hoy.",
                160,
            )
            .unwrap();
        assert_eq!(ids.len(), 160);
        assert!(ids.contains(&codec.special(Special::Eos)));
        let (plan, body) = codec.decode_target(&ids);
        assert_eq!(plan.as_deref(), Some("calma | Pedro no lo sabe | díselo"));
        assert_eq!(body, "Díselo hoy.");
        let long = "x".repeat(500);
        let ids = codec.encode_target(None, &long, 32).unwrap();
        assert_eq!(ids[31], codec.special(Special::Eos));
    }

    #[test]
    fn prompt_respects_budget_and_marks_options() {
        let codec = Codec::bytes();
        let options = vec!["sí".to_string(), "no".to_string()];
        let state = "s".repeat(400);
        let draft = "d".repeat(50);
        let parts = PromptParts {
            question: Some("¿Sabe Pedro lo del viaje?"),
            options: &options,
            state: Some(&state),
            draft: Some(&draft),
            ..PromptParts::new(Task::Decide, Lang::Es)
        };
        let prompt = codec.build_prompt(&parts, 200, 16).unwrap();
        assert!(prompt.ids.len() <= 200);
        assert_eq!(prompt.option_spans.len(), 2);
        let (start, end) = prompt.option_spans[0];
        assert_eq!(prompt.ids[start], codec.special(Special::Option));
        assert_eq!(codec.decode(&prompt.ids[start + 1..end]), "sí");
        // The short draft is kept whole; the state takes the rest.
        let draft_marker = prompt
            .ids
            .iter()
            .position(|&id| id == codec.special(Special::Draft))
            .unwrap();
        assert_eq!(prompt.ids.len() - draft_marker - 2, 50);
        assert!(codec.build_prompt(&parts, 4, 16).is_err());
    }

    #[test]
    fn a_chunk_is_kept_whole_before_the_state() {
        let codec = Codec::bytes();
        let state = "s".repeat(300);
        let chunk = "c".repeat(120);
        let parts = PromptParts {
            state: Some(&state),
            chunk: Some(&chunk),
            ..PromptParts::new(Task::Update, Lang::Es)
        };
        let prompt = codec.build_prompt(&parts, 200, 16).unwrap();
        assert_eq!(
            prompt
                .ids
                .iter()
                .filter(|&&id| id == u32::from(b'c'))
                .count(),
            120
        );
        assert!(prompt.ids.len() <= 200);
    }

    #[test]
    fn water_fill_gives_small_requests_what_they_ask() {
        assert_eq!(water_fill(&[10, 100, 100], 120), vec![10, 55, 55]);
        assert_eq!(water_fill(&[5, 5], 100), vec![5, 5]);
        assert_eq!(water_fill(&[], 100), Vec::<usize>::new());
    }

    #[test]
    fn pre_tokens_keep_leading_space_and_split_long_runs() {
        let pieces: Vec<String> = pre_tokenize("Hola,  Pedro!")
            .into_iter()
            .map(|p| String::from_utf8(p.to_vec()).unwrap())
            .collect();
        assert_eq!(pieces, vec!["Hola", ",", "  Pedro", "!"]);
        let long = "ñ".repeat(100);
        for piece in pre_tokenize(&long) {
            assert!(piece.len() <= MAX_PIECE_BYTES);
            assert!(std::str::from_utf8(piece).is_ok());
        }
    }
}
