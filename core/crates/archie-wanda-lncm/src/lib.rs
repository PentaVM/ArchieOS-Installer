//! # archie-wanda-lncm
//!
//! Wanda's LNCM: a small expert that sits after a language model and before the voice.
//! "L" is for the low-rank adapters that specialise it per context domain without touching
//! the shared weights; "NCM" is next-concept modelling (Hazy Research, arXiv:2610.14162): text
//! is held as a hierarchy of discrete codes, coarse to fine, and a transformer predicts each
//! level in one parallel pass instead of one token at a time.
//!
//! What it does, in the order the runtime uses it:
//!
//! 1. **Gate.** Is this question for Wanda at all? One pass over the question.
//! 2. **Read.** A long conversation is read in chunks. Cortex-style bookkeeping is exact and
//!    deterministic (who was present for which message, who was told what); only the
//!    interpretation (which message states a fact, what a message says about how a person
//!    behaves or feels) comes from a model, as delta lines that are validated before they
//!    touch the state. The state per person is kept: what they have seen, and what to expect
//!    of them. Memory does not grow with the length of the conversation.
//! 3. **State 1.** A decision head answers closed questions at once (yes/no, one of the
//!    options, or "I cannot know"), with a calibrated confidence.
//! 4. **State 2.** The next-concept generator writes the long answer — a short plan, then the
//!    words — conditioned on the question, the state and, when there is one, a language
//!    model's draft. Names it writes are checked against the people in the state.
//!
//! Nothing here acts. Inferences about people are hypotheses; only the person concerned can
//! turn one into a declaration. Policy, leases and Cerebra keep every authority they had.
//!
//! The crate also carries everything training needs: owned synthetic worlds with exact labels
//! ([`synth`]), dataset schemas ([`data`]), the multi-teacher distillation pipeline through
//! the existing egress gate ([`distill`]), and the training stages ([`train`]).
#![forbid(unsafe_code)]

pub mod codec;
pub mod config;
pub mod data;
pub mod distill;
pub mod generator;
pub mod model;
pub mod nn;
pub mod reader;
pub mod safety;
pub mod state;
pub mod synth;
pub mod train;
pub mod vq;

pub use codec::{Codec, Special};
pub use config::LncmConfig;
pub use model::{Composition, Decision, Lncm, QuickAnswer};
pub use reader::{Interpreter, ReadReport, Turn, TurnKind};
pub use state::{Expectation, PersonState, SocialState, Standing};

#[derive(Debug, thiserror::Error)]
pub enum LncmError {
    #[error("invalid LNCM configuration: {0}")]
    Config(String),
    #[error("tensor operation failed: {0}")]
    Tensor(#[from] candle_core::Error),
    #[error("checkpoint is missing, unreadable or does not match its manifest: {0}")]
    Checkpoint(String),
    #[error("input refused: {0}")]
    Input(&'static str),
    #[error("dataset refused: {0}")]
    Data(String),
    #[error("model call failed: {0}")]
    Model(String),
    #[error("i/o failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialisation failed: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, LncmError>;

/// Confidence never claimed for an inference about a person, however much evidence there is:
/// a pattern of behaviour is not the person's word.
pub const MAX_INFERRED_CONFIDENCE: f32 = 0.95;

/// The language an example or a question is in. The codec is language-neutral; this only
/// selects templates and the language tag token.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    pub fn yes(self) -> &'static str {
        match self {
            Lang::Es => "sí",
            Lang::En => "yes",
        }
    }
    pub fn no(self) -> &'static str {
        match self {
            Lang::Es => "no",
            Lang::En => "no",
        }
    }
}

/// What one pass of the model is asked to do. Each task has its own leading token, so one
/// set of weights serves all four.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    /// Is the question for Wanda? Head only.
    Gate,
    /// Does this chunk change the state, and how? Head plus generated delta lines.
    Update,
    /// Which option, or none? Head only.
    Decide,
    /// The long answer: plan, then words. Generated.
    Compose,
}

impl Task {
    pub fn generates(self) -> bool {
        matches!(self, Task::Update | Task::Compose)
    }
}

/// Text that is bounded, not blank and free of control characters other than newlines and tabs.
pub(crate) fn bounded_text(text: &str, max_chars: usize) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= max_chars
        && !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
}

/// Cuts `text` to at most `max_bytes` on a character boundary.
pub(crate) fn clip_bytes(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Lower-case hex SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_never_splits_a_character() {
        assert_eq!(clip_bytes("ñandú", 3), "ña");
        assert_eq!(clip_bytes("ñandú", 2), "ñ");
        assert_eq!(clip_bytes("ñandú", 1), "");
        assert_eq!(clip_bytes("abc", 10), "abc");
    }

    #[test]
    fn bounded_text_refuses_blank_long_and_control() {
        assert!(bounded_text("hola\nqué tal", 20));
        assert!(!bounded_text("   ", 20));
        assert!(!bounded_text("abc", 2));
        assert!(!bounded_text("a\u{0007}b", 20));
    }
}
