//! What was heard, turned into what the floor decides on.
//!
//! The speech plane's [`FloorPolicy`](archie_speech::FloorPolicy) decides whether Archie
//! speaks from a [`ConversationalFloorState`]. The learned estimator that will fill that
//! state from the audio is not trained yet, so this fills it from the evidence the ears do
//! have — the words, whether the recogniser called the utterance finished, how long the
//! silence after it ran — and writes that evidence into the state, so that a person asking
//! why Archie answered (or did not) can read it.
//!
//! Two judgements carry most of the weight, and both are deliberately conservative:
//!
//! - **Was it said to Archie?** A microphone in a room hears conversations that are not
//!   addressed to anybody's computer. An utterance counts as addressed when it names Archie,
//!   or when it follows something Archie said closely enough to be the reply to it. Everything
//!   else leaves the response utility low, and the policy stays silent.
//! - **Was it only an acknowledgement?** "ya", "ok", "mhm" answer nothing and ask nothing.
//!   While Archie speaks they must not stop it; after it speaks they must not start it again.

use crate::addressee::AddresseeCues;
use archie_speech::{
    ConversationalFloorState, InterruptionDirection, OverlapState, Probability, SilenceScope,
    Speaker,
};
use std::time::Duration;

/// How long after Archie finished speaking a reply needs no name to be addressed to it.
pub const FOLLOW_UP_WINDOW: Duration = Duration::from_secs(10);

/// The spellings a recogniser gives the name. Nemotron writes what it hears; "Archi" and
/// "Archy" are the same word said aloud.
const NAMES: &[&str] = &["archie", "archi", "archy"];

/// Real words one letter from the name, which must not wake Archie.
const NOT_NAMES: &[&str] = &["arch", "archive", "archer", "arco", "marchi"];

/// Letters to turn `a` into `b`, one insertion, deletion or change at a time.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// Whether one written word is the name as a recogniser mishears it: "Archier", "Arshi",
/// "Harchi". Only words that start like the name count, so "marchi" or "ancho" never do.
fn sounds_like_archie(word: &str) -> bool {
    if NAMES.contains(&word) {
        return true;
    }
    if NOT_NAMES.contains(&word) || !(4..=8).contains(&word.chars().count()) {
        return false;
    }
    let starts_like = ["ar", "har", "ahr", "ars"]
        .iter()
        .any(|start| word.starts_with(start));
    starts_like && (edit_distance(word, "archi") <= 1 || edit_distance(word, "archie") <= 1)
}

/// Utterances that acknowledge and ask nothing, compared without accents or punctuation.
const ACKNOWLEDGEMENTS: &[&str] = &[
    "ya",
    "ok",
    "okay",
    "oka",
    "mhm",
    "mm",
    "aja",
    "aha",
    "si",
    "vale",
    "listo",
    "dale",
    "perfecto",
    "bueno",
    "claro",
    "ya po",
    "si po",
    "ah ya",
    "ok gracias",
];

/// Lower case, without accents or punctuation, words separated by single spaces.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            other => other,
        };
        if c.is_alphanumeric() || c == 'ñ' {
            out.push(c);
        } else if !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

/// Whether the utterance names Archie — also when the recogniser misspelled the name or
/// split it in two ("ar chi").
pub fn names_archie(text: &str) -> bool {
    let plain = plain(text);
    let words: Vec<&str> = plain.split(' ').collect();
    words.iter().any(|word| sounds_like_archie(word))
        || words.windows(2).any(|pair| {
            pair[0].len() <= 3 && sounds_like_archie(&format!("{}{}", pair[0], pair[1]))
        })
}

/// The pause after a question, an order or the name before the floor is Archie's.
pub const ASKED_PAUSE_MS: u32 = 700;
/// The pause after a statement. Archie is a copilot, not a chatbot: the person may be
/// thinking aloud or giving it the context to reason over, and a thought pauses inside
/// itself for longer than a breath. Until this much silence the floor stays theirs.
pub const STATEMENT_PAUSE_MS: u32 = 1800;

/// Words that open a question, an order or a greeting, in either language.
const ASKING: &[&str] = &[
    "que",
    "como",
    "cuando",
    "donde",
    "quien",
    "cual",
    "cuanto",
    "cuanta",
    "cuantos",
    "cuantas",
    "puedes",
    "podrias",
    "abre",
    "abri",
    "busca",
    "dime",
    "explica",
    "muestra",
    "muestrame",
    "haz",
    "pon",
    "cierra",
    "escribe",
    "ayudame",
    "necesito",
    "quiero",
    "hola",
    "buenas",
    "buenos",
    "what",
    "how",
    "when",
    "where",
    "who",
    "which",
    "why",
    "can",
    "could",
    "would",
    "is",
    "are",
    "do",
    "does",
    "mira",
    "lee",
    "revisa",
    "calcula",
    "traduce",
    "resume",
    "compara",
    "cuenta",
    "open",
    "show",
    "tell",
    "find",
    "explain",
    "please",
    "help",
    "look",
    "read",
    "check",
    "hello",
    "hi",
    "hey",
];

/// Whether something was asked of Archie: a question mark, an opening question word, an
/// order or a greeting, or its name.
pub fn asks_something(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.ends_with('?') || trimmed.contains('¿') || names_archie(text) {
        return true;
    }
    // Within its first words: "no, dime la hora", "oye, ¿y cómo…", "bueno, abre el correo"
    // open with a word that only leads into what is asked.
    let plain = plain(text);
    let words: Vec<&str> = plain.split(' ').take(3).collect();
    words.iter().any(|word| ASKING.contains(word))
        || words.windows(2).any(|pair| pair == ["por", "que"])
}

/// How much there is to reason over. A question carries what it needs; a statement does
/// once it says enough; a fragment ("eh, lo que", "bueno entonces") does not, and Archie keeps
/// listening for the rest instead of answering it.
pub fn context_sufficiency(text: &str) -> f32 {
    sufficiency(text, asks_something(text))
}

fn sufficiency(text: &str, asks: bool) -> f32 {
    let words = plain(text)
        .split(' ')
        .filter(|word| !word.is_empty())
        .count();
    match (words, asks) {
        (0, _) => 0.0,
        (1, true) => 0.5,
        (_, true) => 0.85,
        (8.., false) => 0.6,
        _ => 0.3,
    }
}

/// Words that ask Archie to stop talking.
const STOP_WORDS: &[&str] = &[
    "para", "pare", "detente", "espera", "basta", "silencio", "calla", "callate", "stop", "wait",
    "enough", "quiet",
];

/// Whether the person asks Archie, by name, to stop: "Archie, para", "Archie, espera".
/// Through a speaker this is the only thing heard while Archie speaks that counts, because
/// its own voice comes back through the microphone, and it does not say its own name
/// together with an order to stop.
pub fn is_stop_request(text: &str) -> bool {
    names_archie(text)
        && plain(text)
            .split(' ')
            .any(|word| STOP_WORDS.contains(&word))
}

/// What the model answers when it chooses to say nothing: what it heard was not said to it
/// (a video, two people talking), was half a thought, or wants no answer. The persona and
/// the local voice's guide teach it; the voice drops it instead of reading it aloud.
pub const SILENCE_MARKER: &str = "[silencio]";

/// Whether a segment of the answer is the model choosing silence.
pub fn is_silence_marker(text: &str) -> bool {
    matches!(plain(text).as_str(), "silencio" | "silence")
}

/// Whether the utterance is only an acknowledgement.
pub fn is_acknowledgement(text: &str) -> bool {
    let plain = plain(text);
    !plain.is_empty() && ACKNOWLEDGEMENTS.contains(&plain.as_str())
}

/// One finished utterance, as the ears report it.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    pub text: String,
    /// The recogniser marked it final: it heard the end of the turn, not a breath.
    pub final_by_recogniser: bool,
    /// How long it has been quiet since the last word.
    pub silence_ms: u32,
    /// The streaming ears heard Archie's name, even if a second hearing left it out of
    /// `text`: being called by name is being asked.
    pub named: bool,
}

/// The floor after an utterance, with nobody speaking.
///
/// `since_archie_spoke` is how long ago Archie last finished saying something, if it has.
pub fn after_utterance(
    heard: &Utterance,
    since_archie_spoke: Option<Duration>,
) -> ConversationalFloorState {
    after_utterance_within(heard, since_archie_spoke, FOLLOW_UP_WINDOW)
}

/// As [`after_utterance`], with the follow-up window given: how long after Archie spoke a
/// reply needs no name.
pub fn after_utterance_within(
    heard: &Utterance,
    since_archie_spoke: Option<Duration>,
    follow_up: Duration,
) -> ConversationalFloorState {
    after_utterance_with(
        heard,
        since_archie_spoke,
        follow_up,
        &AddresseeCues::default(),
    )
}

/// As [`after_utterance_within`], with what is known around the words: who else is there,
/// where the speaker looked, whether someone else answered. See [`crate::addressee`].
pub fn after_utterance_with(
    heard: &Utterance,
    since_archie_spoke: Option<Duration>,
    follow_up: Duration,
    cues: &AddresseeCues,
) -> ConversationalFloorState {
    let replying = since_archie_spoke.is_some_and(|since| since <= follow_up);
    let addressee = crate::addressee::estimate(&heard.text, replying, cues);
    let mut evidence = addressee.evidence;

    let acknowledgement = is_acknowledgement(&heard.text);
    if acknowledgement {
        evidence.push("the words are only an acknowledgement".to_string());
    }

    let trimmed = heard.text.trim_end();
    let sounds_finished = trimmed.ends_with(['.', '?', '!', '¿', '¡']) || heard.final_by_recogniser;
    let trailing = plain(trimmed);
    // "abre el formulario y" promises more, whatever the punctuation says.
    let promises_more = ["y", "pero", "o", "que", "entonces", "porque", "and", "but"]
        .iter()
        .any(|word| trailing == *word || trailing.ends_with(&format!(" {word}")));
    if promises_more {
        evidence.push("it ends on a word that promises more".to_string());
    }
    let finished = sounds_finished && !promises_more;
    evidence.push(format!(
        "{} ms of silence{}",
        heard.silence_ms,
        if heard.final_by_recogniser {
            ", and the recogniser called the turn over"
        } else {
            ""
        }
    ));

    // The floor is given when the thought is complete, not at the first breath: a question
    // or an order after a short pause, a statement after a long one. A reply to what Archie
    // just said counts as asked: it completes Archie's own question.
    let asks = asks_something(&heard.text) || replying || heard.named;
    let pause = if asks {
        ASKED_PAUSE_MS
    } else {
        STATEMENT_PAUSE_MS
    };
    evidence.push(format!(
        "{}: the turn ends after {pause} ms of silence",
        if asks {
            "it asks something"
        } else {
            "a statement, maybe the start of a thought"
        }
    ));
    let scope = if finished && heard.silence_ms >= pause {
        SilenceScope::EndOfTurn
    } else if heard.silence_ms >= 300 {
        SilenceScope::BetweenClauses
    } else {
        SilenceScope::WithinClause
    };
    let sufficiency = if heard.text.trim().is_empty() {
        0.0
    } else {
        sufficiency(&heard.text, asks)
    };
    if sufficiency < 0.4 {
        evidence.push("not enough said to reason over yet".to_string());
    }

    ConversationalFloorState {
        current_speaker: Speaker::Nobody,
        speaker_hold: Probability::new(if promises_more { 0.8 } else { 0.1 }),
        thought_completion: Probability::new(if scope == SilenceScope::EndOfTurn {
            0.9
        } else {
            0.3
        }),
        silence_scope: scope,
        silence_ms: heard.silence_ms,
        interruption_utility: Probability::ZERO,
        response_utility: Probability::new(addressee.probability),
        urgency: Probability::new(0.2),
        context_sufficiency: Probability::new(sufficiency),
        acknowledgement_only: Probability::new(if acknowledgement { 0.95 } else { 0.0 }),
        overlap: OverlapState::None,
        recent_interruption: InterruptionDirection::None,
        response_still_valid: Probability::ONE,
        evidence,
    }
}

/// The floor while Archie is speaking and the person says something over it.
///
/// `words` is what the recogniser has of it so far; an empty string is a voice with no
/// words yet, which is a bid for the floor until it turns out to be "mhm".
pub fn over_archie(words: &str) -> ConversationalFloorState {
    let acknowledgement = is_acknowledgement(words);
    ConversationalFloorState {
        current_speaker: Speaker::Archie,
        overlap: OverlapState::UserOverArchie,
        acknowledgement_only: Probability::new(if acknowledgement { 0.95 } else { 0.0 }),
        recent_interruption: InterruptionDirection::UserInterruptedArchie,
        response_still_valid: Probability::new(0.3),
        evidence: vec![if acknowledgement {
            format!("the person said «{words}» over Archie, an acknowledgement")
        } else {
            "the person started speaking over Archie".to_string()
        }],
        ..ConversationalFloorState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_speech::{FloorAction, FloorPolicy, SpeechBudget};

    fn heard(text: &str) -> Utterance {
        Utterance {
            text: text.into(),
            final_by_recogniser: true,
            silence_ms: 700,
            named: false,
        }
    }

    #[test]
    fn words_are_compared_without_accents_case_or_punctuation() {
        assert_eq!(plain("¿Qué hora es, Archie?"), "que hora es archie");
        assert!(names_archie("Oye Archi, abre Firefox."));
        assert!(!names_archie("la arquitectura del edificio"));
        for misheard in [
            "Archier, qué ves?",
            "Oye Arshi.",
            "Harchi, mira esto",
            "oye ar chi qué es esto",
        ] {
            assert!(names_archie(misheard), "{misheard}");
        }
        for other in [
            "Busca el archivo.",
            "el arco del puente",
            "voy a marchar",
            "es muy ancho",
            "the arch",
        ] {
            assert!(!names_archie(other), "{other}");
        }
        assert!(is_acknowledgement("Sí."));
        assert!(is_acknowledgement("ya po"));
        assert!(!is_acknowledgement("sí, ábrelo"));
    }

    #[test]
    fn a_request_that_names_archie_gets_an_answer() {
        let decision = FloorPolicy::default()
            .decide(&after_utterance(&heard("Oye Archie, ¿qué hora es?"), None));
        assert_eq!(decision.action, FloorAction::TakeFloor);
        assert_eq!(decision.budget, SpeechBudget::Full);
    }

    #[test]
    fn talk_in_the_room_that_is_not_to_archie_is_left_alone() {
        let state = after_utterance(&heard("¿Y tú fuiste al cumpleaños de la Cata?"), None);
        let decision = FloorPolicy::default().decide(&state);
        assert_eq!(decision.action, FloorAction::Silent);
        assert!(state
            .evidence
            .iter()
            .any(|line| line.contains("neither names")));
    }

    #[test]
    fn a_reply_right_after_archie_spoke_needs_no_name() {
        let answering = after_utterance(
            &heard("Sí, el de la izquierda."),
            Some(Duration::from_secs(3)),
        );
        assert_eq!(
            FloorPolicy::default().decide(&answering).action,
            FloorAction::TakeFloor
        );
        let too_late = after_utterance(
            &heard("Sí, el de la izquierda."),
            Some(Duration::from_secs(40)),
        );
        assert_eq!(
            FloorPolicy::default().decide(&too_late).action,
            FloorAction::Silent
        );
    }

    #[test]
    fn an_acknowledgement_neither_starts_nor_stops_archie() {
        let after = after_utterance(&heard("ya"), Some(Duration::from_secs(1)));
        assert_eq!(
            FloorPolicy::default().decide(&after).action,
            FloorAction::Silent
        );
        assert_eq!(
            FloorPolicy::default().decide(&over_archie("mhm")).action,
            FloorAction::ContinueSpeaking
        );
        assert_eq!(
            FloorPolicy::default()
                .decide(&over_archie("no, espera"))
                .action,
            FloorAction::Yield
        );
        assert_eq!(
            FloorPolicy::default().decide(&over_archie("")).action,
            FloorAction::Yield,
            "a voice with no words yet is a bid for the floor"
        );
    }

    #[test]
    fn a_question_has_the_floor_after_a_breath_and_a_statement_after_a_thought() {
        let at = |text: &str, silence_ms| {
            after_utterance(
                &Utterance {
                    text: text.into(),
                    final_by_recogniser: true,
                    silence_ms,
                    named: false,
                },
                None,
            )
        };
        assert_eq!(
            at("¿Qué hora es en Tokio?", 700).silence_scope,
            SilenceScope::EndOfTurn
        );
        let statement = "el informe de ventas quedó en el escritorio y lo revisamos mañana";
        assert_eq!(
            at(statement, 700).silence_scope,
            SilenceScope::BetweenClauses
        );
        assert!(at(statement, 700).thought_completion.get() < 0.5);
        assert_eq!(at(statement, 1900).silence_scope, SilenceScope::EndOfTurn);
    }

    #[test]
    fn a_fragment_is_not_enough_to_reason_over() {
        assert!(context_sufficiency("bueno, eh") < 0.4);
        assert!(context_sufficiency("¿qué es esto?") >= 0.4);
        assert!(context_sufficiency("abre firefox") >= 0.4);
        assert!(
            context_sufficiency("el informe de ventas quedó en el escritorio y lo revisamos")
                >= 0.4
        );
        let fragment = after_utterance(
            &Utterance {
                text: "bueno, eh".into(),
                final_by_recogniser: true,
                silence_ms: 2500,
                named: false,
            },
            None,
        );
        let mut asked = fragment.clone();
        asked.response_utility = Probability::new(0.9); // as --always sets it
        assert_eq!(
            FloorPolicy::default().decide(&asked).action,
            FloorAction::Silent,
            "answering everything still needs something to answer"
        );
    }

    #[test]
    fn the_model_can_choose_silence() {
        assert!(is_silence_marker(SILENCE_MARKER));
        assert!(is_silence_marker(" [Silencio]. "));
        assert!(is_silence_marker("[silence]"));
        assert!(!is_silence_marker("Silencio en la sala, por favor."));
        assert!(!is_silence_marker("Cuatro."));
    }

    #[test]
    fn archie_stops_when_told_by_name() {
        for stop in [
            "Archie, para.",
            "Archie espera",
            "archi, stop",
            "Archie, basta ya",
        ] {
            assert!(is_stop_request(stop), "{stop}");
        }
        for not_stop in [
            "para que veas",
            "dale, sigue",
            "Archie, ábrelo",
            "wait for it",
        ] {
            assert!(!is_stop_request(not_stop), "{not_stop}");
        }
    }

    #[test]
    fn a_sentence_left_hanging_is_not_the_end_of_the_turn() {
        let hanging = Utterance {
            text: "Archie, abre el formulario y".into(),
            final_by_recogniser: false,
            silence_ms: 450,
            named: false,
        };
        assert_eq!(
            FloorPolicy::default()
                .decide(&after_utterance(&hanging, None))
                .action,
            FloorAction::Silent
        );
    }
}
