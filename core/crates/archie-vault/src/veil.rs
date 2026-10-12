//! The veil: text that may leave the machine with its secrets and identities left behind.
//!
//! A cloud model reasons as well about `SECRET_1` as about the forty characters it stands
//! for, and as well about `PERSON_1` owing `COMPANY_1` as about the names. So before a
//! request leaves, what identifies or unlocks is replaced with a stand-in, the table that
//! reverses it stays here, and the answer has the real values put back on this machine.
//! The model writes the configuration file, the letter or the script; it never holds the
//! key, the address or the name that goes in it.
//!
//! [`crate::ContextGateway`] does this for Vault entries, with the entities the operator
//! named. This does it for text nobody filed first: what a person typed or dictated. It
//! finds what has a recognisable shape (credentials, e-mail addresses, telephone numbers,
//! payment cards, Chilean RUTs) and replaces the operator's named entities as well.
//!
//! # What it is not
//!
//! It removes identifiers and credentials, not meaning. "PERSON_1 was fired on Tuesday"
//! still says somebody was fired. Text graded `restricted` therefore still does not leave
//! the machine, veiled or not, and `secret` reaches no model at all; the veil is what lets
//! `internal` and `sensitive` work use a model the node does not run.
//!
//! Detection is by shape and is deterministic, so it can be tested and audited, and it
//! errs toward covering: a build hash covered as a secret costs nothing, since it comes
//! back intact. A value with no shape and no name (a street address the operator never
//! registered) is not found. [`Veiled::residue`] reports what a second look still finds,
//! and a caller sends nothing while that is not empty.

use crate::gateway::{Entity, EntityKind};
use std::collections::BTreeMap;
use std::fmt;
use zeroize::Zeroize;

/// What a covered value was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A credential: an API key, a token, a password, a private key.
    Secret,
    Email,
    Phone,
    Card,
    /// A Chilean RUT, with a valid check digit.
    Rut,
}

impl Kind {
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Secret => "SECRET",
            Self::Email => "EMAIL",
            Self::Phone => "PHONE",
            Self::Card => "CARD",
            Self::Rut => "RUT",
        }
    }
}

struct Span {
    start: usize,
    end: usize,
    kind: Kind,
}

/// Words after which the next value is a credential whatever it looks like.
const LABELS: [&str; 14] = [
    "password",
    "passwd",
    "pwd",
    "contraseña",
    "contrasena",
    "clave",
    "api_key",
    "apikey",
    "api-key",
    "token",
    "secret",
    "secreto",
    "access_key",
    "bearer",
];

/// Prefixes credentials are issued with.
const PREFIXES: [&str; 12] = [
    "sk-",
    "ghp_",
    "gho_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "hf_",
    "nvapi-",
    "glpat-",
    "AKIA",
    "AIza",
    "eyJ",
];

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// PEM blocks: a private key is a secret from `-----BEGIN` to the end of its `-----END`
/// line, and to the end of the text when the end is missing.
fn pem(text: &str, spans: &mut Vec<Span>) {
    let mut from = 0;
    while let Some(start) = text[from..].find("-----BEGIN ").map(|at| at + from) {
        let end = text[start..]
            .find("-----END ")
            .and_then(|at| {
                text[start + at + 9..]
                    .find("-----")
                    .map(|close| start + at + 9 + close + 5)
            })
            .unwrap_or(text.len());
        spans.push(Span {
            start,
            end,
            kind: Kind::Secret,
        });
        from = end;
    }
}

fn luhn(digits: &[u8]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(index, digit)| {
            let value = u32::from(digit - b'0');
            if index % 2 == 1 {
                let doubled = value * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            } else {
                value
            }
        })
        .sum();
    sum % 10 == 0
}

/// A RUT starting at `at`: `12.345.678-5` or `12345678-5`, check digit verified. Returns
/// where it ends.
fn rut_at(bytes: &[u8], at: usize) -> Option<usize> {
    let mut digits = Vec::new();
    let mut cursor = at;
    while let Some(&byte) = bytes.get(cursor) {
        match byte {
            b'0'..=b'9' => digits.push(byte),
            b'.' if bytes.get(cursor + 1).is_some_and(u8::is_ascii_digit) => {}
            _ => break,
        }
        cursor += 1;
    }
    if !(7..=8).contains(&digits.len()) || bytes.get(cursor) != Some(&b'-') {
        return None;
    }
    let check = bytes.get(cursor + 1)?.to_ascii_uppercase();
    if bytes.get(cursor + 2).is_some_and(|&next| is_word(next)) {
        return None;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .zip([2u32, 3, 4, 5, 6, 7].iter().cycle())
        .map(|(d, m)| u32::from(d - b'0') * m)
        .sum();
    let expected = match 11 - sum % 11 {
        11 => b'0',
        10 => b'K',
        value => b'0' + value as u8,
    };
    (check == expected).then_some(cursor + 2)
}

/// Numbers that identify: RUTs, payment cards, telephone numbers.
fn numbers(bytes: &[u8], spans: &mut Vec<Span>) {
    let mut at = 0;
    while at < bytes.len() {
        let starts = bytes[at].is_ascii_digit()
            || (bytes[at] == b'+' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit));
        if !starts || (at > 0 && is_word(bytes[at - 1])) {
            at += 1;
            continue;
        }
        if let Some(end) = rut_at(bytes, at) {
            spans.push(Span {
                start: at,
                end,
                kind: Kind::Rut,
            });
            at = end;
            continue;
        }
        // A run of digits, with single spaces, hyphens or dots between groups.
        let international = bytes[at] == b'+';
        let mut end = at + usize::from(international);
        let mut digits = Vec::new();
        while let Some(&byte) = bytes.get(end) {
            let separator = matches!(byte, b' ' | b'-' | b'.')
                && bytes.get(end + 1).is_some_and(u8::is_ascii_digit);
            if byte.is_ascii_digit() {
                digits.push(byte);
            } else if !separator {
                break;
            }
            end += 1;
        }
        let bounded = !bytes.get(end).is_some_and(|&next| is_word(next));
        let kind = match digits.len() {
            8..=15 if international => Some(Kind::Phone),
            13..=19 if luhn(&digits) => Some(Kind::Card),
            // A Chilean mobile written without its country code.
            9 if digits[0] == b'9' => Some(Kind::Phone),
            _ => None,
        };
        if let (true, Some(kind)) = (bounded, kind) {
            spans.push(Span {
                start: at,
                end,
                kind,
            });
        }
        at = end.max(at + 1);
    }
}

fn is_email(word: &str) -> bool {
    let Some((local, domain)) = word.split_once('@') else {
        return false;
    };
    let plain = |part: &str, extra: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
    };
    plain(local, "._%+-")
        && plain(domain, ".-")
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// Whether a word is shaped like an issued credential.
fn is_token(word: &str) -> bool {
    let token_chars = word
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_-+/=.~".contains(&b));
    if !token_chars || word.contains("://") {
        return false;
    }
    if PREFIXES.iter().any(|prefix| word.starts_with(prefix)) && word.len() >= 16 {
        return true;
    }
    let has = |test: fn(&u8) -> bool| word.bytes().any(|b| test(&b));
    let (digit, lower, upper) = (
        has(u8::is_ascii_digit),
        has(u8::is_ascii_lowercase),
        has(u8::is_ascii_uppercase),
    );
    let mut distinct: Vec<u8> = word.bytes().collect();
    distinct.sort_unstable();
    distinct.dedup();
    let hex = word.len() >= 32 && word.bytes().all(|b| b.is_ascii_hexdigit());
    word.len() >= 24 && distinct.len() >= 10 && digit && (hex || (lower && upper))
}

/// Credentials and addresses, word by word.
fn words(text: &str, spans: &mut Vec<Span>) {
    let trim: &[char] = &[
        '"', '\'', '`', '(', ')', '[', ']', '{', '}', '<', '>', ',', ';', '!', '?',
    ];
    let mut labelled = false;
    let mut offset = 0;
    for raw in text.split_inclusive(char::is_whitespace) {
        let start = offset;
        offset += raw.len();
        let word = raw.trim_end();
        let core = word.trim_matches(trim).trim_end_matches('.');
        if core.is_empty() {
            continue;
        }
        let core_start = start + word.find(core).unwrap_or(0);
        let was_labelled = std::mem::take(&mut labelled);
        let lowered = core.to_lowercase();

        // `password=hunter2` and `token:abc`: what follows the label in the same word.
        if let Some(at) = core.find(['=', ':']) {
            let label = core[..at].trim_matches(trim).to_lowercase();
            if LABELS.iter().any(|known| label.ends_with(known)) {
                let value = core[at + 1..].trim_matches(trim);
                if value.is_empty() {
                    labelled = true;
                } else {
                    let from = core_start + at + 1 + core[at + 1..].find(value).unwrap_or(0);
                    spans.push(Span {
                        start: from,
                        end: from + value.len(),
                        kind: Kind::Secret,
                    });
                }
                continue;
            }
        }
        // `password = hunter2`, `Bearer abc`: the label, perhaps a sign, then the secret.
        if was_labelled {
            if core == "=" || core == ":" {
                labelled = true;
            } else {
                spans.push(Span {
                    start: core_start,
                    end: core_start + core.len(),
                    kind: Kind::Secret,
                });
            }
            continue;
        }
        if LABELS.contains(&lowered.as_str()) {
            let next_is_sign = text[offset..].trim_start().starts_with(['=', ':']);
            labelled = next_is_sign || lowered == "bearer";
            continue;
        }
        if is_email(core) {
            spans.push(Span {
                start: core_start,
                end: core_start + core.len(),
                kind: Kind::Email,
            });
        } else if is_token(core) {
            spans.push(Span {
                start: core_start,
                end: core_start + core.len(),
                kind: Kind::Secret,
            });
        }
    }
}

/// What has a sensitive shape in `text`, in order, without overlaps.
fn spans_of(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    pem(text, &mut spans);
    numbers(text.as_bytes(), &mut spans);
    words(text, &mut spans);
    // Earliest first, and the longer of two that start together; then drop what overlaps.
    spans.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut kept: Vec<Span> = Vec::new();
    for span in spans {
        if kept.last().is_none_or(|last| span.start >= last.end) {
            kept.push(span);
        }
    }
    kept
}

/// Real value ↔ stand-in. Held on this machine for the life of one request.
#[derive(Default)]
pub struct Unveil {
    by_alias: BTreeMap<String, String>,
    by_value: BTreeMap<(String, String), String>,
    counters: BTreeMap<String, usize>,
}

impl Unveil {
    fn alias(&mut self, prefix: &str, value: &str) -> String {
        let key = (prefix.to_string(), value.to_string());
        if let Some(existing) = self.by_value.get(&key) {
            return existing.clone();
        }
        let counter = self.counters.entry(prefix.to_string()).or_insert(0);
        *counter += 1;
        let alias = format!("{prefix}_{counter}");
        self.by_alias.insert(alias.clone(), value.to_string());
        self.by_value.insert(key, alias.clone());
        alias
    }

    /// Puts the real values back into a model's answer. A stand-in the model made up
    /// stands for nothing and is left as it came.
    pub fn restore(&self, text: &str) -> String {
        // Longest first, so SECRET_12 is not read as SECRET_1 followed by a 2.
        let mut ordered: Vec<(&String, &String)> = self.by_alias.iter().collect();
        ordered.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(b.0)));
        ordered
            .into_iter()
            .fold(text.to_string(), |out, (alias, value)| {
                out.replace(alias.as_str(), value)
            })
    }

    /// How many values of each kind were covered: what may be written down about it.
    pub fn covered(&self) -> BTreeMap<String, usize> {
        self.counters.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.by_alias.is_empty()
    }
}

impl fmt::Debug for Unveil {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Counts only. The table is the one thing here that must not reach a log.
        f.debug_struct("Unveil")
            .field("covered", &self.counters)
            .finish_non_exhaustive()
    }
}

impl Drop for Unveil {
    fn drop(&mut self) {
        for value in self.by_alias.values_mut() {
            value.zeroize();
        }
    }
}

/// Texts ready to leave, and the table that reverses them.
#[derive(Debug)]
pub struct Veiled {
    pub texts: Vec<String>,
    pub unveil: Unveil,
}

impl Veiled {
    /// The kinds of sensitive value a second look still finds in what would be sent.
    /// Empty when the veil did its work; a caller sends nothing otherwise.
    pub fn residue(&self) -> Vec<Kind> {
        self.texts
            .iter()
            .flat_map(|text| {
                // A stand-in after a label (`password=SECRET_1`) is the veil's own work.
                spans_of(text)
                    .into_iter()
                    .filter(|span| {
                        !self
                            .unveil
                            .by_alias
                            .contains_key(&text[span.start..span.end])
                    })
                    .map(|span| span.kind)
            })
            .collect()
    }
}

/// Covers text before it leaves. Knows the operator's named entities.
#[derive(Debug, Default, Clone)]
pub struct Veil {
    entities: Vec<Entity>,
}

impl Veil {
    pub fn new() -> Self {
        Self::default()
    }

    /// A person, company, project or account whose name must not leave as it is.
    pub fn knowing(mut self, kind: EntityKind, value: impl Into<String>) -> Self {
        self.entities.push(Entity::new(kind, value));
        self
    }

    /// Covers every text with one table, so a value named in the system prompt and again
    /// in the question is the same stand-in in both.
    pub fn cover(&self, texts: &[&str]) -> Veiled {
        // Longest name first: with "Acme" and "Acme Holdings" both known, the short one
        // first would leave "COMPANY_1 Holdings".
        let mut entities: Vec<&Entity> = self
            .entities
            .iter()
            .filter(|e| !e.value.trim().is_empty())
            .collect();
        entities.sort_by(|a, b| {
            b.value
                .len()
                .cmp(&a.value.len())
                .then(a.value.cmp(&b.value))
        });
        let mut unveil = Unveil::default();
        let named = |segment: &str, unveil: &mut Unveil| {
            let mut segment = segment.to_string();
            for entity in &entities {
                if segment.contains(&entity.value) {
                    let alias = unveil.alias(&entity_prefix(&entity.kind), &entity.value);
                    segment = segment.replace(&entity.value, &alias);
                }
            }
            segment
        };
        let mut covered = Vec::with_capacity(texts.len());
        for text in texts {
            let mut out = String::with_capacity(text.len());
            let mut at = 0;
            for span in spans_of(text) {
                out.push_str(&named(&text[at..span.start], &mut unveil));
                out.push_str(&unveil.alias(span.kind.prefix(), &text[span.start..span.end]));
                at = span.end;
            }
            out.push_str(&named(&text[at..], &mut unveil));
            covered.push(out);
        }
        Veiled {
            texts: covered,
            unveil,
        }
    }
}

fn entity_prefix(kind: &EntityKind) -> String {
    match kind {
        EntityKind::Person => "PERSON".into(),
        EntityKind::Company => "COMPANY".into(),
        EntityKind::Contract => "CONTRACT".into(),
        EntityKind::Location => "LOCATION".into(),
        EntityKind::Account => "ACCOUNT".into(),
        EntityKind::Project => "PROJECT".into(),
        EntityKind::Other(name) => {
            let prefix: String = name
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_uppercase();
            if prefix.is_empty() {
                "ENTITY".into()
            } else {
                prefix
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cover(text: &str) -> Veiled {
        Veil::new().cover(&[text])
    }

    #[test]
    fn a_key_never_leaves_and_comes_back_where_the_model_put_its_stand_in() {
        let key = "sk-proj-9fQ2xLmA71bZtR4wYc8uHn3KpVeD6sJg";
        let veiled = cover(&format!(
            "Write a .env file for my service. The API key is {key} and the port is 8080."
        ));
        assert!(!veiled.texts[0].contains(key));
        assert_eq!(
            veiled.texts[0],
            "Write a .env file for my service. The API key is SECRET_1 and the port is 8080."
        );
        assert!(veiled.residue().is_empty());

        let answer = "API_KEY=SECRET_1\nPORT=8080\n";
        assert_eq!(
            veiled.unveil.restore(answer),
            format!("API_KEY={key}\nPORT=8080\n")
        );
        assert_eq!(
            veiled.unveil.restore("SECRET_7 stays"),
            "SECRET_7 stays",
            "a stand-in the model invented means nothing"
        );
    }

    #[test]
    fn a_password_is_covered_by_its_label_whatever_it_looks_like() {
        for text in [
            "password=hunter2 for the router",
            "password: hunter2 for the router",
            "password = hunter2 for the router",
            "la contraseña: hunter2 para el router",
            "Authorization: Bearer hunter2 for the router",
        ] {
            let veiled = cover(text);
            assert!(
                !veiled.texts[0].contains("hunter2"),
                "{text} -> {}",
                veiled.texts[0]
            );
            assert!(veiled.texts[0].contains("SECRET_1"), "{}", veiled.texts[0]);
            assert!(
                veiled.texts[0].ends_with("router"),
                "only the secret is covered: {}",
                veiled.texts[0]
            );
        }
        assert_eq!(
            cover("the token economy is growing").texts[0],
            "the token economy is growing",
            "a label with no sign names nothing"
        );
    }

    #[test]
    fn a_private_key_is_covered_whole() {
        let text = "Deploy with this:\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\nAAAA\n-----END OPENSSH PRIVATE KEY-----\nthen restart.";
        let veiled = cover(text);
        assert_eq!(
            veiled.texts[0],
            "Deploy with this:\nSECRET_1\nthen restart."
        );
        assert!(
            cover("-----BEGIN RSA PRIVATE KEY-----\nMIIE").texts[0].starts_with("SECRET_1"),
            "an unfinished block is covered to the end"
        );
    }

    #[test]
    fn addresses_numbers_and_ruts_are_covered_and_ordinary_numbers_are_not() {
        let veiled = cover(
            "Escribe a maria.soto@example.cl o llama al +56 9 8765 4321 (o al 987654321). \
             Su RUT es 12.345.678-5 y paga con 4111 1111 1111 1111. Son 3 cajas, pedido 20261011, a las 15:30.",
        );
        let text = &veiled.texts[0];
        for raw in [
            "maria.soto@example.cl",
            "8765 4321",
            "987654321",
            "12.345.678-5",
            "4111 1111",
        ] {
            assert!(!text.contains(raw), "{raw} left in: {text}");
        }
        assert_eq!(
            text,
            "Escribe a EMAIL_1 o llama al PHONE_1 (o al PHONE_2). Su RUT es RUT_1 y paga con CARD_1. \
             Son 3 cajas, pedido 20261011, a las 15:30."
        );
        assert_eq!(veiled.unveil.covered()["PHONE"], 2);
        assert!(veiled.residue().is_empty());

        // A RUT whose check digit is wrong is a number, and a card that fails Luhn is too.
        assert_eq!(
            cover("folio 12.345.678-9 y serie 4111 1111 1111 1112").texts[0],
            "folio 12.345.678-9 y serie 4111 1111 1111 1112"
        );
    }

    #[test]
    fn the_same_value_is_one_stand_in_across_the_system_prompt_and_the_question() {
        let veil = Veil::new()
            .knowing(EntityKind::Person, "Camila Rojas")
            .knowing(EntityKind::Company, "Acme")
            .knowing(EntityKind::Company, "Acme Holdings");
        let veiled = veil.cover(&[
            "You help Camila Rojas, who works at Acme Holdings.",
            "Draft an email from Camila Rojas (camila@acme.cl) to Acme about the Acme Holdings merger.",
        ]);
        assert_eq!(
            veiled.texts[0],
            "You help PERSON_1, who works at COMPANY_1."
        );
        assert_eq!(
            veiled.texts[1],
            "Draft an email from PERSON_1 (EMAIL_1) to COMPANY_2 about the COMPANY_1 merger."
        );
        assert_eq!(
            veiled
                .unveil
                .restore("Dear COMPANY_2 team, PERSON_1 (EMAIL_1) writes about COMPANY_1."),
            "Dear Acme team, Camila Rojas (camila@acme.cl) writes about Acme Holdings."
        );
    }

    #[test]
    fn code_and_prose_pass_through_and_the_table_does_not_reach_a_log() {
        let code = "fn route(registry: &ModelRegistry) -> RoutingOutcome { core/crates/archie-router/src/lib.rs https://docs.rs/tokio/1.40.0 }";
        assert_eq!(cover(code).texts[0], code);
        let veiled = cover("token: abc123def456");
        let shown = format!("{veiled:?}");
        assert!(!shown.contains("abc123def456"), "{shown}");
        assert!(
            shown.contains("SECRET"),
            "what kind was covered may be said: {shown}"
        );
    }
}
