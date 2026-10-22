//! Words as they arrive, cut into pieces a voice can say.
//!
//! A model streams sub-word fragments: "Un vol", "cán es", " una". A voice cannot say
//! those. It can say a sentence, and when a sentence runs long it can say a clause, and
//! the listener hears the pause as a pause rather than as a stumble. So text is held until
//! it reaches one of those places, and released there — never in the middle of a word,
//! because a fragment that cannot be spoken alone cannot be committed alone either.
//!
//! What a voice should not read is removed on the way: the asterisks and hashes of
//! markdown, bullet dashes, and line breaks used as layout. The persona asks the model not
//! to write them, and models write them anyway.

/// A piece ready to be spoken, and whether it closes a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spoken {
    pub text: String,
    pub ends_sentence: bool,
}

/// Holds streamed text until it reaches a place where a voice can stop.
#[derive(Debug, Clone)]
pub struct Segmenter {
    buffer: String,
    /// How long a sentence may run before it is released at a clause instead.
    clause_after: usize,
}

impl Default for Segmenter {
    fn default() -> Self {
        Self::new(110)
    }
}

impl Segmenter {
    pub fn new(clause_after: usize) -> Self {
        Self {
            buffer: String::new(),
            clause_after: clause_after.max(24),
        }
    }

    /// Takes the next fragment and returns whatever it completed.
    pub fn push(&mut self, fragment: &str) -> Vec<Spoken> {
        self.push_up_to(fragment, usize::MAX)
    }

    /// Append a fragment and release at most `limit` pieces. A realtime caller can
    /// subsequently pass an empty fragment to drain the retained text incrementally.
    pub fn push_up_to(&mut self, fragment: &str, limit: usize) -> Vec<Spoken> {
        self.buffer.push_str(fragment);
        let mut out = Vec::new();
        while out.len() < limit {
            if let Some(end) = sentence_end(&self.buffer) {
                let piece: String = self.buffer.drain(..end).collect();
                if let Some(spoken) = clean(&piece, true) {
                    out.push(spoken);
                }
                continue;
            }
            if self.buffer.chars().count() > self.clause_after {
                if let Some(end) = clause_end(&self.buffer) {
                    let piece: String = self.buffer.drain(..end).collect();
                    if let Some(spoken) = clean(&piece, false) {
                        out.push(spoken);
                    }
                    continue;
                }
            }
            break;
        }
        out
    }

    /// The answer ended. Whatever is held is released as a sentence.
    pub fn finish(&mut self) -> Option<Spoken> {
        let rest = std::mem::take(&mut self.buffer);
        clean(&rest, true)
    }
}

/// Where the first complete sentence in `text` ends, just past its punctuation and the
/// space after it. A full stop between digits ("3.5") or at the very end of the text so
/// far is not yet an end: the next fragment may continue the number or the sentence.
fn sentence_end(text: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (at, (_, c)) in chars.iter().enumerate() {
        let closes = matches!(c, '.' | '!' | '?' | '…' | '\n');
        if !closes {
            continue;
        }
        let Some((after_at, after)) = chars.get(at + 1) else {
            // Nothing after it yet: wait, it may be "3." of "3.5" or "..." still coming.
            if *c == '\n' {
                return Some(text.len());
            }
            return None;
        };
        if *c == '\n' {
            return Some(*after_at);
        }
        if after.is_whitespace() {
            // Past the space, so the next piece does not begin with one.
            return Some(after_at + after.len_utf8());
        }
    }
    None
}

/// Where the last clause break in `text` ends, for a sentence that has run long.
fn clause_end(text: &str) -> Option<usize> {
    let mut found = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (at, (_, c)) in chars.iter().enumerate() {
        if matches!(c, ',' | ';' | ':') {
            if let Some((after_at, after)) = chars.get(at + 1) {
                if after.is_whitespace() {
                    found = Some(after_at + after.len_utf8());
                }
            }
        }
    }
    found
}

/// The piece as a voice should say it, or nothing if nothing sayable is left.
fn clean(piece: &str, ends_sentence: bool) -> Option<Spoken> {
    let mut text = piece.replace("**", "").replace("__", "").replace('`', "");
    // Layout, not language: a line that begins with a bullet or a heading mark.
    let trimmed = text.trim_start();
    for mark in ["- ", "* ", "• ", "# ", "## ", "### "] {
        if let Some(rest) = trimmed.strip_prefix(mark) {
            text = rest.to_string();
            break;
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // A piece with no letters or digits is punctuation left over, and not worth a breath.
    if !text.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(Spoken {
        text,
        ends_sentence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(fragments: &[&str]) -> Vec<Spoken> {
        let mut segmenter = Segmenter::default();
        let mut out = Vec::new();
        for fragment in fragments {
            out.extend(segmenter.push(fragment));
        }
        out.extend(segmenter.finish());
        out
    }

    #[test]
    fn fragments_become_whole_sentences() {
        let out = run(&[
            "Un vol",
            "cán es una mon",
            "taña. Por den",
            "tro tiene magma.",
        ]);
        assert_eq!(
            out,
            vec![
                Spoken {
                    text: "Un volcán es una montaña.".into(),
                    ends_sentence: true
                },
                Spoken {
                    text: "Por dentro tiene magma.".into(),
                    ends_sentence: true
                },
            ]
        );
    }

    #[test]
    fn a_number_is_not_the_end_of_a_sentence() {
        let out = run(&["Mide 3", ".", "5 kilómetros. Listo."]);
        assert_eq!(out[0].text, "Mide 3.5 kilómetros.");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn a_long_sentence_is_released_at_a_clause_and_never_inside_a_word() {
        let long = "Cuando la lava sale por el cráter, baja por las laderas, quema lo que \
                    encuentra en el camino y se enfría despacio formando roca nueva";
        let mut segmenter = Segmenter::new(60);
        let out = segmenter.push(long);
        assert!(
            !out.is_empty(),
            "a long sentence does not wait for its full stop"
        );
        for piece in &out {
            assert!(!piece.ends_sentence, "a clause is not a sentence end");
            assert!(
                piece.text.ends_with(',') || piece.text.ends_with(';') || piece.text.ends_with(':'),
                "cut at punctuation, not mid-word: {:?}",
                piece.text
            );
        }
    }

    #[test]
    fn what_a_voice_should_not_read_is_removed() {
        let out = run(&["**Claro**, te lo abro.\n- Primero Firefox.\n"]);
        let said: Vec<&str> = out.iter().map(|piece| piece.text.as_str()).collect();
        assert_eq!(said, vec!["Claro, te lo abro.", "Primero Firefox."]);
    }

    #[test]
    fn punctuation_alone_is_not_worth_saying() {
        assert!(run(&["...", " \n"]).is_empty());
    }

    #[test]
    fn spanish_openers_stay_with_their_sentence() {
        let out = run(&["¿Quieres que ", "lo abra? ¡Listo!"]);
        let said: Vec<&str> = out.iter().map(|piece| piece.text.as_str()).collect();
        assert_eq!(said, vec!["¿Quieres que lo abra?", "¡Listo!"]);
    }
}
