//! Speaking the language the person spoke.
//!
//! The ears transcribe whatever language they hear; the model is told to answer in it; and
//! the voice, which is one multilingual Magpie model, is switched per sentence to the
//! language that sentence is written in. Telling Spanish from English by their commonest
//! words and letters is crude, and it is enough: a sentence of either is full of them.

/// `"en"` or `"es"`; Spanish when it cannot tell, because Archie's default is Spanish.
pub fn spoken_language(text: &str) -> &'static str {
    const SPANISH: &[&str] = &[
        "el", "la", "los", "las", "que", "de", "y", "en", "es", "un", "una", "por", "para", "con",
        "no", "sí", "qué", "cómo", "lo", "se", "del", "al", "está", "eso", "esto", "pero", "muy",
        "yo", "tú", "te", "me", "hay", "son", "ya",
    ];
    const ENGLISH: &[&str] = &[
        "the", "and", "is", "are", "you", "what", "how", "this", "that", "it", "of", "to", "in",
        "for", "with", "not", "yes", "an", "i", "it's", "that's", "do", "does", "can", "my",
        "your", "have", "was", "be",
    ];
    let lower = text.to_lowercase();
    // Accented vowels are not counted: English names carry them too ("Pokémon").
    let mut spanish = lower
        .chars()
        .filter(|c| matches!(c, 'ñ' | '¿' | '¡'))
        .count()
        * 2;
    let mut english = 0;
    for word in lower.split(|c: char| !c.is_alphanumeric() && c != '\'' && !"áéíóúñ".contains(c)) {
        if SPANISH.contains(&word) {
            spanish += 1;
        }
        if ENGLISH.contains(&word) {
            english += 1;
        }
    }
    if english > spanish {
        "en"
    } else {
        "es"
    }
}

/// Voseo the model slips into ("¿querés…?"), and the tuteo Archie speaks. Told plainly
/// never to use it, Nemotron still did now and then.
const VOSEO: &[(&str, &str)] = &[
    ("querés", "quieres"), ("tenés", "tienes"), ("podés", "puedes"), ("sabés", "sabes"),
    ("decís", "dices"), ("sos", "eres"), ("vos", "tú"), ("referís", "refieres"),
    ("necesitás", "necesitas"), ("hacés", "haces"), ("mirá", "mira"),
    ("fijate", "fíjate"), ("decime", "dime"), ("contame", "cuéntame"), ("mostrame", "muéstrame"),
    ("esperá", "espera"), ("preguntá", "pregunta"), ("probá", "prueba"), ("tomá", "toma"),
    ("vení", "ven"), ("andá", "anda"), ("dejame", "déjame"),
    ("buscá", "busca"), ("acercá", "acerca"), ("acercalo", "acércalo"), ("mostralo", "muéstralo"),
];

/// Speaks `text` in tuteo: each voseo word becomes its tú form, keeping a capital.
pub fn tutear(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        let lower = word.to_lowercase();
        match VOSEO.iter().find(|(vos, _)| *vos == lower) {
            Some((_, tu)) => {
                let mut chars = tu.chars();
                if word.chars().next().is_some_and(char::is_uppercase) {
                    if let Some(first) = chars.next() {
                        out.extend(first.to_uppercase());
                    }
                }
                out.extend(chars);
            }
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_alphabetic() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// A Magpie voice that says each sentence in the language it is written in.
#[cfg(feature = "nemo")]
pub struct FollowingVoice {
    voice: crate::native::Voice,
    spanish: String,
    english: String,
}

#[cfg(feature = "nemo")]
impl FollowingVoice {
    /// `spanish` and `english` are the locales Magpie is given, e.g. `es-US` and `en-US`.
    pub fn new(voice: crate::native::Voice, spanish: &str, english: &str) -> Self {
        Self {
            voice,
            spanish: spanish.into(),
            english: english.into(),
        }
    }
}

#[cfg(feature = "nemo")]
impl crate::voice::Speaks for FollowingVoice {
    fn sample_rate(&self) -> u32 {
        self.voice.sample_rate()
    }

    fn say(
        &mut self,
        text: &str,
        hear: &mut dyn FnMut(&[i16]) -> bool,
    ) -> Result<crate::voice::Said, String> {
        let (locale, text) = match spoken_language(text) {
            "en" => (self.english.clone(), text.to_string()),
            _ => (self.spanish.clone(), tutear(text)),
        };
        self.voice.set_language(&locale).map_err(|e| e.to_string())?;
        crate::voice::Speaks::say(&mut self.voice, &text, hear)
    }
}

#[cfg(test)]
mod tests {
    use super::spoken_language;

    #[test]
    fn it_tells_spanish_from_english() {
        assert_eq!(spoken_language("¿Qué es esto que tengo en la mano?"), "es");
        assert_eq!(spoken_language("That's a Froakie, a water type Pokémon."), "en");
        assert_eq!(spoken_language("What is this thing in my hand?"), "en");
        assert_eq!(spoken_language("Es un peluche de Froakie."), "es");
        assert_eq!(spoken_language("Froakie"), "es", "unsure means the default");
    }

    #[test]
    fn voseo_is_spoken_as_tuteo() {
        assert_eq!(
            super::tutear("¿Querés ir al océano? Si vos tenés tiempo, mirá esto."),
            "¿Quieres ir al océano? Si tú tienes tiempo, mira esto."
        );
        assert_eq!(super::tutear("Nada que cambiar aquí."), "Nada que cambiar aquí.");
    }
}
