//! Reading a conversation of any length in chunks.
//!
//! For each chunk, in order:
//!
//! 1. the state as it was is rendered for the people in the chunk;
//! 2. the chunk's turns are applied to the bookkeeping (joins, leaves, messages), and who was
//!    present for each message is recorded;
//! 3. the interpreter is asked whether the chunk changes anything and, if so, for delta lines;
//! 4. the lines are validated against the chunk and applied.
//!
//! Memory is the state, whatever the length of the conversation. Turns already read (by
//! number) are skipped, so the same events can be offered again safely.

use crate::model::Lncm;
use crate::state::{parse_delta, ChunkContext, SocialState};
use crate::{bounded_text, Lang, LncmError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TurnKind {
    Message { speaker: String, text: String },
    Join { person: String },
    Leave { person: String },
}

/// One event of a conversation. `speaker`/`person` are person ids in the state; a new one is
/// added with `label` as its display name (the id when absent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub turn: u64,
    pub at_ms: i64,
    pub chat: String,
    pub kind: TurnKind,
    /// Where the event is kept (a Vault reference at runtime), cited by what it supports.
    pub evidence: String,
    #[serde(default)]
    pub label: Option<String>,
}

impl Turn {
    fn person(&self) -> &str {
        match &self.kind {
            TurnKind::Message { speaker, .. } => speaker,
            TurnKind::Join { person } | TurnKind::Leave { person } => person,
        }
    }

    /// The line a model reads for this turn.
    pub fn render(&self, state: &SocialState, lang: Lang) -> String {
        let name = state
            .people
            .get(self.person())
            .map(|p| p.label.as_str())
            .or(self.label.as_deref())
            .unwrap_or(self.person());
        match (&self.kind, lang) {
            (TurnKind::Message { text, .. }, _) => {
                format!("[{}] {name}: {}", self.turn, text.replace('\n', " "))
            }
            (TurnKind::Join { .. }, Lang::Es) => format!("[{}] {name} se unió.", self.turn),
            (TurnKind::Join { .. }, Lang::En) => format!("[{}] {name} joined.", self.turn),
            (TurnKind::Leave { .. }, Lang::Es) => format!("[{}] {name} salió.", self.turn),
            (TurnKind::Leave { .. }, Lang::En) => format!("[{}] {name} left.", self.turn),
        }
    }
}

/// What turns a chunk into delta lines.
pub trait Interpreter {
    /// Probability that the chunk changes the state, and the delta lines when it does.
    fn interpret(&self, state: &str, chunk: &str) -> Result<(f32, Option<String>)>;
}

/// The LNCM as the reader's interpreter.
pub struct LncmInterpreter<'a> {
    pub model: &'a Lncm,
    pub lang: Lang,
    pub threshold: f32,
}

impl Interpreter for LncmInterpreter<'_> {
    fn interpret(&self, state: &str, chunk: &str) -> Result<(f32, Option<String>)> {
        let out = self.model.update(self.lang, state, chunk, self.threshold)?;
        Ok((out.change, out.delta))
    }
}

/// Any function can interpret: synthetic worlds use their exact labels this way.
impl<F> Interpreter for F
where
    F: Fn(&str, &str) -> Result<(f32, Option<String>)>,
{
    fn interpret(&self, state: &str, chunk: &str) -> Result<(f32, Option<String>)> {
        self(state, chunk)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ReaderConfig {
    pub lang: Lang,
    /// Largest chunk, in bytes of rendered turns.
    pub chunk_bytes: usize,
    pub max_turns: usize,
    /// Largest rendering of the state given with each chunk.
    pub state_bytes: usize,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            lang: Lang::Es,
            // With an 8k BPE codec (~3.5 bytes a token) a chunk is ~300 tokens and the state up
            // to ~450: both fit the small preset's 768-token prompt with the markers.
            chunk_bytes: 1024,
            max_turns: 24,
            state_bytes: 1536,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadReport {
    pub turns: usize,
    pub already_read: usize,
    pub chunks: usize,
    /// Chunks the interpreter said change nothing.
    pub unchanged: usize,
    pub applied: usize,
    pub rejected: usize,
}

/// One chunk as the interpreter saw it, for building training data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub state: String,
    pub chunk: String,
    /// The interpreter's probability that the chunk changes the state.
    pub change: f32,
    pub changed: bool,
    pub delta: Option<String>,
}

pub fn read(
    state: &mut SocialState,
    turns: &[Turn],
    interpreter: &dyn Interpreter,
    config: &ReaderConfig,
) -> Result<ReadReport> {
    read_recording(state, turns, interpreter, config, &mut |_| {})
}

/// [`read`], handing every chunk to `record` as the interpreter saw it.
pub fn read_recording(
    state: &mut SocialState,
    turns: &[Turn],
    interpreter: &dyn Interpreter,
    config: &ReaderConfig,
    record: &mut dyn FnMut(ChunkRecord),
) -> Result<ReadReport> {
    if config.chunk_bytes < 64 || config.max_turns == 0 || config.state_bytes < 64 {
        return Err(LncmError::Config(
            "reader chunk or state budget too small".into(),
        ));
    }
    let mut report = ReadReport {
        turns: turns.len(),
        ..Default::default()
    };
    let mut pending: Vec<&Turn> = turns
        .iter()
        .filter(|t| {
            let fresh = t.turn > state.last_turn;
            if !fresh {
                report.already_read += 1;
            }
            fresh
        })
        .collect();
    pending.sort_by_key(|t| t.turn);
    pending.dedup_by_key(|t| t.turn);
    for t in &pending {
        validate_turn(t)?;
    }
    let mut index = 0;
    let mut last_writer: BTreeMap<String, (String, i64)> = BTreeMap::new();
    while index < pending.len() {
        // Gather the chunk first (people are added so their names render).
        let mut chunk_turns = Vec::new();
        let mut size = 0;
        while index < pending.len() && chunk_turns.len() < config.max_turns {
            let t = pending[index];
            let label = t.label.clone().unwrap_or_else(|| t.person().to_string());
            state.ensure_person(t.person(), &label, t.turn);
            let line = t.render(state, config.lang);
            if !chunk_turns.is_empty() && size + line.len() + 1 > config.chunk_bytes {
                break;
            }
            size += line.len() + 1;
            chunk_turns.push((t, line));
            index += 1;
        }
        let focus: Vec<&str> = chunk_turns.iter().map(|(t, _)| t.person()).collect();
        let before = state.render(&focus, config.state_bytes, config.lang);
        let text = chunk_turns
            .iter()
            .map(|(_, l)| crate::clip_bytes(l, config.chunk_bytes))
            .collect::<Vec<_>>()
            .join("\n");
        let mut context = ChunkContext::default();
        for (t, _) in &chunk_turns {
            match &t.kind {
                TurnKind::Join { person } => state.join(person, &t.chat, t.turn),
                TurnKind::Leave { person } => state.leave(person, &t.chat, t.turn),
                TurnKind::Message { speaker, .. } => {
                    let previous = last_writer
                        .get(&t.chat)
                        .filter(|(who, _)| who != speaker)
                        .map(|(_, at)| *at);
                    state.message(speaker, &t.chat, t.turn, t.at_ms, previous);
                    last_writer.insert(t.chat.clone(), (speaker.clone(), t.at_ms));
                    context.speakers.insert(t.turn, speaker.clone());
                    context.presence.insert(t.turn, state.present_in(&t.chat));
                }
            }
            context.evidence.insert(t.turn, t.evidence.clone());
            context.at_ms.insert(t.turn, t.at_ms);
            if let Some(p) = state.people.get_mut(t.person()) {
                p.last_turn = p.last_turn.max(t.turn);
            }
            state.last_turn = state.last_turn.max(t.turn);
            state.last_at_ms = state.last_at_ms.max(t.at_ms);
        }
        report.chunks += 1;
        let (change, delta) = interpreter.interpret(&before, &text)?;
        let delta = delta.filter(|d| !d.trim().is_empty());
        record(ChunkRecord {
            state: before,
            chunk: text,
            change,
            changed: delta.as_deref().is_some_and(|d| d.trim() != "none"),
            delta: delta.clone(),
        });
        match delta {
            None => report.unchanged += 1,
            Some(delta) => {
                let (lines, bad) = parse_delta(&delta);
                let applied = state.apply(&lines, &context);
                report.applied += applied.applied;
                report.rejected += applied.rejected + bad;
            }
        }
    }
    Ok(report)
}

fn validate_turn(t: &Turn) -> Result<()> {
    let ok_id = |s: &str| bounded_text(s, 128) && !s.contains('\n');
    let valid = ok_id(&t.chat)
        && ok_id(t.person())
        && bounded_text(&t.evidence, 256)
        && t.label.as_deref().is_none_or(ok_id)
        && t.at_ms >= 0
        && match &t.kind {
            TurnKind::Message { text, .. } => bounded_text(text, 4000),
            _ => true,
        };
    if valid {
        Ok(())
    } else {
        Err(LncmError::Input("invalid_turn"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(turn: u64, speaker: &str, text: &str) -> Turn {
        Turn {
            turn,
            at_ms: turn as i64 * 60_000,
            chat: "grupo".into(),
            kind: TurnKind::Message {
                speaker: speaker.into(),
                text: text.into(),
            },
            evidence: format!("ev-{turn}"),
            label: None,
        }
    }

    fn join(turn: u64, person: &str) -> Turn {
        Turn {
            turn,
            at_ms: turn as i64 * 60_000,
            chat: "grupo".into(),
            kind: TurnKind::Join {
                person: person.into(),
            },
            evidence: format!("ev-{turn}"),
            label: None,
        }
    }

    #[test]
    fn exact_interpreter_yields_exact_knowledge_with_bounded_chunks() {
        let mut state = SocialState::new("owner", "Yo").unwrap();
        let mut turns = vec![join(1, "ana"), join(2, "luis")];
        turns.push(message(3, "ana", "El viaje es en mayo."));
        turns.push(join(4, "pedro"));
        for t in 5..60 {
            turns.push(message(t, "luis", "jaja"));
        }
        turns.push(message(
            60,
            "luis",
            "Cambio de planes: el viaje es en julio.",
        ));
        // An interpreter that knows exactly which messages state a fact.
        let exact = |_: &str, chunk: &str| -> Result<(f32, Option<String>)> {
            let mut lines = Vec::new();
            for line in chunk.lines() {
                let turn: u64 = line[1..line.find(']').unwrap()].parse().unwrap();
                if line.contains("en mayo") {
                    lines.push(format!("fact el viaje = mayo @{turn}"));
                }
                if line.contains("en julio") {
                    lines.push(format!("fact el viaje = julio @{turn}"));
                }
            }
            Ok(if lines.is_empty() {
                (0.0, None)
            } else {
                (1.0, Some(lines.join("\n")))
            })
        };
        let config = ReaderConfig {
            chunk_bytes: 200,
            ..Default::default()
        };
        let mut records = Vec::new();
        let report = read_recording(&mut state, &turns, &exact, &config, &mut |r| {
            records.push(r)
        })
        .unwrap();
        assert!(report.chunks > 3);
        assert_eq!(report.applied, 2);
        assert_eq!(report.rejected, 0);
        assert!(records.iter().all(|r| r.chunk.len() <= 200));
        assert_eq!(records.iter().filter(|r| r.changed).count(), 2);
        // Pedro joined after May and was there for July; Ana was there for both.
        assert_eq!(state.knows("pedro", "el viaje"), Some(true));
        assert_eq!(state.knows("ana", "el viaje"), Some(true));
        assert_eq!(state.knows("owner", "el viaje"), Some(true));
        // Reading the same turns again changes nothing.
        let again = read(&mut state, &turns, &exact, &config).unwrap();
        assert_eq!(again.already_read, turns.len());
        assert_eq!(again.chunks, 0);
        // Reply latency is bookkept.
        assert!(state.people["luis"].behaviour.reply_latency_ms.is_some());
    }

    #[test]
    fn a_person_who_left_misses_the_update() {
        let mut state = SocialState::new("owner", "Yo").unwrap();
        let mut turns = vec![
            join(1, "ana"),
            join(2, "pedro"),
            message(3, "ana", "El viaje es en mayo."),
        ];
        turns.push(Turn {
            kind: TurnKind::Leave {
                person: "pedro".into(),
            },
            ..join(4, "pedro")
        });
        turns.push(message(5, "ana", "Ahora el viaje es en julio."));
        let exact = |_: &str, chunk: &str| -> Result<(f32, Option<String>)> {
            let mut lines = Vec::new();
            for line in chunk.lines() {
                let turn: u64 = line[1..line.find(']').unwrap()].parse().unwrap();
                for month in ["mayo", "julio"] {
                    if line.contains(&format!("en {month}")) {
                        lines.push(format!("fact el viaje = {month} @{turn}"));
                    }
                }
            }
            Ok((1.0, Some(lines.join("\n"))))
        };
        read(&mut state, &turns, &exact, &ReaderConfig::default()).unwrap();
        assert_eq!(state.knows("pedro", "el viaje"), Some(false));
        assert_eq!(state.belief("pedro", "el viaje"), Some("mayo"));
    }

    #[test]
    fn malformed_turns_are_refused() {
        let mut state = SocialState::new("owner", "Yo").unwrap();
        let bad = Turn {
            chat: "".into(),
            ..message(1, "ana", "hola")
        };
        let never = |_: &str, _: &str| -> Result<(f32, Option<String>)> { Ok((0.0, None)) };
        assert!(read(&mut state, &[bad], &never, &ReaderConfig::default()).is_err());
    }
}
