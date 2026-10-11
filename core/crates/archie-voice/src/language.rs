//! Lightweight language detection shared by speech input and output.

/// Returns `"en"` or `"es"`; uncertain text uses Spanish as Archie's default.
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
    let mut spanish = lower
        .chars()
        .filter(|c| matches!(c, 'ñ' | '¿' | '¡'))
        .count()
        * 2;
    let mut english = 0;
    for word in lower.split(|c: char| !c.is_alphanumeric() && c != '\'' && !"áéíóúñ".contains(c))
    {
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

#[cfg(test)]
mod tests {
    use super::spoken_language;

    #[test]
    fn it_tells_spanish_from_english() {
        assert_eq!(spoken_language("¿Qué es esto que tengo en la mano?"), "es");
        assert_eq!(
            spoken_language("That's a Froakie, a water type Pokémon."),
            "en"
        );
        assert_eq!(spoken_language("What is this thing in my hand?"), "en");
        assert_eq!(spoken_language("Es un peluche de Froakie."), "es");
        assert_eq!(spoken_language("Froakie"), "es", "unsure means the default");
    }
}
