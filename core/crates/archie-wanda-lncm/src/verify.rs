//! Checks of an answer against the state it was written from.
//!
//! The state a model reads is rendered text ([`crate::state::SocialState::render`]); its lines
//! say, per person and topic, the value the person saw and the value the topic has now.
//! [`slots`] reads them back. [`check`] then looks at every value an answer names:
//!
//! - who it is about: the last person named before it, in its sentence or an earlier one, else
//!   the person the question names;
//! - what it claims, from the words just before it: what the person believes ("cree",
//!   "sigue", "still", "was") or what the topic is now ("ahora", "quedó", "now", "knows");
//! - and replaces a value that is not that one, when both are clear.
//!
//! What it cannot fix is a problem, and an answer with a problem is not spoken: a topic the
//! state does not hold, a value nobody saw for that person, an answer that never mentions
//! what was asked. The [`Lexicon`] (every topic and value the training states held) is how a
//! value or a topic the state lacks is recognised at all.

use crate::exact::find_phrase;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One person and topic of a rendered state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub person: String,
    pub topic: String,
    /// The last value the person saw; `None` when they saw nothing about the topic.
    pub seen: Option<String>,
    pub current: String,
}

const KNOWS: [&str; 2] = ["sabe que ", "knows that "];
const SAW: [&str; 2] = ["vio que ", "saw that "];
const NEVER: [&str; 2] = ["nunca vio nada sobre ", "never saw anything about "];
const NOW: [&str; 2] = ["(ahora: ", "(now: "];

/// The person/topic lines of a rendered state, in either language.
pub fn slots(state: &str) -> Vec<Slot> {
    let mut out: Vec<Slot> = Vec::new();
    for line in state.lines() {
        let Some((person, rest)) = line.strip_prefix("- ").and_then(|l| l.split_once(": ")) else {
            continue;
        };
        let strip_point = |s: &str| s.strip_suffix('.').unwrap_or(s).trim().to_string();
        if let Some(rest) = KNOWS.iter().find_map(|k| rest.strip_prefix(k)) {
            if let Some((topic, value)) = rest.split_once(" = ") {
                let value = strip_point(value);
                out.push(Slot {
                    person: person.to_string(),
                    topic: topic.to_string(),
                    seen: Some(value.clone()),
                    current: value,
                });
            }
        } else if let Some(rest) = SAW.iter().find_map(|k| rest.strip_prefix(k)) {
            let Some((topic, rest)) = rest.split_once(" = ") else {
                continue;
            };
            let Some((seen, rest)) = rest.split_once("; ") else {
                continue;
            };
            let Some(current) = NOW.iter().find_map(|n| {
                let start = rest.find(n)? + n.len();
                let end = rest.rfind(')')?;
                (end > start).then(|| rest[start..end].trim().to_string())
            }) else {
                continue;
            };
            out.push(Slot {
                person: person.to_string(),
                topic: topic.to_string(),
                seen: Some(seen.trim().to_string()),
                current,
            });
        } else if let Some(topic) = NEVER.iter().find_map(|k| rest.strip_prefix(k)) {
            out.push(Slot {
                person: person.to_string(),
                topic: strip_point(topic),
                seen: None,
                current: String::new(),
            });
        }
    }
    // A "never saw" line does not say the current value; another line of the topic does.
    let current: BTreeMap<String, String> = out
        .iter()
        .filter(|s| !s.current.is_empty())
        .map(|s| (s.topic.clone(), s.current.clone()))
        .collect();
    for s in &mut out {
        if s.current.is_empty() {
            s.current = current.get(&s.topic).cloned().unwrap_or_default();
        }
    }
    out
}

const EXPECT_CONFIDENCE: [&str; 2] = [" — confianza ", " — confidence "];

/// The "what to expect" lines of a rendered state: person → (statement, confidence).
pub fn expectations(state: &str) -> BTreeMap<String, Vec<(String, f32)>> {
    let mut out: BTreeMap<String, Vec<(String, f32)>> = BTreeMap::new();
    for line in state.lines() {
        let Some((person, rest)) = line.strip_prefix("- ").and_then(|l| l.split_once(": ")) else {
            continue;
        };
        let Some((head, tail)) = EXPECT_CONFIDENCE.iter().find_map(|c| rest.split_once(c)) else {
            continue;
        };
        let statement = head.rsplit_once(" (").map_or(head, |(s, _)| s).trim();
        let Ok(confidence) = tail.split(',').next().unwrap_or("").trim().parse::<f32>() else {
            continue;
        };
        out.entry(person.to_string())
            .or_default()
            .push((statement.to_lowercase(), confidence));
    }
    out
}

/// Every topic and its values, and every expectation's statement, that the training states
/// held, lowercased.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lexicon {
    pub topics: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub statements: BTreeSet<String>,
}

impl Lexicon {
    pub fn add_state(&mut self, state: &str) {
        for s in slots(state) {
            let values = self.topics.entry(s.topic.to_lowercase()).or_default();
            if let Some(seen) = &s.seen {
                values.insert(seen.to_lowercase());
            }
            if !s.current.is_empty() {
                values.insert(s.current.to_lowercase());
            }
        }
        for list in expectations(state).into_values() {
            self.statements.extend(list.into_iter().map(|(s, _)| s));
        }
    }
}

/// Words that say a trait does not hold, or only weakly.
const AGAINST: &[&str] = &[
    "no",
    "nunca",
    "poco probable",
    "va en contra",
    "not",
    "never",
    "unlikely",
    "goes against",
    "rarely",
];

/// What [`check`] found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// The answer with its values corrected.
    pub text: String,
    /// (what it said, what it says now).
    pub corrected: Vec<(String, String)>,
    /// Why the answer should not be spoken; empty when it may be.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Claim {
    Believes,
    Now,
    Either,
}

const BELIEVES: &[&str] = &[
    "cree", "creía", "creyendo", "piensa", "pensando", "sigue", "todavía", "aún", "vio", "era",
    "believes", "believe", "thinks", "think", "still", "saw", "was",
];
const NOW_WORDS: &[&str] = &[
    "ahora", "quedó", "cambió", "actual", "sabe", "now", "moved", "changed", "knows", "current",
];

/// Checks `answer` against `state` (rendered) for `question`. See the module documentation.
pub fn check(answer: &str, question: &str, state: &str, lexicon: Option<&Lexicon>) -> Verdict {
    let slots = slots(state);
    let lower = answer.to_lowercase();
    let mut verdict = Verdict {
        text: answer.to_string(),
        ..Default::default()
    };
    // Offsets of `lower` are offsets of `answer` only when lowercasing kept every length.
    let can_edit = lower.len() == answer.len();
    let people: BTreeSet<&str> = slots.iter().map(|s| s.person.as_str()).collect();
    let mut topics: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in &slots {
        let values = topics.entry(s.topic.to_lowercase()).or_default();
        if let Some(seen) = &s.seen {
            values.insert(seen.to_lowercase());
        }
        if !s.current.is_empty() {
            values.insert(s.current.to_lowercase());
        }
    }
    // Values the lexicon knows for a topic this state holds count too: the state never had
    // them, which is what makes them wrong.
    let mut known_values: Vec<(String, String)> = Vec::new();
    for (topic, values) in &topics {
        let mut all: BTreeSet<String> = values.clone();
        if let Some(extra) = lexicon.and_then(|l| l.topics.get(topic)) {
            all.extend(extra.iter().cloned());
        }
        known_values.extend(all.into_iter().map(|v| (topic.clone(), v)));
    }
    known_values.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    // A topic the lexicon knows, named with one of its values, that this state does not hold.
    if let Some(lexicon) = lexicon {
        for (topic, values) in &lexicon.topics {
            if topics.contains_key(topic) || find_phrase(&lower, topic).is_none() {
                continue;
            }
            if values.iter().any(|v| find_phrase(&lower, v).is_some()) {
                verdict
                    .problems
                    .push(format!("habla de {topic}, que no está en el estado"));
            }
        }
    }

    // The question's topic must be answered.
    let q = question.to_lowercase();
    if let Some(asked) = topics
        .keys()
        .filter(|t| find_phrase(&q, t).is_some())
        .max_by_key(|t| t.len())
    {
        let named = find_phrase(&lower, asked).is_some()
            || topics[asked]
                .iter()
                .any(|v| find_phrase(&lower, v).is_some());
        if !named {
            verdict
                .problems
                .push(format!("no habla de {asked}, que es lo preguntado"));
        }
    }
    let asked_person = people
        .iter()
        .filter(|p| find_phrase(&q, &p.to_lowercase()).is_some())
        .max_by_key(|p| p.len())
        .copied();

    // Every person mention, in order.
    let mut mentions: Vec<(usize, usize, &str)> = Vec::new();
    for p in &people {
        let needle = p.to_lowercase();
        let mut from = 0;
        while let Some((s, e)) = find_phrase(&lower[from..], &needle) {
            mentions.push((from + s, from + e, p));
            from += e;
        }
    }
    mentions.sort();

    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (topic, value) in &known_values {
        let mut from = 0;
        while let Some((s, e)) = find_phrase(&lower[from..], value) {
            let (s, e) = (from + s, from + e);
            from = e;
            if taken.iter().any(|&(a, b)| s < b && a < e) {
                continue;
            }
            taken.push((s, e));
            let subject = mentions
                .iter()
                .rev()
                .find(|(_, end, _)| *end <= s)
                .map(|(_, end, p)| (*end, *p))
                .or(asked_person.map(|p| (0, p)));
            let Some((after, person)) = subject else {
                continue;
            };
            let sentence_start = lower[..s]
                .rfind(['!', '?', ';', '\n', ':'])
                .map_or(0, |i| i + 1)
                .max(sentence_boundary(&lower[..s]));
            let window = &lower[after.max(sentence_start).min(s)..s];
            let claim = claim(window);
            let Some(slot) = slots
                .iter()
                .find(|x| x.person == person && x.topic.to_lowercase() == *topic)
            else {
                continue;
            };
            let expected = match claim {
                Claim::Believes => slot.seen.clone(),
                Claim::Now => Some(slot.current.clone()),
                Claim::Either => {
                    let ok = slot
                        .seen
                        .as_ref()
                        .is_some_and(|v| v.to_lowercase() == *value)
                        || slot.current.to_lowercase() == *value;
                    if ok {
                        continue;
                    }
                    None
                }
            };
            match expected {
                Some(x) if x.to_lowercase() == *value => {}
                Some(x) if can_edit => {
                    verdict
                        .corrected
                        .push((answer[s..e].to_string(), x.clone()));
                    edits.push((s, e, x));
                }
                _ => verdict.problems.push(format!(
                    "{} no corresponde a lo que {person} vio de {topic}",
                    &lower[s..e]
                )),
            }
        }
    }
    // Traits: one the state does not hold for the person it is said of, or one whose evidence
    // is against it said without saying so.
    let held = expectations(state);
    let mut statements: BTreeSet<String> =
        lexicon.map(|l| l.statements.clone()).unwrap_or_default();
    statements.extend(held.values().flatten().map(|(s, _)| s.clone()));
    for statement in &statements {
        let mut from = 0;
        while let Some((s, e)) = find_phrase(&lower[from..], statement) {
            let (s, e) = (from + s, from + e);
            from = e;
            let Some(person) = mentions
                .iter()
                .rev()
                .find(|(_, end, _)| *end <= s)
                .map(|(_, _, p)| *p)
                .or(asked_person)
            else {
                continue;
            };
            let sentence = sentence_around(&lower, s, e);
            let against = AGAINST.iter().any(|w| find_phrase(sentence, w).is_some());
            match held
                .get(person)
                .and_then(|l| l.iter().find(|(x, _)| x == statement))
            {
                None => verdict
                    .problems
                    .push(format!("atribuye «{statement}» a {person} sin evidencia")),
                Some((_, c)) if *c < 0.5 && !against => verdict.problems.push(format!(
                    "«{statement}» de {person} tiene la evidencia en contra ({c:.2})"
                )),
                Some(_) => {}
            }
        }
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    for (s, e, x) in edits {
        verdict.text.replace_range(s..e, &x);
    }
    verdict
}

/// The sentence of `text` holding `[s, e)`.
fn sentence_around(text: &str, s: usize, e: usize) -> &str {
    let start = text[..s]
        .rfind(['!', '?', ';', '\n'])
        .map_or(0, |i| i + 1)
        .max(sentence_boundary(&text[..s]));
    let end = text[e..]
        .find(['!', '?', ';', '\n'])
        .map_or(text.len(), |i| e + i);
    &text[start..end]
}

/// Where the sentence holding the end of `text` starts: after the last `. ` that is not a
/// decimal point.
fn sentence_boundary(text: &str) -> usize {
    let bytes = text.as_bytes();
    (1..bytes.len())
        .rev()
        .find(|&i| bytes[i - 1] == b'.' && bytes[i].is_ascii_whitespace())
        .unwrap_or(0)
}

/// What the words before a value say it is: the last cue wins.
fn claim(window: &str) -> Claim {
    let last = |cues: &[&str]| {
        cues.iter()
            .filter_map(|c| {
                let mut at = None;
                let mut from = 0;
                while let Some((_, e)) = find_phrase(&window[from..], c) {
                    at = Some(from + e);
                    from += e;
                }
                at
            })
            .max()
    };
    match (last(BELIEVES), last(NOW_WORDS)) {
        (Some(b), Some(n)) if b > n => Claim::Believes,
        (Some(_), None) => Claim::Believes,
        (_, Some(_)) => Claim::Now,
        (None, None) => Claim::Either,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = "Personas: Begoña, Héctor, Ana María\nLo que cada persona ha visto:\n\
- Begoña: vio que el viaje = mayo; no vio el cambio (ahora: junio).\n\
- Begoña: nunca vio nada sobre la cena.\n\
- Héctor: sabe que el viaje = junio.\n\
- Héctor: sabe que la cena = martes.\n\
- Ana María: sabe que el presupuesto = 1.500 euros.\n\
Qué esperar:\n- Héctor: tarda en responder (slow-replies) — confianza 0.80, evidencias 3, inferido.";

    #[test]
    fn slots_read_back_every_line() {
        let s = slots(STATE);
        assert_eq!(s.len(), 5);
        assert_eq!(s[0].seen.as_deref(), Some("mayo"));
        assert_eq!(s[0].current, "junio");
        assert_eq!(s[1].seen, None);
        assert_eq!(s[1].current, "martes", "filled from Héctor's line");
        assert_eq!(s[4].current, "1.500 euros");
    }

    #[test]
    fn a_belief_and_a_current_value_are_each_corrected() {
        let v = check(
            "Sí. Begoña sigue creyendo que el viaje es en agosto: no estaba cuando cambió. Escríbele que ahora el viaje es en julio.",
            "¿Tengo que avisar a Begoña de cómo quedó el viaje?",
            STATE,
            Some(&Lexicon {
                topics: [(
                    "el viaje".to_string(),
                    ["mayo", "junio", "julio", "agosto"].map(String::from).into(),
                )]
                .into(),
                ..Default::default()
            }),
        );
        assert!(v.problems.is_empty(), "{:?}", v.problems);
        assert_eq!(
            v.text,
            "Sí. Begoña sigue creyendo que el viaje es en mayo: no estaba cuando cambió. Escríbele que ahora el viaje es en junio."
        );
        assert_eq!(v.corrected.len(), 2);
    }

    #[test]
    fn a_right_answer_is_left_alone() {
        let v = check(
            "No hace falta: Héctor ya sabe que la cena es el martes.",
            "¿Tengo que avisar a Héctor de cómo quedó la cena?",
            STATE,
            None,
        );
        assert!(v.problems.is_empty() && v.corrected.is_empty(), "{v:?}");
    }

    #[test]
    fn a_topic_the_state_lacks_or_one_not_asked_is_a_problem() {
        let lexicon = Lexicon {
            topics: [("la entrega".to_string(), ["15".to_string()].into())].into(),
            ..Default::default()
        };
        let v = check(
            "Sí. Begoña cree que la entrega es el 15.",
            "¿Tengo que avisar a Begoña de cómo quedó el viaje?",
            STATE,
            Some(&lexicon),
        );
        assert_eq!(v.problems.len(), 2, "{:?}", v.problems);
    }

    #[test]
    fn a_trait_must_be_one_the_person_has_shown() {
        let lexicon = Lexicon {
            statements: [
                "suele llegar tarde".to_string(),
                "tarda en responder".to_string(),
            ]
            .into(),
            ..Default::default()
        };
        let q = "¿Qué debería tener en cuenta antes de pedirle algo a Héctor?";
        assert!(
            check("Héctor tarda en responder.", q, STATE, Some(&lexicon))
                .problems
                .is_empty()
        );
        let invented = check("Héctor suele llegar tarde.", q, STATE, Some(&lexicon));
        assert_eq!(invented.problems.len(), 1, "{invented:?}");
        assert_eq!(
            expectations(STATE)["Héctor"],
            vec![("tarda en responder".to_string(), 0.8)]
        );
    }

    #[test]
    fn a_belief_about_something_never_seen_cannot_be_corrected() {
        let v = check(
            "Begoña cree que la cena es el martes.",
            "¿Qué cree Begoña sobre la cena?",
            STATE,
            None,
        );
        assert_eq!(v.problems.len(), 1, "{v:?}");
    }
}
