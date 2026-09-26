//! Editing a schematic sheet in place: a symbol's fields, one atom at a time.
//!
//! The file keeps every byte KiCad wrote except the values that change, so the diff a
//! person reviews is exactly the edit.

use super::design::{placed, references};
use crate::sexpr::{parse, quote, splice, Node};
use std::collections::BTreeMap;

/// Fields a tool may not set. The reference is KiCad's annotation, kept consistent across
/// sheets and the board by KiCad itself.
const REFUSED: [&str; 3] = ["Reference", "Sheetname", "Sheetfile"];
const MAX_VALUE: usize = 1000;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("the sheet does not parse: {0}")]
    Parse(String),
    #[error("no symbol in this sheet is {0}")]
    NoSuchReference(String),
    #[error("{0} is set by KiCad's annotation, not by this tool")]
    Refused(String),
    #[error("a field name is 1 to 64 printable characters: {0:?}")]
    Name(String),
    #[error("{0}'s value is longer than {MAX_VALUE} bytes")]
    TooLong(String),
    #[error("no fields to set")]
    Nothing,
}

/// What changed, per field: the old value, or `None` where the field was added.
pub type Changes = BTreeMap<String, Option<String>>;

pub fn set_fields(
    text: &str,
    reference: &str,
    fields: &BTreeMap<String, String>,
) -> Result<(String, Changes, usize), EditError> {
    if fields.is_empty() {
        return Err(EditError::Nothing);
    }
    for (name, value) in fields {
        if REFUSED.iter().any(|r| r.eq_ignore_ascii_case(name)) {
            return Err(EditError::Refused(name.clone()));
        }
        if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
            return Err(EditError::Name(name.clone()));
        }
        if value.len() > MAX_VALUE {
            return Err(EditError::TooLong(name.clone()));
        }
    }
    let root = parse(text).map_err(|e| EditError::Parse(e.to_string()))?;
    let symbols: Vec<&Node> = placed(&root)
        .filter(|symbol| references(symbol).contains(reference))
        .collect();
    if symbols.is_empty() {
        return Err(EditError::NoSuchReference(reference.to_string()));
    }
    let mut edits = Vec::new();
    let mut changes = Changes::new();
    for symbol in &symbols {
        for (name, value) in fields {
            let existing = symbol
                .children("property")
                .find(|p| p.arg(0) == Some(name.as_str()));
            match existing.and_then(|p| p.items().get(2).map(|v| (p, v))) {
                Some((property, atom)) => {
                    changes
                        .entry(name.clone())
                        .or_insert_with(|| property.arg(1).map(str::to_string));
                    edits.push((atom.span(), quote(value)));
                }
                None => {
                    changes.entry(name.clone()).or_insert(None);
                    let at = symbol.child("at");
                    let x = at.and_then(|a| a.arg(0)).unwrap_or("0");
                    let y = at.and_then(|a| a.arg(1)).unwrap_or("0");
                    let close = symbol.span().end - 1;
                    edits.push((
                        close..close,
                        format!(
                            "\t(property {} {} (at {x} {y} 0) (effects (font (size 1.27 1.27)) (hide yes)))\n\t",
                            quote(name),
                            quote(value)
                        ),
                    ));
                }
            }
        }
    }
    let edited = splice(text, edits);
    // What was written must read back, with the new values in place.
    let check = parse(&edited).map_err(|e| EditError::Parse(e.to_string()))?;
    for symbol in placed(&check).filter(|s| references(s).contains(reference)) {
        for (name, value) in fields {
            let now = symbol
                .children("property")
                .find(|p| p.arg(0) == Some(name.as_str()))
                .and_then(|p| p.arg(1));
            if now != Some(value.as_str()) {
                return Err(EditError::Parse(format!("{name} did not read back")));
            }
        }
    }
    Ok((edited, changes, symbols.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = "(kicad_sch (version 20250114)
\t(lib_symbols (symbol \"Device:R\" (property \"Value\" \"R\")))
\t(symbol (lib_id \"Device:R\") (at 100 50 0) (unit 1)
\t\t(property \"Reference\" \"R1\" (at 0 0 0))
\t\t(property \"Value\" \"10k\" (at 0 0 0))
\t)
\t(symbol (lib_id \"Amplifier_Operational:LM358\") (at 10 10 0) (unit 1)
\t\t(property \"Reference\" \"U1\" (at 0 0 0))
\t\t(property \"Value\" \"LM358\" (at 0 0 0))
\t)
\t(symbol (lib_id \"Amplifier_Operational:LM358\") (at 20 10 0) (unit 2)
\t\t(property \"Reference\" \"U1\" (at 0 0 0))
\t\t(property \"Value\" \"LM358\" (at 0 0 0))
\t)
)";

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn a_value_changes_and_nothing_else_does() {
        let (edited, changes, symbols) =
            set_fields(SHEET, "R1", &fields(&[("Value", "4k7")])).unwrap();
        assert_eq!(edited, SHEET.replacen("\"10k\"", "\"4k7\"", 1));
        assert_eq!(changes["Value"].as_deref(), Some("10k"));
        assert_eq!(symbols, 1);
        assert!(
            edited.contains("(property \"Value\" \"R\")"),
            "the library is untouched"
        );
    }

    #[test]
    fn a_new_field_lands_on_every_unit_hidden() {
        let (edited, changes, symbols) =
            set_fields(SHEET, "U1", &fields(&[("MPN", "LM358DR")])).unwrap();
        assert_eq!(symbols, 2);
        assert_eq!(changes["MPN"], None);
        assert_eq!(
            edited.matches("(property \"MPN\" \"LM358DR\" (at").count(),
            2
        );
        assert!(edited.contains("(hide yes)"));
    }

    #[test]
    fn annotation_and_unknown_parts_are_refused() {
        assert_eq!(
            set_fields(SHEET, "R1", &fields(&[("Reference", "R2")])),
            Err(EditError::Refused("Reference".into()))
        );
        assert_eq!(
            set_fields(SHEET, "R7", &fields(&[("Value", "1k")])),
            Err(EditError::NoSuchReference("R7".into()))
        );
        assert!(set_fields(SHEET, "R1", &fields(&[("Bad\nName", "x")])).is_err());
    }

    #[test]
    fn a_value_with_quotes_survives_the_round_trip() {
        let (edited, _, _) =
            set_fields(SHEET, "R1", &fields(&[("Value", "10k \"1%\" \\ 0603")])).unwrap();
        let root = parse(&edited).unwrap();
        let value = placed(&root)
            .next()
            .unwrap()
            .children("property")
            .find(|p| p.arg(0) == Some("Value"))
            .and_then(|p| p.arg(1))
            .unwrap()
            .to_string();
        assert_eq!(value, "10k \"1%\" \\ 0603");
    }
}
