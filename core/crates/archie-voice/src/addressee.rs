//! Was it said to Archie? The evidence beyond the name.
//!
//! [`listen`](crate::listen) answers when Archie is named or right after it spoke. A person
//! alone who asks "what did I have in my hand a moment ago?" names nobody, and a person with
//! friends who says "what is that?" to one of them names nobody either. What tells them apart
//! is around the words: whether anyone else is there, whether the person looks at Archie's
//! camera, whether someone else answered, whether the words call another person.
//!
//! Each cue moves the log-odds by a fixed, written amount, and each one that counted is
//! written into the evidence, so "why did Archie answer?" can be read back. A cue nobody
//! measured is `None` and moves nothing. The words alone cannot make Archie join a
//! conversation: while nobody knows who else is in the room, what the words suggest counts
//! for half, and never enough to cross the floor's threshold without the name.
//!
//! Face, gaze and voice identity come from IRIS and the speaker model, locally. This module
//! reads their conclusions, never frames or audio.

use crate::listen::{names_archie, plain};

/// Who is speaking, as the local speaker model concluded. Only the owner and people who
/// consented are ever named; anyone else is a stranger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    Owner,
    Consented,
    Stranger,
}

/// What is known around one utterance. Every field is optional evidence.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AddresseeCues {
    /// Other people present, from the camera or distinct voices. `Some(0)`: alone.
    pub others_present: Option<u8>,
    /// The speaker's head turned to Archie's camera or screen while speaking. No reading
    /// from worn glasses, which see what the speaker sees rather than the speaker.
    pub facing_archie: Option<bool>,
    /// Someone else answered within a moment of the utterance.
    pub another_replied: bool,
    /// The speaker is on a phone call.
    pub in_call: bool,
    pub voice: Option<Voice>,
    /// Names of the consented people present, so "Santi, what is that?" is to Santi.
    pub present_names: Vec<String>,
}

/// The estimate and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Addressee {
    pub probability: f32,
    pub evidence: Vec<String>,
}

/// Log-odds of each cue. 0.35, the floor policy's response threshold, is about -0.62.
const PRIOR_WITH_OTHERS: f32 = -2.2;
const PRIOR_ALONE: f32 = -1.0;
const NAMED: f32 = 5.0;
const REPLYING: f32 = 4.4;
const FACING: f32 = 1.5;
const FACING_AWAY_IN_COMPANY: f32 = -1.0;
const ANOTHER_REPLIED: f32 = -2.5;
const CALLS_SOMEONE_ELSE: f32 = -3.0;
const IN_CALL: f32 = -4.0;
const STRANGER: f32 = -1.5;
const ASKS_ABOUT_THE_SCENE: f32 = 1.0;
const ASKS: f32 = 0.4;
const COMMANDS: f32 = 0.7;

/// Words a person uses to call a friend, not a computer.
const CALLS: &[&str] = &[
    "weon",
    "wn",
    "hermano",
    "bro",
    "amor",
    "mama",
    "papa",
    "compadre",
    "compa",
    "loco",
    "amigo",
    "amiga",
    "hijo",
    "hija",
    "cabros",
    "chiquillos",
];

const QUESTION_WORDS: &[&str] = &[
    "que", "quien", "quienes", "cual", "cuales", "como", "donde", "cuando", "cuanto", "cuantos",
    "what", "who", "which", "where", "when", "how",
];

/// Words that point at what is seen or was seen: a question with one is about the scene.
const SCENE_WORDS: &[&str] = &[
    "eso", "esto", "este", "esta", "ese", "esa", "aqui", "ahi", "alla", "mano", "manos", "ves",
    "viste", "veo", "tenia", "tengo", "tiene", "that", "this", "these", "those", "here", "there",
    "hand", "see", "saw", "holding",
];

/// Imperatives said to a machine that can act.
const COMMAND_WORDS: &[&str] = &[
    "abre",
    "abrir",
    "cierra",
    "busca",
    "buscame",
    "pon",
    "pone",
    "ponme",
    "muestra",
    "muestrame",
    "dime",
    "dame",
    "anota",
    "recuerda",
    "recuerdame",
    "llama",
    "manda",
    "envia",
    "sube",
    "baja",
    "para",
    "deten",
    "explica",
    "explicame",
    "open",
    "close",
    "search",
    "find",
    "show",
    "tell",
    "play",
    "stop",
    "remind",
    "explain",
];

fn logistic(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn asks(text: &str, words: &[&str]) -> bool {
    text.contains(['?', '¿'])
        || words
            .first()
            .is_some_and(|first| QUESTION_WORDS.contains(first))
}

pub fn estimate(text: &str, replying: bool, cues: &AddresseeCues) -> Addressee {
    let flat = plain(text);
    let words: Vec<&str> = flat.split(' ').filter(|word| !word.is_empty()).collect();
    let mut evidence = Vec::new();
    let mut logit = match cues.others_present {
        Some(0) => {
            evidence.push("nobody else is there".to_string());
            PRIOR_ALONE
        }
        Some(others) => {
            evidence.push(format!("{others} other people are there"));
            PRIOR_WITH_OTHERS
        }
        None => PRIOR_WITH_OTHERS,
    };
    let company_known = cues.others_present.is_some();
    let in_company = cues.others_present.is_some_and(|others| others > 0);

    if names_archie(text) {
        logit += NAMED;
        evidence.push("the utterance names Archie".to_string());
    } else if replying {
        logit += REPLYING;
        evidence.push("it came right after Archie spoke".to_string());
    } else {
        evidence.push("it neither names Archie nor answers it".to_string());
    }
    match cues.facing_archie {
        Some(true) => {
            logit += FACING;
            evidence.push("the speaker looked at Archie".to_string());
        }
        Some(false) if in_company => {
            logit += FACING_AWAY_IN_COMPANY;
            evidence.push("the speaker looked away from Archie, among others".to_string());
        }
        _ => {}
    }
    if cues.another_replied {
        logit += ANOTHER_REPLIED;
        evidence.push("someone else answered it".to_string());
    }
    let called = [words.first(), words.last()]
        .into_iter()
        .flatten()
        .find(|word| {
            CALLS.contains(word) || cues.present_names.iter().any(|name| plain(name) == **word)
        });
    if let Some(called) = called {
        logit += CALLS_SOMEONE_ELSE;
        evidence.push(format!("it calls someone else: «{called}»"));
    }
    if cues.in_call {
        logit += IN_CALL;
        evidence.push("the speaker is on a call".to_string());
    }
    if cues.voice == Some(Voice::Stranger) {
        logit += STRANGER;
        evidence.push("the voice is not one Archie knows".to_string());
    }

    // What the words suggest counts fully only when it is known who else is there.
    let weight = if company_known { 1.0 } else { 0.5 };
    let question = asks(text, &words);
    if question && words.iter().any(|word| SCENE_WORDS.contains(word)) {
        logit += weight * ASKS_ABOUT_THE_SCENE;
        evidence.push("it asks about what is or was in view".to_string());
    } else if question {
        logit += weight * ASKS;
        evidence.push("it is a question".to_string());
    }
    let command = words
        .iter()
        .find(|word| !matches!(**word, "por" | "favor" | "porfa" | "please"))
        .is_some_and(|word| COMMAND_WORDS.contains(word));
    if command {
        logit += weight * COMMANDS;
        evidence.push("it is an instruction".to_string());
    }
    Addressee {
        probability: logistic(logit),
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLD: f32 = 0.35;

    fn alone() -> AddresseeCues {
        AddresseeCues {
            others_present: Some(0),
            ..AddresseeCues::default()
        }
    }

    fn with_friends(facing: Option<bool>) -> AddresseeCues {
        AddresseeCues {
            others_present: Some(2),
            facing_archie: facing,
            present_names: vec!["Santi".into()],
            ..AddresseeCues::default()
        }
    }

    fn answered(text: &str, replying: bool, cues: &AddresseeCues) -> bool {
        estimate(text, replying, cues).probability >= THRESHOLD
    }

    #[test]
    fn with_nothing_known_only_the_name_or_a_reply_is_answered_as_before() {
        let unknown = AddresseeCues::default();
        assert!(answered("Archie, ¿qué hora es?", false, &unknown));
        assert!(answered("sí, ese", true, &unknown));
        assert!(!answered("¿qué es eso?", false, &unknown));
        assert!(!answered("abre el navegador", false, &unknown));
        assert!(!answered("¿qué es eso? abre", false, &unknown));
    }

    #[test]
    fn alone_a_question_about_the_scene_or_an_instruction_is_answered_but_a_remark_is_not() {
        assert!(answered(
            "¿Qué tenía en la mano hace un momento?",
            false,
            &alone()
        ));
        assert!(answered("¿qué es eso?", false, &alone()));
        assert!(answered("busca el precio del motor", false, &alone()));
        assert!(!answered("uf, qué calor", false, &alone()));
    }

    #[test]
    fn among_friends_looking_at_archie_decides_and_a_friend_answering_undoes_it() {
        assert!(answered("¿qué es eso?", false, &with_friends(Some(true))));
        assert!(!answered("¿qué es eso?", false, &with_friends(Some(false))));
        assert!(!answered("¿qué es eso?", false, &with_friends(None)));
        let replied = AddresseeCues {
            another_replied: true,
            ..with_friends(Some(true))
        };
        assert!(!answered("¿qué es eso?", false, &replied));
    }

    #[test]
    fn calling_a_friend_by_name_or_on_a_call_is_not_to_archie() {
        assert!(!answered(
            "Santi, ¿qué es eso?",
            false,
            &with_friends(Some(true))
        ));
        assert!(!answered("weón, pásame eso", false, &alone()));
        let calling = AddresseeCues {
            in_call: true,
            ..alone()
        };
        assert!(!answered("¿qué es eso?", false, &calling));
        assert!(answered(
            "Archie, ¿qué es eso?",
            false,
            &with_friends(Some(false))
        ));
    }

    #[test]
    fn a_stranger_needs_the_name_and_every_cue_that_counted_is_written_down() {
        let stranger = AddresseeCues {
            voice: Some(Voice::Stranger),
            ..alone()
        };
        assert!(!answered("¿qué es eso?", false, &stranger));
        let estimate = estimate("¿qué es eso?", false, &with_friends(Some(true)));
        assert!(estimate
            .evidence
            .iter()
            .any(|line| line.contains("looked at Archie")));
        assert!(estimate
            .evidence
            .iter()
            .any(|line| line.contains("what is or was in view")));
    }
}
