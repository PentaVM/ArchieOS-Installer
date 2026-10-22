//! State 1 from the bookkeeping: the questions the social state answers by itself.
//!
//! Who saw what is counted, not inferred ([`crate::state`]). So "does X know that …" and
//! "what does X believe about …" need no model: the answer is read from the state, with the
//! reason it holds; someone the conversations never show saw nothing in them. "Will X …" is
//! read from one of X's expectations when the question names it clearly. Anything not
//! recognised with certainty (no person or two, no topic or two, a value other than the current
//! one, an expectation named only in passing) is left to the model.
//!
//! The state holds only what the conversations show: someone told in person is not in it.
//! Answers say where they come from, and their confidence stays at
//! [`MAX_INFERRED_CONFIDENCE`], like any inference about a person.

use crate::model::{AnswerSource, Decision, QuickAnswer};
use crate::state::{PersonState, SeenHow, SocialState};
use crate::{Lang, MAX_INFERRED_CONFIDENCE};
use std::collections::BTreeSet;

/// Evidence an expectation needs before it answers yes or no.
pub const MIN_EVIDENCE: usize = 2;

const KNOW: &[&str] = &[
    "sabe", "sabía", "enteró", "enterado", "enterada", "know", "knows", "aware", "heard",
];
const BELIEVE: &[&str] = &[
    "cree", "creen", "piensa", "believe", "believes", "think", "thinks",
];
const WHAT: &[&str] = &["qué", "what"];
const ARTICLES: &[&str] = &["el ", "la ", "los ", "las ", "the "];
/// Words too common to tie a question to an expectation.
const COMMON: &[&str] = &[
    "that", "they", "their", "them", "will", "with", "from", "this", "what", "when", "para",
    "pero", "como", "algo", "cuando", "esto", "esta", "este", "suele", "usually",
];

/// What the state says, for one recognised question.
#[derive(Debug, Clone, PartialEq)]
pub enum Exact {
    /// Whether `person` saw the current value of `topic`.
    Knows {
        person: String,
        topic: String,
        current: String,
        /// The last value they saw, and how.
        seen: Option<(String, SeenHow)>,
    },
    /// The last value of `topic` that `person` saw. `asked` is the value a yes/no question
    /// names ("does X think the trip is in July?").
    Believes {
        person: String,
        topic: String,
        value: Option<String>,
        asked: Option<String>,
    },
    /// One of the expectations the state holds about `person`.
    Expects {
        person: String,
        key: String,
        statement: String,
        confidence: f32,
        evidence: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Knows,
    Believes,
    Other,
}

/// The state's answer to `question`, or `None` when the question is not one it answers.
pub fn resolve(state: &SocialState, question: &str) -> Option<Exact> {
    let q = question.to_lowercase();
    let kind = kind(&q);
    let person = named_person(state, &q).and_then(|id| state.people.get(id));
    if kind == Kind::Other {
        return expectation(person?, &q);
    }
    let (topic, asked) = named_topic(state, &q)?;
    // Someone the conversations never show saw nothing in them.
    let (label, seen) = match person {
        Some(p) => (p.label.clone(), p.seen.get(&topic)),
        None => (unknown_person(state, question)?, None),
    };
    if kind == Kind::Knows {
        let current = state.facts.get(&topic)?.current()?.value.clone();
        if asked.as_ref().is_some_and(|a| *a != current) {
            return None;
        }
        Some(Exact::Knows {
            person: label,
            topic,
            current,
            seen: seen.map(|s| (s.value.clone(), s.how)),
        })
    } else {
        Some(Exact::Believes {
            person: label,
            topic,
            value: seen.map(|s| s.value.clone()),
            asked,
        })
    }
}

impl Exact {
    /// `Some(true)` yes, `Some(false)` no; `None` when the state cannot tell, or when the
    /// question asks what someone believes rather than yes or no.
    pub fn answer(&self) -> Option<bool> {
        match self {
            Exact::Knows { current, seen, .. } => {
                Some(seen.as_ref().is_some_and(|(v, _)| v == current))
            }
            Exact::Believes { value, asked, .. } => match (value, asked) {
                (Some(v), Some(a)) => Some(v == a),
                _ => None,
            },
            Exact::Expects {
                confidence,
                evidence,
                ..
            } => {
                if *evidence < MIN_EVIDENCE || (confidence - 0.5).abs() < f32::EPSILON {
                    None
                } else {
                    Some(*confidence > 0.5)
                }
            }
        }
    }

    /// Probability of the answer given.
    pub fn confidence(&self) -> f32 {
        match (self, self.answer()) {
            (Exact::Expects { confidence, .. }, Some(true)) => *confidence,
            (Exact::Expects { confidence, .. }, Some(false)) => 1.0 - confidence,
            _ => MAX_INFERRED_CONFIDENCE,
        }
    }

    /// The answer as it is spoken, with the reason it holds.
    pub fn text(&self, lang: Lang) -> String {
        let es = lang == Lang::Es;
        match self {
            Exact::Knows {
                person,
                topic,
                current,
                seen,
            } => match seen {
                Some((v, how)) if v == current => match (*how, es) {
                    (SeenHow::Said, true) => format!("Sí. Lo dijo {person}."),
                    (SeenHow::Said, false) => format!("Yes. {person} said it."),
                    (SeenHow::Present, true) => {
                        format!("Sí. {person} estaba en la conversación cuando se dijo.")
                    }
                    (SeenHow::Present, false) => {
                        format!("Yes. {person} was in the conversation when it was said.")
                    }
                    (SeenHow::Told, true) => format!("Sí. A {person} se lo contaron."),
                    (SeenHow::Told, false) => format!("Yes. Someone told {person}."),
                },
                Some((v, _)) if es => format!(
                    "No. {person} vio que {topic} era {v}, pero no vio el cambio a {current}."
                ),
                Some((v, _)) => format!(
                    "No. {person} saw that {topic} was {v}, but not the change to {current}."
                ),
                None if es => format!(
                    "No, por lo que muestran las conversaciones: {person} no vio nada sobre {topic}."
                ),
                None => format!(
                    "Not from what the conversations show: {person} saw nothing about {topic}."
                ),
            },
            Exact::Believes {
                person,
                topic,
                value,
                asked,
            } => match (value, asked) {
                (None, _) if es => {
                    format!("No puedo saberlo: {person} nunca vio nada sobre {topic}.")
                }
                (None, _) => format!("I can't tell: {person} never saw anything about {topic}."),
                (Some(v), Some(a)) if v != a && es => {
                    format!("No. Por lo último que vio, {person} cree que {topic} es {v}.")
                }
                (Some(v), Some(a)) if v != a => format!(
                    "No. From the last thing they saw, {person} believes {topic} is {v}."
                ),
                (Some(v), Some(_)) if es => {
                    format!("Sí. Por lo último que vio, {person} cree que {topic} es {v}.")
                }
                (Some(v), Some(_)) => format!(
                    "Yes. From the last thing they saw, {person} believes {topic} is {v}."
                ),
                (Some(v), None) if es => {
                    format!("Por lo último que vio, {person} cree que {topic} es {v}.")
                }
                (Some(v), None) => {
                    format!("From the last thing they saw, {person} believes {topic} is {v}.")
                }
            },
            Exact::Expects {
                person, statement, ..
            } => match (self.answer(), es) {
                (Some(true), true) => format!("Probablemente sí: {person} {statement}."),
                (Some(true), false) => format!("Probably yes: {person} {statement}."),
                (Some(false), true) => {
                    format!("Probablemente no, por lo que se ha visto de {person}.")
                }
                (Some(false), false) => format!("Probably not, from what {person} has shown."),
                (None, true) => format!(
                    "Todavía no hay suficiente en las conversaciones para saberlo de {person}."
                ),
                (None, false) => {
                    format!("There isn't enough in the conversations yet to tell with {person}.")
                }
            },
        }
    }

    /// State 1's spoken answer.
    pub fn quick(&self, lang: Lang) -> QuickAnswer {
        QuickAnswer {
            text: self.text(lang),
            answer: self.answer(),
            confidence: self.confidence(),
            source: AnswerSource::State,
        }
    }

    /// The answer among explicit options, as the decision head would give it. `None` when an
    /// option the answer needs is not offered.
    pub fn choose(&self, lang: Lang, options: &[String]) -> Option<Decision> {
        let index = |wanted: &str| {
            options
                .iter()
                .position(|o| o.trim().to_lowercase() == wanted.trim().to_lowercase())
        };
        let choice = match (self, self.answer()) {
            (
                Exact::Believes {
                    asked: None, value, ..
                },
                _,
            ) => match value {
                Some(v) => Some(index(v)?),
                None => None,
            },
            (_, Some(true)) => Some(index(lang.yes())?),
            (_, Some(false)) => Some(index(lang.no())?),
            (_, None) => None,
        };
        let confidence = self.confidence();
        let rest = (1.0 - confidence) / options.len().max(1) as f32;
        let probabilities = (0..options.len())
            .map(|i| if Some(i) == choice { confidence } else { rest })
            .collect();
        Some(Decision {
            probabilities,
            abstain: if choice.is_none() { confidence } else { rest },
            choice,
            confidence,
        })
    }
}

fn kind(q: &str) -> Kind {
    let words: BTreeSet<&str> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |list: &[&str]| list.iter().any(|w| words.contains(w));
    if has(BELIEVE) {
        Kind::Believes
    } else if has(KNOW) || q.contains("al tanto") {
        if has(WHAT) {
            Kind::Believes
        } else {
            Kind::Knows
        }
    } else {
        Kind::Other
    }
}

/// Where `needle` occurs in `hay` as whole words.
fn find_phrase(hay: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    hay.match_indices(needle).find_map(|(start, _)| {
        let end = start + needle.len();
        let before = hay[..start].chars().next_back();
        let after = hay[end..].chars().next();
        let clear = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        (clear(before) && clear(after)).then_some((start, end))
    })
}

/// The spans that no longer span contains: "Ana María" wins over "Ana".
fn outermost<T: Clone>(found: &[(T, usize, usize)]) -> Vec<T> {
    found
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            !found
                .iter()
                .enumerate()
                .any(|(j, b)| j != *i && b.1 <= a.1 && a.2 <= b.2 && (b.2 - b.1) > (a.2 - a.1))
        })
        .map(|(_, a)| a.0.clone())
        .collect()
}

/// The one person the question names: by full name, or by first name when only one person
/// has it. The owner is never the subject.
fn named_person<'a>(state: &'a SocialState, q: &str) -> Option<&'a str> {
    let others = || state.people.values().filter(|p| p.id != state.owner);
    let full: Vec<(&str, usize, usize)> = others()
        .filter_map(|p| {
            let (s, e) = find_phrase(q, &p.label.to_lowercase())?;
            Some((p.id.as_str(), s, e))
        })
        .collect();
    let mut found: Vec<&str> = outermost(&full);
    found.dedup();
    if found.is_empty() {
        found = others()
            .filter(|p| {
                p.label
                    .split_whitespace()
                    .next()
                    .filter(|first| first.chars().count() >= 3)
                    .is_some_and(|first| find_phrase(q, &first.to_lowercase()).is_some())
            })
            .map(|p| p.id.as_str())
            .collect();
    }
    match found.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// A name the question gives that is no one in the state: one run of capitalised words that
/// is not the question's first word and not part of a person, topic or value the state has.
fn unknown_person(state: &SocialState, question: &str) -> Option<String> {
    let lower_words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let known: BTreeSet<String> = state
        .people
        .values()
        .flat_map(|p| lower_words(&p.label))
        .chain(state.facts.values().flat_map(|f| {
            std::iter::once(&f.topic)
                .chain(f.versions.iter().map(|v| &v.value))
                .flat_map(|t| lower_words(t))
                .collect::<Vec<_>>()
        }))
        .collect();
    let mut runs: Vec<Vec<&str>> = Vec::new();
    let mut in_run = false;
    let words = question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty());
    for (i, w) in words.enumerate() {
        let name = i > 0
            && w != "I"
            && w.chars().next().is_some_and(char::is_uppercase)
            && !known.contains(&w.to_lowercase());
        match (name, in_run, runs.last_mut()) {
            (true, true, Some(run)) => run.push(w),
            (true, _, _) => runs.push(vec![w]),
            _ => {}
        }
        in_run = name;
    }
    match runs.as_slice() {
        [one] => Some(one.join(" ")),
        _ => None,
    }
}

/// The one topic the question names, with or without its article, and the value of it the
/// question names, if any.
fn named_topic(state: &SocialState, q: &str) -> Option<(String, Option<String>)> {
    let found: Vec<(&str, usize, usize)> = state
        .facts
        .keys()
        .filter_map(|topic| {
            let lower = topic.to_lowercase();
            let bare = ARTICLES
                .iter()
                .find_map(|a| lower.strip_prefix(a))
                .filter(|b| b.chars().count() >= 4);
            let (s, e) = find_phrase(q, &lower).or_else(|| find_phrase(q, bare?))?;
            Some((topic.as_str(), s, e))
        })
        .collect();
    let mut topics = outermost(&found);
    topics.dedup();
    let [topic] = topics.as_slice() else {
        return None;
    };
    let asked = state.facts[*topic]
        .versions
        .iter()
        .map(|v| v.value.as_str())
        .filter(|v| find_phrase(q, &v.to_lowercase()).is_some())
        .max_by_key(|v| v.len())
        .map(str::to_string);
    Some((topic.to_string(), asked))
}

fn stems(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4 && !COMMON.contains(w))
        .map(|w| w.chars().take(4).collect())
        .collect()
}

/// The expectation the question is about. Its key must be among the question's words, or
/// two words must be shared with the key and statement together: one shared word ("¿llegará
/// Pedro a la cena?" against "suele llegar tarde") is not enough.
fn expectation(person: &PersonState, q: &str) -> Option<Exact> {
    let name = stems(&person.label.to_lowercase());
    let asked: BTreeSet<String> = stems(q).difference(&name).cloned().collect();
    let mut scored: Vec<(usize, usize)> = person
        .expectations
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let key = stems(&e.key.replace(['-', '_'], " "));
            let all: BTreeSet<String> = key
                .union(&stems(&e.statement.to_lowercase()))
                .cloned()
                .collect();
            let shared = asked.intersection(&all).count();
            let by_key = asked.intersection(&key).count();
            (shared >= 2 || by_key >= 1).then_some((shared, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    match scored.as_slice() {
        [(best, i), rest @ ..] if rest.first().is_none_or(|(next, _)| next < best) => {
            let e = &person.expectations[*i];
            Some(Exact::Expects {
                person: person.label.clone(),
                key: e.key.clone(),
                statement: e.statement.clone(),
                confidence: e.confidence(),
                evidence: e.evidence.len(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Behaviour, Expectation, Fact, FactVersion, Standing};
    use std::collections::BTreeMap;

    fn person(label: &str, expectations: Vec<Expectation>) -> PersonState {
        PersonState {
            id: label.to_string(),
            label: label.to_string(),
            chats: BTreeSet::new(),
            first_turn: 0,
            last_turn: 0,
            seen: BTreeMap::new(),
            expectations,
            affect: None,
            behaviour: Behaviour::default(),
        }
    }

    fn expect(key: &str, statement: &str, support: f32, contradict: f32) -> Expectation {
        let shown = (support + contradict) as usize;
        Expectation {
            key: key.into(),
            statement: statement.into(),
            support,
            contradict,
            standing: Standing::Inferred,
            first_at_ms: 0,
            last_at_ms: 0,
            evidence: vec!["t".into(); shown],
        }
    }

    #[test]
    fn phrases_match_whole_words_only() {
        assert_eq!(find_phrase("¿sabe ana que…?", "ana"), Some((7, 10)));
        assert_eq!(find_phrase("¿sabe mariana que…?", "ana"), None);
        assert!(find_phrase("¿y la cena del lunes?", "la cena").is_some());
    }

    #[test]
    fn the_question_kind_follows_its_verb() {
        assert_eq!(kind("¿sabe pedro que el viaje es en julio?"), Kind::Knows);
        assert_eq!(kind("does pedro know the trip moved?"), Kind::Knows);
        assert_eq!(kind("¿qué sabe pedro del viaje?"), Kind::Believes);
        assert_eq!(kind("¿qué cree pedro sobre el viaje?"), Kind::Believes);
        assert_eq!(kind("si pedro promete algo, ¿lo cumplirá?"), Kind::Other);
    }

    #[test]
    fn an_expectation_is_named_by_its_key_or_by_two_shared_words() {
        let pedro = person(
            "Pedro",
            vec![
                expect("late", "suele llegar tarde", 3.0, 0.0),
                expect("slow-replies", "tarda en responder", 1.0, 2.0),
                expect("direct", "dice las cosas claras", 1.0, 0.0),
            ],
        );
        let answer = |q: &str| expectation(&pedro, &q.to_lowercase());
        let late = answer("Si quedas con Pedro a las ocho, ¿es probable que llegue tarde?");
        assert!(matches!(&late, Some(Exact::Expects { key, .. }) if key == "late"));
        assert_eq!(late.unwrap().answer(), Some(true));
        let slow = answer("Si le escribes a Pedro algo urgente, ¿tardará en responder?");
        assert!(matches!(&slow, Some(Exact::Expects { key, .. }) if key == "slow-replies"));
        assert_eq!(slow.unwrap().answer(), Some(false));
        // One piece of evidence is not enough to say yes.
        let direct = answer("If Pedro dislikes a plan, will he tell you directly?");
        assert_eq!(direct.unwrap().answer(), None);
        // "Llegar" alone does not make "will he come to dinner" a question about lateness.
        assert_eq!(answer("¿Pedro llegará a la cena?"), None);
    }

    #[test]
    fn a_name_the_state_does_not_have_is_someone_who_saw_nothing() {
        let mut state = SocialState::new("yo", "Yo").unwrap();
        state.facts.insert(
            "the dinner".into(),
            Fact {
                topic: "the dinner".into(),
                versions: vec![FactVersion {
                    value: "on Tuesday".into(),
                    turn: 1,
                    at_ms: 0,
                    evidence: String::new(),
                }],
            },
        );
        let found = resolve(&state, "Does Nico know that the dinner is on Tuesday?").unwrap();
        assert!(matches!(&found, Exact::Knows { person, .. } if person == "Nico"));
        assert_eq!(found.answer(), Some(false));
        let belief = resolve(&state, "What does Nico believe about the dinner?").unwrap();
        assert_eq!(belief.answer(), None);
        // Two names: which one is asked about is not for this module to guess.
        assert_eq!(
            resolve(&state, "Does Nico know that Ana moved the dinner?"),
            None
        );
    }

    #[test]
    fn options_are_matched_without_case_or_spaces() {
        let belief = Exact::Believes {
            person: "Ana".into(),
            topic: "la cena".into(),
            value: Some("el lunes".into()),
            asked: None,
        };
        let options = vec!["El martes".to_string(), "el Lunes ".to_string()];
        let d = belief.choose(Lang::Es, &options).unwrap();
        assert_eq!(d.choice, Some(1));
        assert!(belief.choose(Lang::Es, &options[..1]).is_none());
        let never = Exact::Believes {
            person: "Ana".into(),
            topic: "la cena".into(),
            value: None,
            asked: None,
        };
        assert_eq!(never.choose(Lang::Es, &options).unwrap().choice, None);
    }
}
