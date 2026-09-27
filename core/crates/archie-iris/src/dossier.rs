//! What Archie has read about something it saw, kept in layers so a follow-up is instant.
//!
//! Asked "what is that?", Archie answers with a headline first ("a John Deere engine from a
//! 9600 combine"). The search that fills in the details runs after the answer, and its facts
//! wait here. "Tell me more" is then read from this buffer, a few facts at a time, instead
//! of starting a search while the person waits.
//!
//! Every fact carries the page it came from. A model's own recollection is not a fact: the
//! point of reading is that the number of cylinders is the one the page states.
//!
//! The buffer is keyed by track, so it lives as long as working memory follows the object.
//! Recognising the same object tomorrow is a durable memory and passes admission elsewhere.

use crate::TrackId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

const MAX_HEADLINE_BYTES: usize = 280;
const MAX_FACT_BYTES: usize = 600;
const MAX_URL_BYTES: usize = 2_048;
const MAX_FACTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DossierBounds {
    pub ttl_ms: i64,
    pub max_dossiers: usize,
}

impl Default for DossierBounds {
    fn default() -> Self {
        Self {
            ttl_ms: 1_800_000,
            max_dossiers: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub text: String,
    pub source_url: String,
}

impl Fact {
    fn validate(&self) -> Result<(), DossierError> {
        let text = self.text.trim();
        if text.is_empty() || text.len() > MAX_FACT_BYTES || text.contains('\0') {
            return Err(DossierError::InvalidFact);
        }
        if self.source_url.len() > MAX_URL_BYTES
            || !(self.source_url.starts_with("https://") || self.source_url.starts_with("http://"))
            || self.source_url.chars().any(char::is_whitespace)
        {
            return Err(DossierError::UnsourcedFact);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DossierState {
    /// Asked about; nothing named yet.
    Identifying,
    /// Named, and the details are being read.
    Reading,
    /// The reading finished; what it found is all there is.
    Complete,
    /// The reading failed. The headline, if any, still stands.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dossier {
    pub track: TrackId,
    pub state: DossierState,
    pub headline: Option<String>,
    pub headline_confidence_permille: u16,
    pub facts: Vec<Fact>,
    /// Facts already told; the next "more" starts here.
    pub told: usize,
    pub updated_at_ms: i64,
}

impl Dossier {
    /// Whether Archie can offer more without reading anything new.
    pub fn has_untold(&self) -> bool {
        self.told < self.facts.len()
    }
}

/// What "tell me more" gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum More<'a> {
    Facts(&'a [Fact]),
    /// The reading has not returned yet: say so rather than guess.
    StillReading,
    /// Everything read has been told.
    NothingMore,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DossierError {
    #[error("dossier bounds are invalid")]
    InvalidBounds,
    #[error("dossier time is invalid")]
    InvalidTime,
    #[error("headline is empty, too long or its confidence is out of range")]
    InvalidHeadline,
    #[error("fact is empty or too long")]
    InvalidFact,
    #[error("fact has no http(s) source")]
    UnsourcedFact,
    #[error("no dossier for this track")]
    Missing,
}

pub struct DossierBuffer {
    bounds: DossierBounds,
    dossiers: BTreeMap<TrackId, Dossier>,
}

impl DossierBuffer {
    pub fn new(bounds: DossierBounds) -> Result<Self, DossierError> {
        if bounds.ttl_ms <= 0 || bounds.max_dossiers == 0 || bounds.max_dossiers > 4_096 {
            return Err(DossierError::InvalidBounds);
        }
        Ok(Self {
            bounds,
            dossiers: BTreeMap::new(),
        })
    }

    /// Opens a dossier when a track is first asked about. One that exists is kept as it is,
    /// so asking twice does not throw away what was read.
    pub fn begin(&mut self, track: TrackId, now_ms: i64) -> Result<&Dossier, DossierError> {
        self.expire(now_ms)?;
        if !self.dossiers.contains_key(&track) {
            while self.dossiers.len() >= self.bounds.max_dossiers {
                let oldest = self
                    .dossiers
                    .values()
                    .min_by_key(|dossier| (dossier.updated_at_ms, dossier.track.clone()))
                    .map(|dossier| dossier.track.clone())
                    .ok_or(DossierError::Missing)?;
                self.dossiers.remove(&oldest);
            }
            self.dossiers.insert(
                track.clone(),
                Dossier {
                    track: track.clone(),
                    state: DossierState::Identifying,
                    headline: None,
                    headline_confidence_permille: 0,
                    facts: Vec::new(),
                    told: 0,
                    updated_at_ms: now_ms,
                },
            );
        }
        self.dossiers.get(&track).ok_or(DossierError::Missing)
    }

    /// The first answer. Details are read after it, so the state moves to `Reading`.
    pub fn name(
        &mut self,
        track: &TrackId,
        headline: impl Into<String>,
        confidence_permille: u16,
        now_ms: i64,
    ) -> Result<(), DossierError> {
        let headline = headline.into();
        let trimmed = headline.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_HEADLINE_BYTES || confidence_permille > 1_000 {
            return Err(DossierError::InvalidHeadline);
        }
        let dossier = self.live(track, now_ms)?;
        dossier.headline = Some(trimmed.to_owned());
        dossier.headline_confidence_permille = confidence_permille;
        if dossier.state == DossierState::Identifying {
            dossier.state = DossierState::Reading;
        }
        Ok(())
    }

    /// Facts from a finished reading. Every one must name its page, or none is kept. A fact
    /// already held is not added twice.
    pub fn read(
        &mut self,
        track: &TrackId,
        facts: Vec<Fact>,
        now_ms: i64,
    ) -> Result<(), DossierError> {
        for fact in &facts {
            fact.validate()?;
        }
        let dossier = self.live(track, now_ms)?;
        for fact in facts {
            let text = fact.text.trim().to_owned();
            if dossier.facts.len() < MAX_FACTS
                && !dossier.facts.iter().any(|held| held.text == text)
            {
                dossier.facts.push(Fact {
                    text,
                    source_url: fact.source_url,
                });
            }
        }
        dossier.state = DossierState::Complete;
        Ok(())
    }

    pub fn failed(&mut self, track: &TrackId, now_ms: i64) -> Result<(), DossierError> {
        self.live(track, now_ms)?.state = DossierState::Failed;
        Ok(())
    }

    /// Up to `count` facts not yet told, and marks them told.
    pub fn more(
        &mut self,
        track: &TrackId,
        count: usize,
        now_ms: i64,
    ) -> Result<More<'_>, DossierError> {
        self.expire(now_ms)?;
        let Some(dossier) = self.dossiers.get_mut(track) else {
            return Ok(More::Unknown);
        };
        if !dossier.has_untold() {
            return Ok(match dossier.state {
                DossierState::Identifying | DossierState::Reading => More::StillReading,
                DossierState::Complete | DossierState::Failed => More::NothingMore,
            });
        }
        let start = dossier.told;
        let end = (start + count.max(1)).min(dossier.facts.len());
        dossier.told = end;
        dossier.updated_at_ms = now_ms;
        Ok(More::Facts(&dossier.facts[start..end]))
    }

    pub fn get(&self, track: &TrackId) -> Option<&Dossier> {
        self.dossiers.get(track)
    }

    pub fn expire(&mut self, now_ms: i64) -> Result<(), DossierError> {
        if now_ms < 0 {
            return Err(DossierError::InvalidTime);
        }
        let cutoff = now_ms.saturating_sub(self.bounds.ttl_ms);
        self.dossiers
            .retain(|_, dossier| dossier.updated_at_ms >= cutoff);
        Ok(())
    }

    fn live(&mut self, track: &TrackId, now_ms: i64) -> Result<&mut Dossier, DossierError> {
        self.expire(now_ms)?;
        let dossier = self.dossiers.get_mut(track).ok_or(DossierError::Missing)?;
        dossier.updated_at_ms = now_ms;
        Ok(dossier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> TrackId {
        TrackId::new("track:engine").unwrap()
    }

    fn fact(text: &str) -> Fact {
        Fact {
            text: text.into(),
            source_url: "https://example.org/9600".into(),
        }
    }

    #[test]
    fn the_headline_comes_first_and_more_is_told_from_the_buffer_in_order() {
        let mut buffer = DossierBuffer::new(DossierBounds::default()).unwrap();
        buffer.begin(track(), 0).unwrap();
        assert_eq!(buffer.more(&track(), 2, 10).unwrap(), More::StillReading);
        buffer
            .name(
                &track(),
                "A John Deere engine from a 9600 combine",
                700,
                400,
            )
            .unwrap();
        assert_eq!(buffer.get(&track()).unwrap().state, DossierState::Reading);
        assert_eq!(buffer.more(&track(), 2, 500).unwrap(), More::StillReading);
        buffer
            .read(
                &track(),
                vec![
                    fact("Six-cylinder diesel"),
                    fact("7.6 litres"),
                    fact("Built 1989-1998"),
                ],
                3_000,
            )
            .unwrap();
        let first: Vec<_> = match buffer.more(&track(), 2, 4_000).unwrap() {
            More::Facts(facts) => facts.iter().map(|fact| fact.text.clone()).collect(),
            other => panic!("{other:?}"),
        };
        assert_eq!(first, ["Six-cylinder diesel", "7.6 litres"]);
        assert!(buffer.get(&track()).unwrap().has_untold());
        assert!(
            matches!(buffer.more(&track(), 2, 5_000).unwrap(), More::Facts([only]) if only.text == "Built 1989-1998")
        );
        assert_eq!(buffer.more(&track(), 2, 6_000).unwrap(), More::NothingMore);
    }

    #[test]
    fn a_fact_without_a_page_is_refused_and_none_of_its_batch_is_kept() {
        let mut buffer = DossierBuffer::new(DossierBounds::default()).unwrap();
        buffer.begin(track(), 0).unwrap();
        let unsourced = Fact {
            text: "Eight cylinders".into(),
            source_url: "the model said so".into(),
        };
        assert_eq!(
            buffer
                .read(&track(), vec![fact("7.6 litres"), unsourced], 1)
                .unwrap_err(),
            DossierError::UnsourcedFact
        );
        assert!(buffer.get(&track()).unwrap().facts.is_empty());
    }

    #[test]
    fn asking_again_keeps_what_was_read_and_duplicates_are_not_added() {
        let mut buffer = DossierBuffer::new(DossierBounds::default()).unwrap();
        buffer.begin(track(), 0).unwrap();
        buffer.read(&track(), vec![fact("7.6 litres")], 1).unwrap();
        buffer.begin(track(), 2).unwrap();
        buffer
            .read(
                &track(),
                vec![fact(" 7.6 litres "), fact("Six-cylinder diesel")],
                3,
            )
            .unwrap();
        assert_eq!(buffer.get(&track()).unwrap().facts.len(), 2);
    }

    #[test]
    fn dossiers_expire_and_the_least_recently_used_is_dropped_at_capacity() {
        let mut buffer = DossierBuffer::new(DossierBounds {
            ttl_ms: 100,
            max_dossiers: 2,
        })
        .unwrap();
        let a = TrackId::new("track:a").unwrap();
        let b = TrackId::new("track:b").unwrap();
        let c = TrackId::new("track:c").unwrap();
        buffer.begin(a.clone(), 0).unwrap();
        buffer.begin(b.clone(), 10).unwrap();
        buffer.name(&a, "a plush toy", 600, 20).unwrap();
        buffer.begin(c.clone(), 30).unwrap();
        assert!(buffer.get(&b).is_none());
        assert!(buffer.get(&a).is_some());
        assert_eq!(buffer.more(&c, 1, 200).unwrap(), More::Unknown);
    }
}
