//! Magpie language switching and Spanish phrasing.

pub use crate::language::spoken_language;

/// Voseo the model slips into ("¿querés…?"), and the tuteo Archie speaks. Told plainly
/// never to use it, Nemotron still did now and then.
const VOSEO: &[(&str, &str)] = &[
    ("querés", "quieres"),
    ("tenés", "tienes"),
    ("podés", "puedes"),
    ("sabés", "sabes"),
    ("decís", "dices"),
    ("sos", "eres"),
    ("vos", "tú"),
    ("referís", "refieres"),
    ("necesitás", "necesitas"),
    ("hacés", "haces"),
    ("mirá", "mira"),
    ("fijate", "fíjate"),
    ("decime", "dime"),
    ("contame", "cuéntame"),
    ("mostrame", "muéstrame"),
    ("esperá", "espera"),
    ("preguntá", "pregunta"),
    ("probá", "prueba"),
    ("tomá", "toma"),
    ("vení", "ven"),
    ("andá", "anda"),
    ("dejame", "déjame"),
    ("buscá", "busca"),
    ("acercá", "acerca"),
    ("acercalo", "acércalo"),
    ("mostralo", "muéstralo"),
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
        self.voice
            .set_language(&locale)
            .map_err(|e| e.to_string())?;
        crate::voice::Speaks::say(&mut self.voice, &text, hear)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn voseo_is_spoken_as_tuteo() {
        assert_eq!(
            super::tutear("¿Querés ir al océano? Si vos tenés tiempo, mirá esto."),
            "¿Quieres ir al océano? Si tú tienes tiempo, mira esto."
        );
        assert_eq!(
            super::tutear("Nada que cambiar aquí."),
            "Nada que cambiar aquí."
        );
    }
}
