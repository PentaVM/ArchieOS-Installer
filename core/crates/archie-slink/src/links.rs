//! The links of the plan (§5.2), one payload type each.
//!
//! | Link | Producer → consumer | Grade |
//! |---|---|---|
//! | `scene.state` | IRIS → action, reading, Cortex | restricted |
//! | `scene.text` | IRIS reading / accessibility → Wanda's LNCM | restricted |
//! | `scene.people` | IRIS → Wanda | restricted |
//! | `social.intent` | Wanda → LLM, harness | restricted |
//! | `social.focus` | Wanda → Cortex | restricted |
//! | `llm.partial` | LLM → harness, joiner | restricted |
//! | `task.intent` | harness → joiner | restricted |
//! | `focus.plan` | Cortex → archie-accel | internal |
//!
//! What a screen, a camera, a conversation or the people in it say does not leave this
//! machine. `task.intent` names what a partial answer seems to need; it is a proposal and
//! grants nothing.

use crate::{Payload, PrivacyGrade};
use archie_accel::advice::ResourceAdvice;
use serde::{Deserialize, Serialize};

fn finite(values: impl IntoIterator<Item = f32>, what: &str) -> Result<(), String> {
    if values.into_iter().all(f32::is_finite) {
        Ok(())
    } else {
        Err(format!("{what} must be finite"))
    }
}

fn bounded(text: &str, chars: usize, what: &str) -> Result<(), String> {
    if text.chars().count() > chars {
        Err(format!("{what} longer than {chars} characters"))
    } else {
        Ok(())
    }
}

fn at_most<T>(items: &[T], count: usize, what: &str) -> Result<(), String> {
    if items.len() > count {
        Err(format!("more than {count} {what}"))
    } else {
        Ok(())
    }
}

/// A thing IRIS follows in a source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneTrack {
    pub id: u64,
    pub label: String,
    /// Pixels of the source, `x0 y0 x1 y1`.
    pub bbox: [f32; 4],
    pub score: f32,
    /// `inferred`, `reused`, `tracked` or `lost`.
    pub source: String,
}

/// `scene.state`: a source's scene as fixed-size state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneState {
    /// The focus: `screen:0`, `camera:desk`.
    pub focus: String,
    /// The scene slots IRIS pools from the fused map (at most 16 of at most 1024 values).
    #[serde(default)]
    pub slots: Vec<Vec<f32>>,
    pub tracks: Vec<SceneTrack>,
    /// Changed fraction of the frame this state describes.
    pub damage: f32,
}

impl Payload for SceneState {
    const LINK: &'static str = "scene.state";
    const MAX_BYTES: usize = 512 * 1024;

    fn grade(&self) -> PrivacyGrade {
        PrivacyGrade::Restricted
    }

    fn validate(&self) -> Result<(), String> {
        bounded(&self.focus, 128, "focus")?;
        at_most(&self.slots, 16, "slots")?;
        if self.slots.iter().any(|s| s.len() > 1024) {
            return Err("a slot wider than 1024".into());
        }
        finite(self.slots.iter().flatten().copied(), "slots")?;
        at_most(&self.tracks, 256, "tracks")?;
        for track in &self.tracks {
            bounded(&track.label, 64, "track label")?;
            bounded(&track.source, 16, "track source")?;
            finite(track.bbox.iter().copied().chain([track.score]), "tracks")?;
        }
        finite([self.damage], "damage")
    }
}

/// Where a line of text came from, in order of preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOrigin {
    /// A specialist OCR (none ships yet).
    ExternalOcr,
    /// IRIS's reading head (`read_decoder`, not yet ported to Rust).
    IrisRead,
    /// A web page's DOM, through the browser bridge.
    Dom,
    /// The accessibility tree: exact and free.
    Accessibility,
}

/// A line of text seen in a source, new or changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneTextLine {
    /// The track (a region, a control) the line belongs to.
    pub track: u64,
    pub text: String,
    pub origin: TextOrigin,
    pub confidence: f32,
    pub observed_at_ms: u64,
}

/// `scene.text`: the lines that are new or changed this tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneTextDelta {
    pub focus: String,
    pub lines: Vec<SceneTextLine>,
}

impl Payload for SceneTextDelta {
    const LINK: &'static str = "scene.text";
    const MAX_BYTES: usize = 256 * 1024;

    fn grade(&self) -> PrivacyGrade {
        PrivacyGrade::Restricted
    }

    fn validate(&self) -> Result<(), String> {
        bounded(&self.focus, 128, "focus")?;
        at_most(&self.lines, 256, "lines")?;
        for line in &self.lines {
            bounded(&line.text, 512, "line")?;
            finite([line.confidence], "confidence")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenePerson {
    pub track: u64,
    /// IRIS's instance embedding of the person, unit length.
    pub embedding: Vec<f32>,
}

/// `scene.people`: people in a source, for Wanda to match with its roster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenePeople {
    pub focus: String,
    pub people: Vec<ScenePerson>,
}

impl Payload for ScenePeople {
    const LINK: &'static str = "scene.people";
    const MAX_BYTES: usize = 256 * 1024;

    fn grade(&self) -> PrivacyGrade {
        PrivacyGrade::Restricted
    }

    fn validate(&self) -> Result<(), String> {
        bounded(&self.focus, 128, "focus")?;
        at_most(&self.people, 32, "people")?;
        for person in &self.people {
            if person.embedding.len() > 1024 {
                return Err("an embedding wider than 1024".into());
            }
            finite(person.embedding.iter().copied(), "embedding")?;
        }
        Ok(())
    }
}

/// `social.intent`: Wanda's quick decision and the coarse level of its plan, published as
/// soon as its first pass ends so the LLM can take it as a short suffix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialIntent {
    pub conversation: String,
    /// The quick decision (`reply`, `wait`, `ask`, ...).
    pub decision: String,
    /// Level-0 codes of the plan in Wanda's codec.
    #[serde(default)]
    pub coarse_plan: Vec<u32>,
    #[serde(default)]
    pub tone: Option<String>,
}

impl Payload for SocialIntent {
    const LINK: &'static str = "social.intent";
    const MAX_BYTES: usize = 16 * 1024;

    fn grade(&self) -> PrivacyGrade {
        PrivacyGrade::Restricted
    }

    fn validate(&self) -> Result<(), String> {
        bounded(&self.conversation, 128, "conversation")?;
        bounded(&self.decision, 64, "decision")?;
        at_most(&self.coarse_plan, 256, "plan codes")?;
        if let Some(tone) = &self.tone {
            bounded(tone, 64, "tone")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusPerson {
    pub who: String,
    pub strength: f32,
}

/// `social.focus`: who or what matters now, as a suggestion for Cortex's focus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialFocus {
    pub people: Vec<FocusPerson>,
    pub reason: String,
}

impl Payload for SocialFocus {
    const LINK: &'static str = "social.focus";
    const MAX_BYTES: usize = 16 * 1024;

    fn grade(&self) -> PrivacyGrade {
        PrivacyGrade::Restricted
    }

    fn validate(&self) -> Result<(), String> {
        at_most(&self.people, 32, "people")?;
        for person in &self.people {
            bounded(&person.who, 128, "who")?;
            finite([person.strength], "strength")?;
        }
        bounded(&self.reason, 256, "reason")
    }
}

/// `llm.partial`: the LLM's answer as it streams.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmPartial {
    pub conversation: String,
    /// The text so far.
    pub text: String,
    pub done: bool,
}

impl Payload for LlmPartial {
    const LINK: &'static str = "llm.partial";
    const MAX_BYTES: usize = 128 * 1024;

    fn grade(&self) -> PrivacyGrade {
        PrivacyGrade::Restricted
    }

    fn validate(&self) -> Result<(), String> {
        bounded(&self.conversation, 128, "conversation")?;
        bounded(&self.text, 64 * 1024, "text")
    }
}

/// What the harness may prepare, without effects, while the answer streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preparation {
    OpenFile,
    FetchContext,
    CompileInSandbox,
    DraftMessage,
}

/// `task.intent`: a proposal the harness read in a partial answer. It grants nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskIntent {
    pub conversation: String,
    pub preparation: Preparation,
    /// What it names: a path, a tool, a contact.
    pub target: String,
}

impl Payload for TaskIntent {
    const LINK: &'static str = "task.intent";
    const MAX_BYTES: usize = 8 * 1024;

    fn grade(&self) -> PrivacyGrade {
        PrivacyGrade::Restricted
    }

    fn validate(&self) -> Result<(), String> {
        bounded(&self.conversation, 128, "conversation")?;
        bounded(&self.target, 512, "target")
    }
}

/// `focus.plan`: Cortex's advice for every focus, for archie-accel to apply inside Policy's
/// envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusPlan {
    pub advice: Vec<ResourceAdvice>,
}

impl Payload for FocusPlan {
    const LINK: &'static str = "focus.plan";
    const MAX_BYTES: usize = 64 * 1024;

    fn grade(&self) -> PrivacyGrade {
        PrivacyGrade::Internal
    }

    fn validate(&self) -> Result<(), String> {
        at_most(&self.advice, 16, "foci")?;
        for advice in &self.advice {
            finite([advice.share, advice.max_fps], "advice")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, Locality, Message};

    #[test]
    fn screen_text_stays_on_this_machine() {
        let delta = SceneTextDelta {
            focus: "screen:0".into(),
            lines: vec![SceneTextLine {
                track: 4,
                text: "Reunión con Ana el viernes".into(),
                origin: TextOrigin::Accessibility,
                confidence: 1.0,
                observed_at_ms: 12,
            }],
        };
        let message = Message::new(1, 12, delta).unwrap();
        assert!(message.may_reach(Locality::Local));
        assert!(!message.may_reach(Locality::Fabric));
        assert!(!message.may_reach(Locality::Cloud));
    }

    #[test]
    fn messages_from_elsewhere_are_checked_whole() {
        let intent = TaskIntent {
            conversation: "c1".into(),
            preparation: Preparation::OpenFile,
            target: "informe.odt".into(),
        };
        let message = Message::new(3, 0, intent).unwrap();
        let bytes = serde_json::to_vec(&message).unwrap();
        assert_eq!(Message::<TaskIntent>::from_slice(&bytes).unwrap(), message);

        let mut tampered = message.clone();
        tampered.payload.target = "/etc/shadow".into();
        let bytes = serde_json::to_vec(&tampered).unwrap();
        assert!(matches!(
            Message::<TaskIntent>::from_slice(&bytes),
            Err(Error::Digest { .. })
        ));

        let mut loosened = message.clone();
        loosened.header.grade = PrivacyGrade::Public;
        assert!(loosened.verify().is_err());

        let mut value = serde_json::to_value(&message).unwrap();
        value["payload"]["grant"] = serde_json::json!("execute");
        assert!(Message::<TaskIntent>::from_slice(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(
            Message::<SocialIntent>::from_slice(&serde_json::to_vec(&message).unwrap()).is_err()
        );
    }

    #[test]
    fn oversized_or_non_finite_content_is_refused() {
        let state = SceneState {
            focus: "camera:desk".into(),
            slots: vec![vec![0.0; 1025]],
            tracks: Vec::new(),
            damage: 0.0,
        };
        assert!(Message::new(1, 0, state).is_err());
        let partial = LlmPartial {
            conversation: "c".into(),
            text: "x".repeat(70_000),
            done: false,
        };
        assert!(matches!(
            Message::new(1, 0, partial),
            Err(Error::Invalid { .. })
        ));
        let people = ScenePeople {
            focus: "camera:desk".into(),
            people: vec![ScenePerson {
                track: 1,
                embedding: vec![f32::INFINITY],
            }],
        };
        assert!(Message::new(1, 0, people).is_err());
    }
}
