//! A fixed rule, not a model: a question that shows signs of a crisis is not interpreted.
//!
//! When someone may be in danger, guessing at what others think is the wrong help. Wanda then
//! answers with a fixed message that points to emergency services and people who can help, and
//! no model output reaches the voice. The match is deliberately simple and errs towards
//! answering with the message; the message is safe to say when the match was wrong.

use crate::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    SelfHarm,
    Violence,
}

const SELF_HARM: &[&str] = &[
    "suicid",
    "quitarme la vida",
    "matarme",
    "no quiero vivir",
    "no quiero seguir viviendo",
    "hacerme dano",
    "autolesion",
    "acabar con mi vida",
    "kill myself",
    "end my life",
    "self-harm",
    "self harm",
    "hurt myself",
    "don't want to live",
    "dont want to live",
    "want to die",
    "quiero morir",
];

const VIOLENCE: &[&str] = &[
    "me va a matar",
    "me esta pegando",
    "me pega",
    "me amenaza con",
    "me quiere hacer dano",
    "abuso de mi",
    "going to kill me",
    "hits me",
    "is hitting me",
    "threatening to hurt me",
    "threatened to kill",
];

/// Lower-case and without the accents that are optional on a phone keyboard.
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            '’' => '\'',
            c => c,
        })
        .collect()
}

pub fn crisis(text: &str) -> Option<Signal> {
    let text = fold(text);
    if SELF_HARM.iter().any(|k| text.contains(k)) {
        Some(Signal::SelfHarm)
    } else if VIOLENCE.iter().any(|k| text.contains(k)) {
        Some(Signal::Violence)
    } else {
        None
    }
}

pub fn crisis_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Es => "Esto es importante y no es algo que deba interpretar yo. Si tú u otra persona estáis en peligro ahora, llama al número de emergencias de tu país. Si estás pensando en hacerte daño, puedes hablar ahora mismo con una línea de ayuda: en Chile, *4141; en España, el 024. Y si puedes, habla también con alguien de confianza.",
        Lang::En => "This matters, and it isn't something I should interpret. If you or someone else is in danger right now, call your local emergency number. If you're thinking about hurting yourself, you can talk to a crisis line right now: in the US, call or text 988. And if you can, reach out to someone you trust.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_are_found_with_or_without_accents() {
        assert_eq!(crisis("No quiero vivir más"), Some(Signal::SelfHarm));
        assert_eq!(
            crisis("creo que voy a hacerme daño"),
            Some(Signal::SelfHarm)
        );
        assert_eq!(
            crisis("creo que voy a hacerme dano"),
            Some(Signal::SelfHarm)
        );
        assert_eq!(
            crisis("My partner hits me when he drinks"),
            Some(Signal::Violence)
        );
        assert_eq!(crisis("¿Sabe Pedro que el viaje es en julio?"), None);
        assert!(crisis_message(Lang::Es).contains("emergencias"));
    }
}
