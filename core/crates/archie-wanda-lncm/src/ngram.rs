//! An n-gram index of the answers the model was trained on, to tell text from noise.
//!
//! A suffix array over every training answer's tokens (infini-gram's structure, at the size
//! of this corpus) counts any n-gram up to [`MAX_N`] tokens by binary search. An answer whose
//! token trigrams the corpus has mostly never seen ("TenTen en cuentaTen en en que") is noise;
//! so is one that loops. The index says nothing about whether an answer is true: that is
//! [`crate::verify`]'s job, against the state.
//!
//! The file also carries the [`Lexicon`] of topics and values the training states held.

use crate::codec::Codec;
use crate::verify::Lexicon;
use crate::{LncmError, Result};
use std::path::Path;

pub const NGRAM_FILE: &str = "answers.ngram";
/// Longest n-gram the index orders by and counts.
pub const MAX_N: usize = 8;
/// Share of unseen trigrams above which an answer is noise.
pub const UNSEEN_LIMIT: f32 = 0.35;
const MAGIC: &[u8; 4] = b"WNG1";
const SEPARATOR: u32 = u32::MAX;
const MAX_TOKENS: usize = 64 * 1024 * 1024;

pub struct NgramIndex {
    tokens: Vec<u32>,
    suffixes: Vec<u32>,
    pub lexicon: Lexicon,
}

/// How an answer reads against the corpus.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Fluency {
    /// Share of the answer's token trigrams no training answer contains.
    pub unseen: f32,
    /// A stretch of one to four tokens repeated three times in a row.
    pub looped: bool,
}

impl Fluency {
    pub fn is_noise(&self) -> bool {
        self.looped || self.unseen > UNSEEN_LIMIT
    }
}

impl NgramIndex {
    /// Indexes `answers` (token ids of each answer's text, no markers).
    pub fn build(answers: &[Vec<u32>], lexicon: Lexicon) -> Self {
        let mut tokens = Vec::new();
        for a in answers {
            tokens.extend(a.iter().copied().filter(|&t| t != SEPARATOR));
            tokens.push(SEPARATOR);
        }
        let mut suffixes: Vec<u32> = (0..tokens.len() as u32)
            .filter(|&i| tokens[i as usize] != SEPARATOR)
            .collect();
        suffixes.sort_unstable_by(|&a, &b| prefix(&tokens, a).cmp(prefix(&tokens, b)));
        Self {
            tokens,
            suffixes,
            lexicon,
        }
    }

    /// How many times `ngram` (at most [`MAX_N`] tokens) occurs in the corpus.
    pub fn count(&self, ngram: &[u32]) -> usize {
        if ngram.is_empty() || ngram.len() > MAX_N {
            return 0;
        }
        let key = |&i: &u32| {
            let p = prefix(&self.tokens, i);
            &p[..p.len().min(ngram.len())]
        };
        let lo = self.suffixes.partition_point(|i| key(i) < ngram);
        let hi = self.suffixes.partition_point(|i| key(i) <= ngram);
        hi - lo
    }

    /// Reads `ids` (an answer's text tokens).
    pub fn fluency(&self, ids: &[u32]) -> Fluency {
        // A stretch of one to four tokens said three times in a row.
        let looped = (1..=4).any(|p| {
            ids.windows(3 * p)
                .any(|w| w[..p] == w[p..2 * p] && w[p..2 * p] == w[2 * p..])
        });
        let trigrams: Vec<&[u32]> = ids.windows(3).collect();
        let unseen = if trigrams.is_empty() {
            0.0
        } else {
            trigrams.iter().filter(|t| self.count(t) == 0).count() as f32 / trigrams.len() as f32
        };
        Fluency { unseen, looped }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let lexicon = serde_json::to_vec(&self.lexicon)?;
        let mut bytes = Vec::with_capacity(16 + 4 * (self.tokens.len() + self.suffixes.len()));
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(self.tokens.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.suffixes.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(lexicon.len() as u32).to_le_bytes());
        for t in self.tokens.iter().chain(&self.suffixes) {
            bytes.extend_from_slice(&t.to_le_bytes());
        }
        bytes.extend_from_slice(&lexicon);
        crate::model::write_atomic(path, &bytes)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let bad = || LncmError::Checkpoint(format!("{} is not an n-gram index", path.display()));
        if bytes.len() < 16 || &bytes[..4] != MAGIC {
            return Err(bad());
        }
        let word = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
        let (n, s, l) = (word(4) as usize, word(8) as usize, word(12) as usize);
        if n > MAX_TOKENS || s > n || bytes.len() != 16 + 4 * (n + s) + l {
            return Err(bad());
        }
        let read = |from: usize, count: usize| {
            (0..count).map(|k| word(from + 4 * k)).collect::<Vec<u32>>()
        };
        let tokens = read(16, n);
        let suffixes = read(16 + 4 * n, s);
        if suffixes.iter().any(|&i| i as usize >= n) {
            return Err(bad());
        }
        let lexicon = serde_json::from_slice(&bytes[16 + 4 * (n + s)..])?;
        Ok(Self {
            tokens,
            suffixes,
            lexicon,
        })
    }
}

/// The first [`MAX_N`] tokens from `i`, stopping at an answer's end.
fn prefix(tokens: &[u32], i: u32) -> &[u32] {
    let rest = &tokens[i as usize..];
    let end = rest
        .iter()
        .take(MAX_N)
        .position(|&t| t == SEPARATOR)
        .unwrap_or(rest.len().min(MAX_N));
    &rest[..end]
}

/// The text tokens of a decoded answer, for [`NgramIndex::fluency`].
pub fn text_tokens(codec: &Codec, text: &str) -> Vec<u32> {
    codec.encode(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> (Codec, NgramIndex) {
        let codec = Codec::bytes();
        let answers: Vec<Vec<u32>> = [
            "Ten en cuenta que Ana no vio el cambio.",
            "Ten en cuenta que suele llegar tarde.",
        ]
        .iter()
        .map(|a| codec.encode(a))
        .collect();
        (codec, NgramIndex::build(&answers, Lexicon::default()))
    }

    #[test]
    fn counts_ngrams_and_never_across_answers() {
        let (codec, idx) = index();
        assert_eq!(idx.count(&codec.encode("Ten en")), 2);
        assert_eq!(idx.count(&codec.encode("Ana")), 1);
        assert_eq!(
            idx.count(&codec.encode(".T")),
            0,
            "no n-gram spans two answers"
        );
        assert_eq!(idx.count(&codec.encode("zzz")), 0);
    }

    #[test]
    fn noise_is_told_from_text_and_the_file_round_trips() {
        let (codec, idx) = index();
        let fine = idx.fluency(&codec.encode("Ten en cuenta que Ana suele llegar tarde."));
        assert!(!fine.is_noise(), "{fine:?}");
        // Byte tokens here, so the noise has to be plain; with the real codec's word pieces
        // "TenTen en cuentaTen" is just as unseen.
        let noise = idx.fluency(&codec.encode("qxz wkv jpb"));
        assert!(noise.is_noise(), "{noise:?}");
        let looped = idx.fluency(&codec.encode("sin sin sin sin"));
        assert!(looped.looped);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(NGRAM_FILE);
        idx.save(&path).unwrap();
        let back = NgramIndex::load(&path).unwrap();
        assert_eq!(back.count(&codec.encode("Ten en")), 2);
    }
}
