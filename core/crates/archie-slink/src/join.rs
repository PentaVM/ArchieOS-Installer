//! The joiner: the latest of every link at once, then the post-processing in one batch.
//!
//! When the main producer of a turn finishes (the LLM), the joiner takes the latest
//! revision of every link without waiting for anyone and runs the post-processing steps
//! over that one snapshot: checking names against Wanda's roster, choosing among
//! candidates, Policy's check, formatting for voice or a notice (§5.3). The turn's latency
//! becomes the slowest model plus this batch, instead of the sum of the models.

use crate::link::Link;
use crate::{Payload, PrivacyGrade};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Any link, read as JSON.
pub trait Latest: Send + Sync {
    fn link(&self) -> &'static str;
    fn latest_entry(&self) -> Option<Entry>;
}

impl<P: Payload> Latest for Link<P> {
    fn link(&self) -> &'static str {
        P::LINK
    }

    fn latest_entry(&self) -> Option<Entry> {
        let message = self.latest()?;
        Some(Entry {
            revision: message.header.revision,
            observed_at_ms: message.header.observed_at_ms,
            grade: message.header.grade,
            payload: serde_json::to_value(&message.payload).ok()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub revision: u64,
    pub observed_at_ms: u64,
    pub grade: PrivacyGrade,
    pub payload: Value,
}

/// Every link's latest revision at one moment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub links: BTreeMap<&'static str, Entry>,
}

impl Snapshot {
    /// A link's payload as its type, if the link had published.
    pub fn get<P: Payload>(&self) -> Option<P> {
        serde_json::from_value(self.links.get(P::LINK)?.payload.clone()).ok()
    }

    /// The most restrictive grade among the links read: what the joined result carries.
    pub fn grade(&self) -> PrivacyGrade {
        self.links
            .values()
            .map(|e| e.grade)
            .max()
            .unwrap_or(PrivacyGrade::Public)
    }
}

/// A post-processing step over the snapshot.
pub type Step<T> = dyn Fn(&Snapshot, T) -> Result<T, String> + Send + Sync;

#[derive(Default, Clone)]
pub struct Joiner {
    links: Vec<Arc<dyn Latest>>,
}

impl Joiner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, link: Arc<dyn Latest>) -> Self {
        self.links.push(link);
        self
    }

    /// The latest revision of every link, without waiting for any.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            links: self
                .links
                .iter()
                .filter_map(|l| Some((l.link(), l.latest_entry()?)))
                .collect(),
        }
    }

    /// Runs `steps` in order over one snapshot, starting from `start`.
    pub fn join<T>(&self, start: T, steps: &[&Step<T>]) -> Result<(Snapshot, T), String> {
        let snapshot = self.snapshot();
        let mut value = start;
        for step in steps {
            value = step(&snapshot, value)?;
        }
        Ok((snapshot, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::{FocusPerson, LlmPartial, SocialFocus, SocialIntent};

    #[test]
    fn the_join_reads_every_link_once_and_runs_the_batch() {
        let intent: Link<SocialIntent> = Link::new(8);
        let focus: Link<SocialFocus> = Link::new(8);
        let answer: Link<LlmPartial> = Link::new(8);
        let joiner = Joiner::new()
            .with(Arc::new(intent.clone()))
            .with(Arc::new(focus.clone()))
            .with(Arc::new(answer.clone()));
        assert!(joiner.snapshot().links.is_empty());

        intent
            .publish(
                5,
                SocialIntent {
                    conversation: "c1".into(),
                    decision: "reply".into(),
                    coarse_plan: vec![3, 9],
                    tone: Some("cálido".into()),
                },
            )
            .unwrap();
        focus
            .publish(
                6,
                SocialFocus {
                    people: vec![FocusPerson {
                        who: "Ana".into(),
                        strength: 0.9,
                    }],
                    reason: "she asked".into(),
                },
            )
            .unwrap();
        answer
            .publish(
                9,
                LlmPartial {
                    conversation: "c1".into(),
                    text: "Claro, Ana y Bruno, nos vemos el viernes.".into(),
                    done: true,
                },
            )
            .unwrap();

        // Names the roster (social.focus) does not know are not asserted as people.
        let guard_names: &Step<String> = &|snapshot, text| {
            let known: Vec<String> = snapshot
                .get::<SocialFocus>()
                .map(|f| f.people.into_iter().map(|p| p.who).collect())
                .unwrap_or_default();
            Ok(text
                .split(' ')
                .map(|word| {
                    let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
                    let capital = bare.chars().next().is_some_and(char::is_uppercase);
                    if capital
                        && bare.len() > 1
                        && bare != "Claro"
                        && !known.iter().any(|k| k == bare)
                    {
                        word.replace(bare, "[?]")
                    } else {
                        word.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(" "))
        };
        let with_tone: &Step<String> = &|snapshot, text| {
            let tone = snapshot.get::<SocialIntent>().and_then(|i| i.tone);
            Ok(format!("[{}] {text}", tone.unwrap_or_default()))
        };
        let (snapshot, out) = joiner
            .join(
                answer.latest().unwrap().payload.text.clone(),
                &[guard_names, with_tone],
            )
            .unwrap();
        assert_eq!(snapshot.links.len(), 3);
        assert_eq!(snapshot.grade(), PrivacyGrade::Restricted);
        assert_eq!(out, "[cálido] Claro, Ana y [?], nos vemos el viernes.");
        let refuse: &Step<String> = &|_, _| Err("policy: not now".into());
        assert!(joiner.join(String::new(), &[refuse]).is_err());
    }
}
