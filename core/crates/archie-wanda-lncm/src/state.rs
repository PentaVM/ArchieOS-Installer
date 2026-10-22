//! What Wanda keeps about each person: what they have seen, and what to expect of them.
//!
//! Two kinds of knowledge live here and are never confused:
//!
//! - **Bookkeeping**, exact and deterministic: who was in which conversation when a message was
//!   posted, who was told what, how often someone writes and how long they take to answer.
//!   Nothing about it is a model's opinion; it is what Cortex can compute from presence.
//! - **Interpretation**, from a model: which message states a fact, what a message says about
//!   how someone behaves or feels. It enters only as [`DeltaLine`]s, which are validated against
//!   the people and turns of the chunk that produced them. A line about someone who is not in
//!   the state, or about a turn outside the chunk, is refused.
//!
//! Expectations are hypotheses. Their confidence is a Beta posterior over decayed counts, so old
//! behaviour fades, and it is capped at [`MAX_INFERRED_CONFIDENCE`] unless the person concerned
//! declared it themselves. The owner can confirm or retract one; neither makes it a fact.

use crate::{bounded_text, clip_bytes, Lang, LncmError, Result, MAX_INFERRED_CONFIDENCE};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const STATE_FORMAT: u16 = 1;
pub const MAX_PEOPLE: usize = 512;
pub const MAX_TOPICS: usize = 256;
pub const MAX_EXPECTATIONS: usize = 32;
pub const MAX_VERSIONS: usize = 8;
pub const MAX_EVIDENCE: usize = 8;
pub const MAX_CHATS_PER_PERSON: usize = 64;
/// How long it takes for a piece of behavioural evidence to count half as much.
pub const DEFAULT_HALF_LIFE_MS: i64 = 90 * 24 * 3_600_000;
const DECLARED_CONFIDENCE: f32 = 0.99;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Inferred from behaviour. A hypothesis.
    Inferred,
    /// The owner agrees with the inference. Still about someone else; still not a fact.
    OwnerConfirmed,
    /// The person concerned said it about themselves.
    SubjectDeclared,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    pub key: String,
    pub statement: String,
    pub support: f32,
    pub contradict: f32,
    pub standing: Standing,
    pub first_at_ms: i64,
    pub last_at_ms: i64,
    pub evidence: Vec<String>,
}

impl Expectation {
    /// Posterior mean of a Beta(1, 1) prior over the decayed counts, capped by standing.
    pub fn confidence(&self) -> f32 {
        let mean = (self.support + 1.0) / (self.support + self.contradict + 2.0);
        match self.standing {
            Standing::SubjectDeclared => DECLARED_CONFIDENCE,
            _ => mean.min(MAX_INFERRED_CONFIDENCE),
        }
    }
    /// How much evidence there is, decayed.
    pub fn weight(&self) -> f32 {
        self.support + self.contradict
    }
    fn decay(&mut self, now_ms: i64, half_life_ms: i64) {
        if now_ms <= self.last_at_ms || half_life_ms <= 0 {
            return;
        }
        let factor = 0.5f64.powf((now_ms - self.last_at_ms) as f64 / half_life_ms as f64) as f32;
        self.support *= factor;
        self.contradict *= factor;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeenHow {
    /// Was in the conversation when it was posted.
    Present,
    /// Posted it.
    Said,
    /// Someone told them.
    Told,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seen {
    pub value: String,
    pub turn: u64,
    pub at_ms: i64,
    pub how: SeenHow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Affect {
    pub label: String,
    pub intensity: u8,
    pub at_ms: i64,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Behaviour {
    pub messages: u32,
    pub last_message_at_ms: Option<i64>,
    /// Messages per hour of day (UTC).
    pub active_hours: Vec<u32>,
    /// Exponential average of how long they take to write after someone else did, in the same
    /// conversation.
    pub reply_latency_ms: Option<f64>,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self {
            messages: 0,
            last_message_at_ms: None,
            active_hours: vec![0; 24],
            reply_latency_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonState {
    pub id: String,
    pub label: String,
    /// Conversations the person is currently in.
    pub chats: BTreeSet<String>,
    pub first_turn: u64,
    pub last_turn: u64,
    pub seen: BTreeMap<String, Seen>,
    pub expectations: Vec<Expectation>,
    pub affect: Option<Affect>,
    pub behaviour: Behaviour,
}

impl PersonState {
    fn new(id: &str, label: &str, turn: u64) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            chats: BTreeSet::new(),
            first_turn: turn,
            last_turn: turn,
            seen: BTreeMap::new(),
            expectations: Vec::new(),
            affect: None,
            behaviour: Behaviour::default(),
        }
    }

    /// Expectations, strongest first: confidence away from even odds, weighted by evidence.
    pub fn what_to_expect(&self) -> Vec<&Expectation> {
        let mut out: Vec<&Expectation> = self.expectations.iter().collect();
        out.sort_by(|a, b| {
            let score = |e: &Expectation| (e.confidence() - 0.5).abs() * e.weight().min(20.0);
            score(b)
                .total_cmp(&score(a))
                .then_with(|| a.key.cmp(&b.key))
        });
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactVersion {
    pub value: String,
    pub turn: u64,
    pub at_ms: i64,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub topic: String,
    pub versions: Vec<FactVersion>,
}

impl Fact {
    pub fn current(&self) -> Option<&FactVersion> {
        self.versions.last()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialState {
    pub format: u16,
    pub owner: String,
    pub people: BTreeMap<String, PersonState>,
    pub facts: BTreeMap<String, Fact>,
    pub last_turn: u64,
    pub last_at_ms: i64,
    pub half_life_ms: i64,
}

/// One interpretation, as a model writes it.
///
/// ```text
/// none
/// fact <topic> = <value> @<turn>
/// told <person> <topic> = <value> @<turn>
/// expect <person> +|- <key>[: <statement>]
/// affect <person> <label> <0-100>
/// ```
///
/// A person with spaces in their name is written in double quotes.
#[derive(Debug, Clone, PartialEq)]
pub enum DeltaLine {
    None,
    Fact {
        topic: String,
        value: String,
        turn: u64,
    },
    Told {
        person: String,
        topic: String,
        value: String,
        turn: u64,
    },
    Expect {
        person: String,
        positive: bool,
        key: String,
        statement: Option<String>,
    },
    Affect {
        person: String,
        label: String,
        intensity: u8,
    },
}

impl std::fmt::Display for DeltaLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeltaLine::None => f.write_str("none"),
            DeltaLine::Fact { topic, value, turn } => write!(f, "fact {topic} = {value} @{turn}"),
            DeltaLine::Told {
                person,
                topic,
                value,
                turn,
            } => write!(f, "told {} {topic} = {value} @{turn}", quote(person)),
            DeltaLine::Expect {
                person,
                positive,
                key,
                statement,
            } => {
                write!(
                    f,
                    "expect {} {} {key}",
                    quote(person),
                    if *positive { '+' } else { '-' }
                )?;
                if let Some(s) = statement {
                    write!(f, ": {s}")?;
                }
                Ok(())
            }
            DeltaLine::Affect {
                person,
                label,
                intensity,
            } => write!(f, "affect {} {label} {intensity}", quote(person)),
        }
    }
}

fn quote(person: &str) -> String {
    if person.contains(char::is_whitespace) {
        format!("\"{person}\"")
    } else {
        person.to_string()
    }
}

fn slug(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max
        && value.chars().all(|c| {
            (c.is_alphanumeric() && c.is_lowercase()) || c.is_ascii_digit() || c == '-' || c == '_'
        })
}

/// Splits off the first token, honouring double quotes.
fn take_token(text: &str) -> Option<(String, &str)> {
    let text = text.trim_start();
    if let Some(rest) = text.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some((rest[..end].to_string(), &rest[end + 1..]));
    }
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    Some((text[..end].to_string(), &text[end..]))
}

fn topic_value_turn(text: &str) -> Option<(String, String, u64)> {
    let (topic, rest) = text.split_once(" = ")?;
    let (value, turn) = rest.rsplit_once(" @")?;
    let topic = topic.trim().to_lowercase();
    let value = value.trim().to_string();
    if !bounded_text(&topic, 48)
        || topic.contains(['=', '@'])
        || !bounded_text(&value, 120)
        || value.contains('\n')
    {
        return None;
    }
    Some((topic, value, turn.trim().parse().ok()?))
}

fn parse_line(line: &str) -> Option<DeltaLine> {
    let line = line.trim();
    if line == "none" {
        return Some(DeltaLine::None);
    }
    let (verb, rest) = line.split_once(' ')?;
    match verb {
        "fact" => {
            let (topic, value, turn) = topic_value_turn(rest)?;
            Some(DeltaLine::Fact { topic, value, turn })
        }
        "told" => {
            let (person, rest) = take_token(rest)?;
            let (topic, value, turn) = topic_value_turn(rest.trim_start())?;
            Some(DeltaLine::Told {
                person,
                topic,
                value,
                turn,
            })
        }
        "expect" => {
            let (person, rest) = take_token(rest)?;
            let (sign, rest) = take_token(rest)?;
            let positive = match sign.as_str() {
                "+" => true,
                "-" => false,
                _ => return None,
            };
            let (key, statement) = match rest.split_once(':') {
                Some((k, s)) => (k.trim().to_string(), Some(s.trim().to_string())),
                None => (rest.trim().to_string(), None),
            };
            if !slug(&key, 40) || statement.as_deref().is_some_and(|s| !bounded_text(s, 200)) {
                return None;
            }
            Some(DeltaLine::Expect {
                person,
                positive,
                key,
                statement: statement.filter(|s| !s.is_empty()),
            })
        }
        "affect" => {
            let (person, rest) = take_token(rest)?;
            let (label, rest) = take_token(rest)?;
            let (intensity, rest) = take_token(rest)?;
            if !rest.trim().is_empty() || !slug(&label, 32) {
                return None;
            }
            let intensity: u8 = intensity.parse().ok().filter(|n| *n <= 100)?;
            Some(DeltaLine::Affect {
                person,
                label,
                intensity,
            })
        }
        _ => None,
    }
}

/// Parses delta lines. Lines that do not parse are counted, never guessed at.
pub fn parse_delta(text: &str) -> (Vec<DeltaLine>, usize) {
    let mut lines = Vec::new();
    let mut rejected = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()).take(64) {
        match parse_line(line) {
            Some(d) => lines.push(d),
            None => rejected += 1,
        }
    }
    (lines, rejected)
}

/// What the reader knows about the chunk a delta came from; a delta is checked against it.
#[derive(Debug, Default, Clone)]
pub struct ChunkContext {
    /// Turn → people present in its conversation when it was posted (speaker included).
    pub presence: BTreeMap<u64, Vec<String>>,
    /// Turn → evidence reference.
    pub evidence: BTreeMap<u64, String>,
    /// Turn → speaker, for message turns.
    pub speakers: BTreeMap<u64, String>,
    pub at_ms: BTreeMap<u64, i64>,
}

impl ChunkContext {
    fn last_message_of(&self, person: &str) -> Option<u64> {
        self.speakers
            .iter()
            .rev()
            .find(|(_, s)| s.as_str() == person)
            .map(|(&t, _)| t)
    }
    fn last_turn(&self) -> Option<u64> {
        self.evidence.keys().next_back().copied()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyReport {
    pub applied: usize,
    pub rejected: usize,
}

impl SocialState {
    pub fn new(owner_id: &str, owner_label: &str) -> Result<Self> {
        if !bounded_text(owner_id, 128) || !bounded_text(owner_label, 128) {
            return Err(LncmError::Input("invalid_owner"));
        }
        let mut people = BTreeMap::new();
        people.insert(
            owner_id.to_string(),
            PersonState::new(owner_id, owner_label, 0),
        );
        Ok(Self {
            format: STATE_FORMAT,
            owner: owner_id.to_string(),
            people,
            facts: BTreeMap::new(),
            last_turn: 0,
            last_at_ms: 0,
            half_life_ms: DEFAULT_HALF_LIFE_MS,
        })
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let state: Self = serde_json::from_str(text)?;
        state.validate()?;
        Ok(state)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != STATE_FORMAT
            || !self.people.contains_key(&self.owner)
            || self.people.len() > MAX_PEOPLE
            || self.facts.len() > MAX_TOPICS
            || self.people.iter().any(|(id, p)| {
                id != &p.id
                    || p.expectations.len() > MAX_EXPECTATIONS
                    || p.chats.len() > MAX_CHATS_PER_PERSON
                    || p.behaviour.active_hours.len() != 24
                    || p.expectations
                        .iter()
                        .any(|e| e.evidence.len() > MAX_EVIDENCE)
            })
            || self.facts.values().any(|f| f.versions.len() > MAX_VERSIONS)
        {
            return Err(LncmError::Input("invalid_state"));
        }
        Ok(())
    }

    /// The person whose id or label is `name` (case-insensitive).
    pub fn find(&self, name: &str) -> Option<&str> {
        let name = name.trim();
        if self.people.contains_key(name) {
            return Some(self.people.get_key_value(name)?.0.as_str());
        }
        let lower = name.to_lowercase();
        self.people
            .values()
            .find(|p| p.label.to_lowercase() == lower || p.id.to_lowercase() == lower)
            .map(|p| p.id.as_str())
    }

    /// The person with this id, created if new. `None` when the state is full.
    pub fn ensure_person(&mut self, id: &str, label: &str, turn: u64) -> Option<&mut PersonState> {
        if !self.people.contains_key(id) {
            if self.people.len() >= MAX_PEOPLE
                || !bounded_text(id, 128)
                || !bounded_text(label, 128)
            {
                return None;
            }
            self.people
                .insert(id.to_string(), PersonState::new(id, label, turn));
        }
        self.people.get_mut(id)
    }

    pub fn join(&mut self, person: &str, chat: &str, turn: u64) {
        if let Some(p) = self.people.get_mut(person) {
            if p.chats.len() < MAX_CHATS_PER_PERSON {
                p.chats.insert(chat.to_string());
            }
            p.last_turn = p.last_turn.max(turn);
        }
    }

    pub fn leave(&mut self, person: &str, chat: &str, turn: u64) {
        if let Some(p) = self.people.get_mut(person) {
            p.chats.remove(chat);
            p.last_turn = p.last_turn.max(turn);
        }
    }

    /// Records a message: the speaker is in the conversation, and their behaviour statistics
    /// move. `previous` is when someone else last wrote in it, for reply latency.
    pub fn message(
        &mut self,
        speaker: &str,
        chat: &str,
        turn: u64,
        at_ms: i64,
        previous: Option<i64>,
    ) {
        self.join(speaker, chat, turn);
        if let Some(p) = self.people.get_mut(speaker) {
            let b = &mut p.behaviour;
            b.messages = b.messages.saturating_add(1);
            b.last_message_at_ms = Some(at_ms);
            let hour = (at_ms.rem_euclid(86_400_000) / 3_600_000) as usize;
            b.active_hours[hour] = b.active_hours[hour].saturating_add(1);
            if let Some(previous) = previous.filter(|&t| t <= at_ms) {
                let latency = (at_ms - previous) as f64;
                b.reply_latency_ms = Some(match b.reply_latency_ms {
                    None => latency,
                    Some(avg) => avg * 0.8 + latency * 0.2,
                });
            }
        }
        self.last_turn = self.last_turn.max(turn);
        self.last_at_ms = self.last_at_ms.max(at_ms);
    }

    /// The people currently in a conversation (the owner always is: they are the one reading it).
    pub fn present_in(&self, chat: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .people
            .values()
            .filter(|p| p.chats.contains(chat))
            .map(|p| p.id.clone())
            .collect();
        if !out.contains(&self.owner) {
            out.push(self.owner.clone());
        }
        out.sort();
        out
    }

    /// Applies interpretation lines produced for one chunk.
    pub fn apply(&mut self, lines: &[DeltaLine], chunk: &ChunkContext) -> ApplyReport {
        let mut report = ApplyReport::default();
        for line in lines {
            if self.apply_one(line, chunk) {
                report.applied += 1;
            } else {
                report.rejected += 1;
            }
        }
        report
    }

    fn apply_one(&mut self, line: &DeltaLine, chunk: &ChunkContext) -> bool {
        match line {
            DeltaLine::None => true,
            DeltaLine::Fact { topic, value, turn } => {
                let (Some(evidence), Some(present)) =
                    (chunk.evidence.get(turn), chunk.presence.get(turn))
                else {
                    return false;
                };
                if !chunk.speakers.contains_key(turn) {
                    return false; // a fact is stated in a message
                }
                if !self.facts.contains_key(topic) && self.facts.len() >= MAX_TOPICS {
                    return false;
                }
                let at_ms = chunk.at_ms.get(turn).copied().unwrap_or(self.last_at_ms);
                let fact = self.facts.entry(topic.clone()).or_insert_with(|| Fact {
                    topic: topic.clone(),
                    versions: Vec::new(),
                });
                if fact.current().map(|v| &v.value) != Some(value) {
                    fact.versions.push(FactVersion {
                        value: value.clone(),
                        turn: *turn,
                        at_ms,
                        evidence: evidence.clone(),
                    });
                    if fact.versions.len() > MAX_VERSIONS {
                        fact.versions.remove(0);
                    }
                }
                let speaker = chunk.speakers.get(turn);
                for person in present {
                    if let Some(p) = self.people.get_mut(person) {
                        let how = if Some(person) == speaker {
                            SeenHow::Said
                        } else {
                            SeenHow::Present
                        };
                        p.seen.insert(
                            topic.clone(),
                            Seen {
                                value: value.clone(),
                                turn: *turn,
                                at_ms,
                                how,
                            },
                        );
                    }
                }
                true
            }
            DeltaLine::Told {
                person,
                topic,
                value,
                turn,
            } => {
                let Some(id) = self.find(person).map(str::to_string) else {
                    return false;
                };
                if !chunk.evidence.contains_key(turn) {
                    return false;
                }
                let at_ms = chunk.at_ms.get(turn).copied().unwrap_or(self.last_at_ms);
                if let Some(p) = self.people.get_mut(&id) {
                    p.seen.insert(
                        topic.clone(),
                        Seen {
                            value: value.clone(),
                            turn: *turn,
                            at_ms,
                            how: SeenHow::Told,
                        },
                    );
                }
                true
            }
            DeltaLine::Expect {
                person,
                positive,
                key,
                statement,
            } => {
                let Some(id) = self.find(person).map(str::to_string) else {
                    return false;
                };
                let Some(turn) = chunk.last_message_of(&id).or_else(|| chunk.last_turn()) else {
                    return false;
                };
                let evidence = chunk.evidence.get(&turn).cloned().unwrap_or_default();
                let at_ms = chunk.at_ms.get(&turn).copied().unwrap_or(self.last_at_ms);
                let half_life = self.half_life_ms;
                let Some(p) = self.people.get_mut(&id) else {
                    return false;
                };
                if let Some(e) = p.expectations.iter_mut().find(|e| &e.key == key) {
                    e.decay(at_ms, half_life);
                    if *positive {
                        e.support += 1.0;
                        if let Some(s) = statement {
                            e.statement = s.clone();
                        }
                    } else {
                        e.contradict += 1.0;
                    }
                    e.last_at_ms = e.last_at_ms.max(at_ms);
                    push_evidence(&mut e.evidence, evidence);
                    return true;
                }
                // A new expectation needs a statement of the hypothesis. Evidence against it
                // creates it too: "is usually late, confidence 0.33" is what a punctual person
                // looks like, which is more than no entry at all.
                let Some(statement) = statement.clone() else {
                    return false;
                };
                if p.expectations.len() >= MAX_EXPECTATIONS {
                    let weakest = p
                        .expectations
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.standing == Standing::Inferred)
                        .min_by(|a, b| a.1.weight().total_cmp(&b.1.weight()))
                        .map(|(i, _)| i);
                    match weakest {
                        Some(i) => {
                            p.expectations.remove(i);
                        }
                        None => return false,
                    }
                }
                p.expectations.push(Expectation {
                    key: key.clone(),
                    statement,
                    support: if *positive { 1.0 } else { 0.0 },
                    contradict: if *positive { 0.0 } else { 1.0 },
                    standing: Standing::Inferred,
                    first_at_ms: at_ms,
                    last_at_ms: at_ms,
                    evidence: vec![evidence],
                });
                true
            }
            DeltaLine::Affect {
                person,
                label,
                intensity,
            } => {
                let Some(id) = self.find(person).map(str::to_string) else {
                    return false;
                };
                let Some(turn) = chunk.last_message_of(&id) else {
                    return false; // a mood is read from what the person wrote
                };
                let evidence = chunk.evidence.get(&turn).cloned().unwrap_or_default();
                let at_ms = chunk.at_ms.get(&turn).copied().unwrap_or(self.last_at_ms);
                if let Some(p) = self.people.get_mut(&id) {
                    p.affect = Some(Affect {
                        label: label.clone(),
                        intensity: *intensity,
                        at_ms,
                        evidence,
                    });
                }
                true
            }
        }
    }

    /// Whether `person` knows the current value of `topic`. `None` when the topic is unknown.
    pub fn knows(&self, person: &str, topic: &str) -> Option<bool> {
        let current = self.facts.get(topic)?.current()?;
        let p = self.people.get(self.find(person)?)?;
        Some(p.seen.get(topic).is_some_and(|s| s.value == current.value))
    }

    /// What `person` believes `topic` is: the last value they saw.
    pub fn belief(&self, person: &str, topic: &str) -> Option<&str> {
        let p = self.people.get(self.find(person)?)?;
        p.seen.get(topic).map(|s| s.value.as_str())
    }

    /// The owner agrees with an inferred expectation.
    pub fn confirm(&mut self, person: &str, key: &str) -> bool {
        self.with_expectation(person, key, |e| {
            if e.standing == Standing::Inferred {
                e.standing = Standing::OwnerConfirmed;
            }
        })
    }

    /// The person concerned declared this about themselves. Only an identity flow that has
    /// established it is them may call this.
    pub fn declare(&mut self, person: &str, key: &str, statement: &str, at_ms: i64) -> bool {
        if !slug(key, 40) || !bounded_text(statement, 200) {
            return false;
        }
        let Some(id) = self.find(person).map(str::to_string) else {
            return false;
        };
        let Some(p) = self.people.get_mut(&id) else {
            return false;
        };
        match p.expectations.iter_mut().find(|e| e.key == key) {
            Some(e) => {
                e.standing = Standing::SubjectDeclared;
                e.statement = statement.to_string();
                e.last_at_ms = e.last_at_ms.max(at_ms);
            }
            None => {
                if p.expectations.len() >= MAX_EXPECTATIONS {
                    return false;
                }
                p.expectations.push(Expectation {
                    key: key.to_string(),
                    statement: statement.to_string(),
                    support: 0.0,
                    contradict: 0.0,
                    standing: Standing::SubjectDeclared,
                    first_at_ms: at_ms,
                    last_at_ms: at_ms,
                    evidence: Vec::new(),
                });
            }
        }
        true
    }

    /// Removes an expectation the owner says is wrong.
    pub fn retract(&mut self, person: &str, key: &str) -> bool {
        let Some(id) = self.find(person).map(str::to_string) else {
            return false;
        };
        let Some(p) = self.people.get_mut(&id) else {
            return false;
        };
        let before = p.expectations.len();
        p.expectations.retain(|e| e.key != key);
        p.expectations.len() != before
    }

    /// Forgets a person entirely. The owner cannot be forgotten from their own state.
    pub fn forget(&mut self, person: &str) -> bool {
        let Some(id) = self.find(person).map(str::to_string) else {
            return false;
        };
        if id == self.owner {
            return false;
        }
        self.people.remove(&id).is_some()
    }

    fn with_expectation(
        &mut self,
        person: &str,
        key: &str,
        f: impl FnOnce(&mut Expectation),
    ) -> bool {
        let Some(id) = self.find(person).map(str::to_string) else {
            return false;
        };
        match self
            .people
            .get_mut(&id)
            .and_then(|p| p.expectations.iter_mut().find(|e| e.key == key))
        {
            Some(e) => {
                f(e);
                true
            }
            None => false,
        }
    }

    /// The state as a model reads it ("pack v3"): people, what each has seen, what to expect,
    /// moods. People in `focus` first, then the most recently active; cut at a line boundary
    /// to `max_bytes`.
    pub fn render(&self, focus: &[&str], max_bytes: usize, lang: Lang) -> String {
        let words = Words::for_lang(lang);
        let mut order: Vec<&PersonState> = Vec::new();
        for name in focus {
            if let Some(p) = self.find(name).and_then(|id| self.people.get(id)) {
                if !order.iter().any(|q| q.id == p.id) {
                    order.push(p);
                }
            }
        }
        let mut rest: Vec<&PersonState> = self
            .people
            .values()
            .filter(|p| !order.iter().any(|q| q.id == p.id))
            .collect();
        rest.sort_by(|a, b| b.last_turn.cmp(&a.last_turn).then_with(|| a.id.cmp(&b.id)));
        order.extend(rest);

        let mut lines = vec![format!(
            "{} {}",
            words.people,
            order
                .iter()
                .map(|p| p.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )];
        lines.push(words.seen.to_string());
        for p in &order {
            for (topic, fact) in &self.facts {
                let Some(current) = fact.current() else {
                    continue;
                };
                let line = match p.seen.get(topic) {
                    None => format!("- {}: {} {topic}.", p.label, words.never),
                    Some(s) if s.value == current.value => {
                        format!("- {}: {} {topic} = {}.", p.label, words.knows, s.value)
                    }
                    Some(s) => format!(
                        "- {}: {} {topic} = {}; {} ({} {}).",
                        p.label, words.saw, s.value, words.missed, words.now, current.value
                    ),
                };
                lines.push(line);
            }
        }
        lines.push(words.expect.to_string());
        for p in &order {
            for e in p.what_to_expect().into_iter().take(6) {
                let standing = match e.standing {
                    Standing::Inferred => words.inferred,
                    Standing::OwnerConfirmed => words.confirmed,
                    Standing::SubjectDeclared => words.declared,
                };
                lines.push(format!(
                    "- {}: {} ({}) — {} {:.2}, {} {:.0}, {standing}.",
                    p.label,
                    e.statement,
                    e.key,
                    words.confidence,
                    e.confidence(),
                    words.evidence,
                    e.weight()
                ));
            }
            if let Some(latency) = p
                .behaviour
                .reply_latency_ms
                .filter(|_| p.behaviour.messages >= 3)
            {
                lines.push(format!(
                    "- {}: {} {}.",
                    p.label,
                    words.replies,
                    human_duration(latency, lang)
                ));
            }
        }
        let moods: Vec<String> = order
            .iter()
            .filter_map(|p| {
                p.affect
                    .as_ref()
                    .map(|a| format!("- {}: {} ({}).", p.label, a.label, a.intensity))
            })
            .collect();
        if !moods.is_empty() {
            lines.push(words.mood.to_string());
            lines.extend(moods);
        }
        let mut out = String::new();
        for line in lines {
            if out.len() + line.len() + 1 > max_bytes {
                if out.is_empty() {
                    out.push_str(clip_bytes(&line, max_bytes));
                }
                break;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&line);
        }
        out
    }
}

fn push_evidence(evidence: &mut Vec<String>, item: String) {
    if item.is_empty() || evidence.contains(&item) {
        return;
    }
    evidence.push(item);
    if evidence.len() > MAX_EVIDENCE {
        evidence.remove(0);
    }
}

pub(crate) fn human_duration(ms: f64, lang: Lang) -> String {
    let minutes = ms / 60_000.0;
    match (lang, minutes) {
        (Lang::Es, m) if m < 1.0 => "menos de un minuto".into(),
        (Lang::En, m) if m < 1.0 => "under a minute".into(),
        (Lang::Es, m) if m < 90.0 => format!("~{m:.0} min"),
        (Lang::En, m) if m < 90.0 => format!("~{m:.0} min"),
        (Lang::Es, m) => format!("~{:.0} h", m / 60.0),
        (Lang::En, m) => format!("~{:.0} h", m / 60.0),
    }
}

struct Words {
    people: &'static str,
    seen: &'static str,
    never: &'static str,
    knows: &'static str,
    saw: &'static str,
    missed: &'static str,
    now: &'static str,
    expect: &'static str,
    confidence: &'static str,
    evidence: &'static str,
    inferred: &'static str,
    confirmed: &'static str,
    declared: &'static str,
    replies: &'static str,
    mood: &'static str,
}

impl Words {
    fn for_lang(lang: Lang) -> Self {
        match lang {
            Lang::Es => Self {
                people: "Personas:",
                seen: "Lo que cada persona ha visto:",
                never: "nunca vio nada sobre",
                knows: "sabe que",
                saw: "vio que",
                missed: "no vio el cambio",
                now: "ahora:",
                expect: "Qué esperar:",
                confidence: "confianza",
                evidence: "evidencias",
                inferred: "inferido",
                confirmed: "confirmado por el dueño",
                declared: "declarado por la persona",
                replies: "suele responder en",
                mood: "Ánimo:",
            },
            Lang::En => Self {
                people: "People:",
                seen: "What each person has seen:",
                never: "never saw anything about",
                knows: "knows that",
                saw: "saw that",
                missed: "missed the change",
                now: "now:",
                expect: "What to expect:",
                confidence: "confidence",
                evidence: "evidence",
                inferred: "inferred",
                confirmed: "confirmed by the owner",
                declared: "declared by the person",
                replies: "usually replies in",
                mood: "Mood:",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(turns: &[(u64, &str, &[&str])]) -> ChunkContext {
        let mut c = ChunkContext::default();
        for &(t, speaker, present) in turns {
            c.presence
                .insert(t, present.iter().map(|s| s.to_string()).collect());
            c.evidence.insert(t, format!("ev-{t}"));
            c.speakers.insert(t, speaker.to_string());
            c.at_ms.insert(t, t as i64 * 60_000);
        }
        c
    }

    fn state() -> SocialState {
        let mut s = SocialState::new("owner", "Yo").unwrap();
        for (id, label) in [("pedro", "Pedro"), ("ana", "Ana María"), ("luis", "Luis")] {
            s.ensure_person(id, label, 1);
        }
        s
    }

    #[test]
    fn delta_lines_parse_and_print_back() {
        let text = "fact el viaje = julio @12\ntold \"Ana María\" el viaje = julio @13\nexpect Pedro + late: suele llegar tarde\nexpect Pedro - late\naffect Luis molesto 70\nnone\nbasura aquí\nexpect Pedro ? late";
        let (lines, rejected) = parse_delta(text);
        assert_eq!(rejected, 2);
        assert_eq!(lines.len(), 6);
        assert_eq!(
            lines[0],
            DeltaLine::Fact {
                topic: "el viaje".into(),
                value: "julio".into(),
                turn: 12
            }
        );
        assert_eq!(
            lines[1],
            DeltaLine::Told {
                person: "Ana María".into(),
                topic: "el viaje".into(),
                value: "julio".into(),
                turn: 13
            }
        );
        for line in &lines {
            let (again, bad) = parse_delta(&line.to_string());
            assert_eq!(bad, 0);
            assert_eq!(&again[0], line);
        }
    }

    #[test]
    fn who_knows_is_exact_from_presence() {
        let mut s = state();
        let c = chunk(&[
            (1, "pedro", &["pedro", "owner", "luis"]),
            (2, "luis", &["owner", "luis"]),
        ]);
        let report = s.apply(
            &[
                DeltaLine::Fact {
                    topic: "el viaje".into(),
                    value: "mayo".into(),
                    turn: 1,
                },
                DeltaLine::Fact {
                    topic: "el viaje".into(),
                    value: "julio".into(),
                    turn: 2,
                },
            ],
            &c,
        );
        assert_eq!(
            report,
            ApplyReport {
                applied: 2,
                rejected: 0
            }
        );
        assert_eq!(s.knows("Luis", "el viaje"), Some(true));
        assert_eq!(s.knows("Pedro", "el viaje"), Some(false));
        assert_eq!(s.belief("Pedro", "el viaje"), Some("mayo"));
        assert_eq!(s.knows("Ana María", "el viaje"), Some(false));
        assert_eq!(s.belief("ana", "el viaje"), None);
        // Telling someone is exposure too.
        let c2 = chunk(&[(3, "luis", &["ana", "luis", "owner"])]);
        s.apply(
            &[DeltaLine::Told {
                person: "Ana María".into(),
                topic: "el viaje".into(),
                value: "julio".into(),
                turn: 3,
            }],
            &c2,
        );
        assert_eq!(s.knows("ana", "el viaje"), Some(true));
    }

    #[test]
    fn invented_people_and_turns_are_refused() {
        let mut s = state();
        let c = chunk(&[(5, "pedro", &["pedro"])]);
        let report = s.apply(
            &[
                DeltaLine::Fact {
                    topic: "x".into(),
                    value: "y".into(),
                    turn: 99,
                },
                DeltaLine::Expect {
                    person: "Rodrigo".into(),
                    positive: true,
                    key: "late".into(),
                    statement: Some("llega tarde".into()),
                },
                DeltaLine::Expect {
                    person: "Pedro".into(),
                    positive: false,
                    key: "late".into(),
                    statement: None,
                },
                DeltaLine::Affect {
                    person: "Luis".into(),
                    label: "molesto".into(),
                    intensity: 50,
                },
            ],
            &c,
        );
        // Wrong turn, unknown person, a contradiction of nothing, a mood with no message.
        assert_eq!(
            report,
            ApplyReport {
                applied: 0,
                rejected: 4
            }
        );
    }

    #[test]
    fn expectations_accumulate_decay_and_cap() {
        let mut s = state();
        for t in 1..=40u64 {
            let c = chunk(&[(t, "pedro", &["pedro", "owner"])]);
            s.apply(
                &[DeltaLine::Expect {
                    person: "pedro".into(),
                    positive: true,
                    key: "late".into(),
                    statement: Some("llega tarde".into()),
                }],
                &c,
            );
        }
        let e = &s.people["pedro"].expectations[0];
        assert!(
            (e.confidence() - MAX_INFERRED_CONFIDENCE).abs() < 1e-6,
            "capped: {}",
            e.confidence()
        );
        let c = chunk(&[(41, "pedro", &["pedro", "owner"])]);
        s.apply(
            &[DeltaLine::Expect {
                person: "pedro".into(),
                positive: false,
                key: "late".into(),
                statement: None,
            }],
            &c,
        );
        assert!(s.people["pedro"].expectations[0].contradict > 0.0);
        // A year later, the old evidence has faded: one contradiction weighs more.
        let mut old = s.people["pedro"].expectations[0].clone();
        let w = old.weight();
        old.decay(old.last_at_ms + 365 * 24 * 3_600_000, DEFAULT_HALF_LIFE_MS);
        assert!(old.weight() < w / 10.0);
        // Declared by the person outranks inference; the owner cannot forget themselves.
        assert!(s.declare("pedro", "late", "Llego tarde, lo reconozco", 0));
        assert!(
            (s.people["pedro"].expectations[0].confidence() - DECLARED_CONFIDENCE).abs() < 1e-6
        );
        assert!(!s.forget("owner"));
        assert!(s.retract("pedro", "late"));
        assert!(s.forget("Luis"));
    }

    #[test]
    fn render_puts_focus_first_and_respects_budget() {
        let mut s = state();
        let c = chunk(&[(1, "luis", &["luis", "owner"])]);
        s.apply(
            &[DeltaLine::Fact {
                topic: "el viaje".into(),
                value: "julio".into(),
                turn: 1,
            }],
            &c,
        );
        s.apply(
            &[DeltaLine::Expect {
                person: "luis".into(),
                positive: true,
                key: "direct".into(),
                statement: Some("dice las cosas claras".into()),
            }],
            &c,
        );
        let text = s.render(&["Pedro"], 4096, Lang::Es);
        assert!(text.starts_with("Personas: Pedro,"));
        assert!(text.contains("- Pedro: nunca vio nada sobre el viaje."));
        assert!(text.contains("- Luis: sabe que el viaje = julio."));
        assert!(text.contains("dice las cosas claras (direct)"));
        let short = s.render(&[], 40, Lang::En);
        assert!(short.len() <= 40);
        let back = SocialState::from_json(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
