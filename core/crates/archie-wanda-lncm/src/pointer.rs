//! The copy head: what the generator cannot spell, it points at.
//!
//! The next-concept generator writes through codes, and a code holds a meaning, not a
//! spelling: asked for the month someone saw, it writes *a* month. The prompt says which one.
//! This head lets each position of the finest level, which is one token of the answer, point
//! at a token of the prompt (the state, the chunk, the draft, the question) instead of keeping
//! the token its codes decoded to, as pointer-generator and copy-paste language models do:
//!
//! - the keys are the prompt's hidden states, which the prompt pass already computed and the
//!   cache keeps, so pointing costs one product per answer, not one more pass;
//! - a gate per position chooses between copying and keeping;
//! - copies are made as spans: a run of copying positions takes the contiguous stretch of the
//!   prompt its scores agree on, and a value inside the state (after `=` or "now:") is taken
//!   whole, up to its delimiters, so a value is never stitched from two lines.
//!
//! The generator and the tokenizer stay frozen; only this head trains (`train pointer`), on
//! answer positions whose token is a content token the prompt holds.

use crate::codec::{Codec, Special};
use crate::nn::{Lin, MASKED};
use candle_core::{Result, Tensor, D};
use candle_nn::VarBuilder;
use std::collections::BTreeSet;

pub const POINTER_FILE: &str = "pointer.safetensors";
/// Names the generator weights the head was trained on.
pub const POINTER_BASE_FILE: &str = "pointer.json";
/// Prompt positions tried as the start of each copied run.
const STARTS: usize = 8;

#[derive(Clone)]
pub struct PointerHead {
    query: Lin,
    key: Lin,
    gate: Lin,
    scale: f64,
}

impl PointerHead {
    pub fn new(vb: &VarBuilder, d: usize) -> Result<Self> {
        let vb = vb.pp("pointer");
        Ok(Self {
            query: Lin::new(&vb, "query", d, d, false, 0.02, None)?,
            key: Lin::new(&vb, "key", d, d, false, 0.02, None)?,
            gate: Lin::new(&vb, "gate", d, 1, true, 0.02, None)?,
            scale: 1.0 / (d as f64).sqrt(),
        })
    }

    /// Scores of every answer position over every prompt position `(B, L, P)`, masked where a
    /// prompt token may not be copied, and the gate's logits `(B, L)`. `answer` holds the
    /// finest level's hidden states `(B, L, d)`, `prompt` the prompt's `(B, P, d)`, and
    /// `copyable` `(B, P)` is 1 where a prompt token may be copied.
    pub fn scores(
        &self,
        answer: &Tensor,
        prompt: &Tensor,
        copyable: &Tensor,
    ) -> Result<(Tensor, Tensor)> {
        let q = self.query.forward(answer)?;
        let k = self.key.forward(prompt)?;
        let s = (q.matmul(&k.transpose(1, 2)?.contiguous()?)? * self.scale)?;
        let mask = ((copyable.ones_like()? - copyable)? * f64::from(MASKED))?.unsqueeze(1)?;
        let gate = self.gate.forward(answer)?.squeeze(D::Minus1)?;
        Ok((s.broadcast_add(&mask)?, gate))
    }
}

/// 1 where a prompt token may be copied: any text token, never a marker or padding.
pub fn copyable(codec: &Codec, ids: &[u32], width: usize) -> Vec<f32> {
    (0..width)
        .map(|j| match ids.get(j) {
            Some(&id) if !codec.is_special(id) => 1.0,
            _ => 0.0,
        })
        .collect()
}

/// A token worth copying: a word of three letters or more, or one with a digit, and not one
/// of the commonest tokens of the answers. Articles and connectives are the generator's.
pub fn is_content(codec: &Codec, id: u32, common: &BTreeSet<u32>) -> bool {
    if codec.is_special(id) || common.contains(&id) {
        return false;
    }
    let text = codec.decode(&[id]);
    text.chars().any(|c| c.is_ascii_digit())
        || text.chars().filter(|c| c.is_alphanumeric()).count() >= 3
}

/// One row's training targets: per answer position, whether it copies and whether it counts,
/// and the prompt positions that hold its token, `L × width` flattened.
pub struct Labels {
    pub copy: Vec<f32>,
    pub valid: Vec<f32>,
    pub gold: Vec<f32>,
}

pub fn labels(
    codec: &Codec,
    prompt: &[u32],
    width: usize,
    target: &[u32],
    common: &BTreeSet<u32>,
) -> Labels {
    let eos = codec.special(Special::Eos);
    let end = target
        .iter()
        .position(|&t| t == eos)
        .map_or(target.len(), |e| e + 1);
    let mut copy = vec![0.0; target.len()];
    let mut valid = vec![0.0; target.len()];
    let mut gold = vec![0.0; target.len() * width];
    for (i, &t) in target.iter().enumerate().take(end) {
        valid[i] = 1.0;
        if !is_content(codec, t, common) {
            continue;
        }
        let mut found = false;
        for (j, &p) in prompt.iter().enumerate().take(width) {
            if p == t {
                gold[i * width + j] = 1.0;
                found = true;
            }
        }
        if found {
            copy[i] = 1.0;
        }
    }
    Labels { copy, valid, gold }
}

/// `−log Σ_gold softmax(scores)` over copying positions plus the gate's binary cross-entropy
/// over the positions that count. `scores` `(B, L, P)`, `gate` `(B, L)`, `copy` and `valid`
/// `(B, L)`, `gold` `(B, L, P)`.
pub fn loss(
    scores: &Tensor,
    gate: &Tensor,
    copy: &Tensor,
    valid: &Tensor,
    gold: &Tensor,
) -> Result<Tensor> {
    let off_gold = ((gold.ones_like()? - gold)? * f64::from(MASKED))?;
    let picked = (scores + off_gold)?.log_sum_exp(D::Minus1)?;
    let all = scores.log_sum_exp(D::Minus1)?;
    let copies = copy.sum_all()?.maximum(1.0)?;
    let copy_loss = ((all - picked)? * copy)?
        .sum_all()?
        .broadcast_div(&copies)?;
    // softplus(x) − y·x, written stably.
    let bce = ((gate.relu()? - (gate * copy)?)? + (gate.abs()?.neg()?.exp()? + 1.0)?.log()?)?;
    let counted = valid.sum_all()?.maximum(1.0)?;
    let gate_loss = (bce * valid)?.sum_all()?.broadcast_div(&counted)?;
    copy_loss + gate_loss
}

/// What [`apply`] copied: per run, the answer positions it replaced and the prompt span.
#[derive(Debug, Clone, PartialEq)]
pub struct Copied {
    pub ids: Vec<u32>,
    pub runs: Vec<((usize, usize), (usize, usize))>,
}

/// Copies into one answer. `ids` are its token ids as the codes decoded them, `prompt` the
/// prompt's ids, `scores` `L × P` and `gate` `L` this head's outputs for the answer. Every run
/// of copying positions is replaced by the prompt span it points at; the answer keeps its
/// length, cut or padded after its end marker.
pub fn apply(
    codec: &Codec,
    ids: &[u32],
    prompt: &[u32],
    scores: &[Vec<f32>],
    gate: &[f32],
) -> Copied {
    let eos = codec.special(Special::Eos);
    let pad = codec.special(Special::Pad);
    let end = ids.iter().position(|&t| t == eos).unwrap_or(ids.len());
    let copying =
        |i: usize| i < end && !codec.is_special(ids[i]) && gate.get(i).is_some_and(|&g| g > 0.0);
    let state = section(codec, prompt, Special::State);
    let mut out = Vec::with_capacity(ids.len());
    let mut runs = Vec::new();
    let mut i = 0;
    while i < end {
        if !copying(i) {
            out.push(ids[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < end && copying(i) {
            i += 1;
        }
        let Some(span) = best_span(codec, prompt, scores, start, i) else {
            out.extend_from_slice(&ids[start..i]);
            continue;
        };
        let mut span = state
            .and_then(|s| whole_value(codec, prompt, span, s))
            .unwrap_or(span);
        // One space between words: the answer's own, or the copied value's.
        let spaced = out
            .last()
            .is_some_and(|&t| codec.decode(&[t]).ends_with(char::is_whitespace));
        while spaced
            && span.0 < span.1
            && codec.decode(&prompt[span.0..span.0 + 1]).trim().is_empty()
        {
            span.0 += 1;
        }
        out.extend_from_slice(&prompt[span.0..span.1]);
        runs.push(((start, i), span));
    }
    out.push(eos);
    out.resize(ids.len().max(1), pad);
    out.truncate(ids.len().max(1));
    if out.len() == ids.len() && !out.contains(&eos) {
        if let Some(last) = out.last_mut() {
            *last = eos;
        }
    }
    Copied { ids: out, runs }
}

/// `[start, end)` of the text after `marker` in the prompt, up to the next marker.
fn section(codec: &Codec, prompt: &[u32], marker: Special) -> Option<(usize, usize)> {
    let m = codec.special(marker);
    let start = prompt.iter().position(|&t| t == m)? + 1;
    let end = prompt[start..]
        .iter()
        .position(|&t| codec.is_special(t))
        .map_or(prompt.len(), |e| start + e);
    Some((start, end))
}

/// The prompt span a run `[a, b)` agrees on most: of the best few starts for its first
/// position, the one whose contiguous stretch scores highest over the whole run.
fn best_span(
    codec: &Codec,
    prompt: &[u32],
    scores: &[Vec<f32>],
    a: usize,
    b: usize,
) -> Option<(usize, usize)> {
    let first = scores.get(a)?;
    let mut starts: Vec<usize> = (0..first.len().min(prompt.len())).collect();
    starts.sort_by(|&x, &y| first[y].total_cmp(&first[x]));
    starts.truncate(STARTS);
    let len = b - a;
    starts
        .into_iter()
        .filter(|&j| j + len <= prompt.len())
        .filter(|&j| (j..j + len).all(|p| !codec.is_special(prompt[p])))
        .map(|j| {
            let total: f32 = (0..len)
                .map(|t| {
                    scores
                        .get(a + t)
                        .and_then(|row| row.get(j + t))
                        .copied()
                        .unwrap_or(f32::MIN)
                })
                .sum();
            (j, total)
        })
        .filter(|(_, total)| total.is_finite() && *total > f32::from(i16::MIN))
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(j, _)| (j, j + len))
}

/// Inside the state, a span within a value grows to the whole value: back to the `=` or the
/// "now:" before it, forward to the punctuation after it. Anything else is copied as pointed.
fn whole_value(
    codec: &Codec,
    prompt: &[u32],
    span: (usize, usize),
    state: (usize, usize),
) -> Option<(usize, usize)> {
    if span.0 < state.0 || span.1 > state.1 {
        return None;
    }
    let piece = |j: usize| codec.decode(&prompt[j..j + 1]);
    let digit_at = |j: usize| piece(j).chars().any(|c| c.is_ascii_digit());
    let delimiter = |j: usize| {
        let p = piece(j);
        if p.contains(['=', ';', ':', '(', ')', '\n', '—']) || p.trim() == "-" {
            return true;
        }
        // A point or a comma ends a value unless it sits between digits (1.500, 1,500).
        (p.trim() == "." || p.trim() == ",")
            && !(j > state.0 && j + 1 < state.1 && digit_at(j - 1) && digit_at(j + 1))
    };
    let mut start = span.0;
    while start > state.0 && !delimiter(start - 1) {
        start -= 1;
    }
    let mut end = span.1;
    while end < state.1 && !delimiter(end) {
        end += 1;
    }
    if start == state.0 || start == end {
        return None;
    }
    let before = piece(start - 1);
    let lead = codec
        .decode(&prompt[start.saturating_sub(8).max(state.0)..start - 1])
        .to_lowercase();
    let lead = lead.trim_end();
    let after_now = before.contains(':') && (lead.ends_with("ahora") || lead.ends_with("now"));
    (before.contains('=') || after_now).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::PromptParts;
    use crate::{Lang, Task};

    fn codec() -> Codec {
        Codec::bytes()
    }

    #[test]
    fn a_run_takes_the_whole_value_it_points_into() {
        let codec = codec();
        let state =
            "- Ana: vio que el viaje = la vieja biblioteca; no vio el cambio (ahora: la azotea).";
        let prompt = codec
            .build_prompt(
                &PromptParts {
                    state: Some(state),
                    ..PromptParts::new(Task::Compose, Lang::Es)
                },
                256,
                16,
            )
            .unwrap()
            .ids;
        let at = |needle: &str| {
            let n = codec.encode(needle);
            prompt
                .windows(n.len())
                .position(|w| w == n.as_slice())
                .unwrap()
        };
        // The answer "Cree que es Xx." with "Xx" pointing at "vieja".
        let answer = codec.encode_target(None, "Cree que es Xx.", 64).unwrap();
        let xx = codec.encode("Cree que es ").len();
        let mut scores = vec![vec![0.0; prompt.len()]; answer.len()];
        let mut gate = vec![-1.0; answer.len()];
        let vieja = at("vieja");
        for t in 0..2 {
            gate[xx + t] = 1.0;
            scores[xx + t][vieja + t] = 5.0;
        }
        let copied = apply(&codec, &answer, &prompt, &scores, &gate);
        let (_, body) = codec.decode_target(&copied.ids);
        assert_eq!(body, "Cree que es la vieja biblioteca.");
        // After "ahora:" a value is whole too.
        let azotea = at("azotea");
        let mut scores = vec![vec![0.0; prompt.len()]; answer.len()];
        for t in 0..2 {
            scores[xx + t][azotea + t] = 5.0;
        }
        let copied = apply(&codec, &answer, &prompt, &scores, &gate);
        assert_eq!(codec.decode_target(&copied.ids).1, "Cree que es la azotea.");
    }

    #[test]
    fn digits_keep_their_separators_and_nothing_else_moves() {
        let codec = codec();
        let state =
            "- Ana: sabe que el presupuesto = 1.500 euros.\n- Luis: sabe que la cena = lunes.";
        let prompt = codec
            .build_prompt(
                &PromptParts {
                    state: Some(state),
                    ..PromptParts::new(Task::Compose, Lang::Es)
                },
                256,
                16,
            )
            .unwrap()
            .ids;
        let five = prompt
            .windows(3)
            .position(|w| w == codec.encode("500").as_slice())
            .unwrap();
        let answer = codec.encode_target(None, "Son 9 ya.", 24).unwrap();
        let nine = codec.encode("Son ").len();
        let mut scores = vec![vec![0.0; prompt.len()]; answer.len()];
        let mut gate = vec![-1.0; answer.len()];
        gate[nine] = 1.0;
        scores[nine][five] = 5.0;
        let copied = apply(&codec, &answer, &prompt, &scores, &gate);
        assert_eq!(codec.decode_target(&copied.ids).1, "Son 1.500 euros ya.");
        // With the gate closed everywhere the answer is untouched.
        let closed = vec![-1.0; answer.len()];
        assert_eq!(
            apply(&codec, &answer, &prompt, &scores, &closed).ids,
            answer
        );
    }

    #[test]
    fn labels_mark_content_tokens_the_prompt_holds() {
        let codec = codec();
        let prompt = codec.encode("mayo junio");
        let target = codec.encode_target(None, "en mayo", 16).unwrap();
        let common = BTreeSet::new();
        let l = labels(&codec, &prompt, prompt.len(), &target, &common);
        // Byte codec: every byte is a token; digits and letters of "mayo" are in the prompt but
        // single letters are not content, so nothing copies.
        assert!(l.copy.iter().all(|&c| c == 0.0));
        assert_eq!(
            l.valid.iter().filter(|&&v| v == 1.0).count(),
            "en mayo".len() + 1
        );
    }
}
