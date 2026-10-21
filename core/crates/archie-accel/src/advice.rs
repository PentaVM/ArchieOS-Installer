//! Cortex's advice for a focus, clamped into Policy's envelope.
//!
//! Cortex proposes how to share IRIS among foci (`CandidateSchedulerChange` carrying a
//! `ResourceAdvice`); its contract says `predict_process` cannot change scheduler policy,
//! priority or grants. The envelope is Policy's: the highest class IRIS work may run at,
//! the inference rate, VRAM, and whether experts may leave the GPU. Advice is applied inside
//! it and can only lower what the envelope allows, never raise it.

use crate::plan::{FocusBudget, FocusMode, Priority};
use crate::Error;
use serde::{Deserialize, Serialize};

/// Phrases per focus at most (IRIS reads 16 per row).
pub const MAX_PHRASES: usize = 16;
/// Characters per phrase at most.
pub const MAX_PHRASE_CHARS: usize = 64;

/// What Policy approved for this process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub max_priority: Priority,
    pub max_fps: f32,
    pub max_vram_bytes: u64,
    pub allow_partial_experts: bool,
    /// Fraction of each layer's experts that must stay resident if any leave.
    pub expert_floor: f32,
}

/// Cortex's advice for one focus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceAdvice {
    pub focus: String,
    /// Suggested share of IRIS's GPU time, [0, 1].
    pub share: f32,
    pub mode: FocusMode,
    #[serde(default)]
    pub phrases: Vec<String>,
    #[serde(default)]
    pub read_text: bool,
    pub priority: Priority,
    pub max_fps: f32,
}

impl ResourceAdvice {
    /// The budget this advice gives within `envelope`, with the phrases to read.
    pub fn apply(&self, envelope: &Envelope) -> Result<(FocusBudget, Vec<String>), Error> {
        if self.focus.is_empty() || self.focus.len() > 128 {
            return Err(Error::Advice("focus id must be 1 to 128 bytes".into()));
        }
        if !self.share.is_finite() || !self.max_fps.is_finite() {
            return Err(Error::Advice("share and rate must be finite".into()));
        }
        if self.phrases.len() > MAX_PHRASES {
            return Err(Error::Advice(format!(
                "{} phrases, at most {MAX_PHRASES}",
                self.phrases.len()
            )));
        }
        let mut phrases = Vec::with_capacity(self.phrases.len());
        for phrase in &self.phrases {
            let phrase = phrase.trim();
            if phrase.is_empty() || phrase.chars().count() > MAX_PHRASE_CHARS {
                return Err(Error::Advice(format!(
                    "phrases must be 1 to {MAX_PHRASE_CHARS} characters"
                )));
            }
            phrases.push(phrase.to_owned());
        }
        let share = self.share.clamp(0.0, 1.0);
        let budget = FocusBudget {
            focus: self.focus.clone(),
            priority: self.priority.min(envelope.max_priority),
            strength: share,
            max_fps: self.max_fps.clamp(0.0, envelope.max_fps.max(0.0)),
            mode: if share == 0.0 {
                FocusMode::Off
            } else {
                self.mode
            },
            read_text: self.read_text,
        };
        Ok((budget, phrases))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> Envelope {
        Envelope {
            max_priority: Priority::ActiveFocus,
            max_fps: 20.0,
            max_vram_bytes: 3 << 30,
            allow_partial_experts: false,
            expert_floor: 0.5,
        }
    }

    #[test]
    fn advice_can_lower_but_never_raise_what_the_envelope_allows() {
        let advice: ResourceAdvice = serde_json::from_str(
            r#"{"focus":"camera:desk","share":1.7,"mode":"focused","phrases":[" taza roja "],
                "priority":"voice_reflex","max_fps":60}"#,
        )
        .unwrap();
        let (budget, phrases) = advice.apply(&envelope()).unwrap();
        assert_eq!(budget.priority, Priority::ActiveFocus);
        assert_eq!(budget.max_fps, 20.0);
        assert_eq!(budget.strength, 1.0);
        assert_eq!(phrases, ["taza roja"]);
    }

    #[test]
    fn unknown_fields_and_oversized_phrases_are_refused() {
        assert!(serde_json::from_str::<ResourceAdvice>(
            r#"{"focus":"a","share":1,"mode":"open","priority":"batch","max_fps":1,"grant":"gpu"}"#
        )
        .is_err());
        let mut advice: ResourceAdvice = serde_json::from_str(
            r#"{"focus":"a","share":0.5,"mode":"open","priority":"batch","max_fps":1}"#,
        )
        .unwrap();
        advice.phrases = vec!["x".repeat(65)];
        assert!(advice.apply(&envelope()).is_err());
        advice.phrases = vec!["y".into(); 17];
        assert!(advice.apply(&envelope()).is_err());
        advice.phrases.clear();
        advice.share = 0.0;
        assert_eq!(advice.apply(&envelope()).unwrap().0.mode, FocusMode::Off);
    }
}
