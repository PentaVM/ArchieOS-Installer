//! S-expressions, as KiCad writes them: `(kicad_sch (version 20250114) (symbol …))`.
//!
//! Each node keeps the byte span it came from, so an edit replaces one atom in the
//! original text and leaves every other byte — formatting, ordering, anything this parser
//! does not model — as KiCad wrote it.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Atom {
        /// The value, with a quoted string's escapes resolved.
        text: String,
        quoted: bool,
        span: Range<usize>,
    },
    List {
        items: Vec<Node>,
        span: Range<usize>,
    },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SexprError {
    #[error("unbalanced parentheses at byte {0}")]
    Unbalanced(usize),
    #[error("a string that never closes, from byte {0}")]
    OpenString(usize),
    #[error("nesting deeper than {0} levels")]
    TooDeep(usize),
    #[error("nothing to read")]
    Empty,
}

/// Deeper than any KiCad file; a guard against a file built to exhaust the stack.
pub const MAX_DEPTH: usize = 256;

impl Node {
    pub fn span(&self) -> Range<usize> {
        match self {
            Node::Atom { span, .. } | Node::List { span, .. } => span.clone(),
        }
    }

    pub fn items(&self) -> &[Node] {
        match self {
            Node::List { items, .. } => items,
            Node::Atom { .. } => &[],
        }
    }

    pub fn atom(&self) -> Option<&str> {
        match self {
            Node::Atom { text, .. } => Some(text),
            Node::List { .. } => None,
        }
    }

    /// The first atom of a list: `symbol` in `(symbol …)`.
    pub fn head(&self) -> Option<&str> {
        self.items().first().and_then(Node::atom)
    }

    /// The direct children that are lists headed `name`.
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.items()
            .iter()
            .filter(move |item| item.head() == Some(name))
    }

    pub fn child<'a>(&'a self, name: &str) -> Option<&'a Node> {
        self.items().iter().find(|item| item.head() == Some(name))
    }

    /// The `n`th item after the head, as an atom: `(at 10 20 0)`.`arg(1)` is `"20"`.
    pub fn arg(&self, n: usize) -> Option<&str> {
        self.items().get(n + 1).and_then(Node::atom)
    }

    /// `(name value …)` → value.
    pub fn value_of(&self, name: &str) -> Option<&str> {
        self.child(name).and_then(|c| c.arg(0))
    }

    /// Every list headed `name`, at any depth, in document order.
    pub fn descendants<'a>(&'a self, name: &'a str, out: &mut Vec<&'a Node>) {
        for item in self.items() {
            if item.head() == Some(name) {
                out.push(item);
            }
            item.descendants(name, out);
        }
    }
}

pub fn parse(text: &str) -> Result<Node, SexprError> {
    let bytes = text.as_bytes();
    let mut stack: Vec<(usize, Vec<Node>)> = Vec::new();
    let mut top: Option<Node> = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'(' => {
                if stack.len() == MAX_DEPTH {
                    return Err(SexprError::TooDeep(MAX_DEPTH));
                }
                stack.push((i, Vec::new()));
                i += 1;
            }
            b')' => {
                let (start, items) = stack.pop().ok_or(SexprError::Unbalanced(i))?;
                let node = Node::List {
                    items,
                    span: start..i + 1,
                };
                i += 1;
                match stack.last_mut() {
                    Some((_, parent)) => parent.push(node),
                    None if top.is_none() => top = Some(node),
                    None => return Err(SexprError::Unbalanced(start)),
                }
            }
            b'"' => {
                let start = i;
                let mut text = Vec::new();
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return Err(SexprError::OpenString(start)),
                        Some(b'"') => break,
                        Some(b'\\') => {
                            match bytes.get(i + 1) {
                                Some(b'n') => text.push(b'\n'),
                                Some(b't') => text.push(b'\t'),
                                Some(b'r') => text.push(b'\r'),
                                Some(&other) => text.push(other),
                                None => return Err(SexprError::OpenString(start)),
                            }
                            i += 2;
                        }
                        Some(&b) => {
                            text.push(b);
                            i += 1;
                        }
                    }
                }
                i += 1;
                let node = Node::Atom {
                    text: String::from_utf8_lossy(&text).into_owned(),
                    quoted: true,
                    span: start..i,
                };
                stack
                    .last_mut()
                    .ok_or(SexprError::Unbalanced(start))?
                    .1
                    .push(node);
            }
            _ => {
                let start = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b'"')
                {
                    i += 1;
                }
                let node = Node::Atom {
                    text: text[start..i].to_string(),
                    quoted: false,
                    span: start..i,
                };
                stack
                    .last_mut()
                    .ok_or(SexprError::Unbalanced(start))?
                    .1
                    .push(node);
            }
        }
    }
    if let Some((start, _)) = stack.last() {
        return Err(SexprError::Unbalanced(*start));
    }
    top.ok_or(SexprError::Empty)
}

/// A string as KiCad writes one: quoted, with `\`, `"` and line breaks escaped.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Replacements of byte spans in `text`, applied from the end so earlier spans stay valid.
/// Spans must not overlap.
pub fn splice(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    let mut out = text.to_string();
    for (span, replacement) in edits {
        out.replace_range(span, &replacement);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCH: &str = r#"(kicad_sch (version 20250114) (generator "eeschema")
  (symbol (lib_id "Device:R") (at 100.33 50.8 0) (unit 1)
    (property "Reference" "R1" (at 102 49 0))
    (property "Value" "10k \"precise\"" (at 102 52 0))
  )
)"#;

    #[test]
    fn a_schematic_reads_as_a_tree_with_spans() {
        let root = parse(SCH).unwrap();
        assert_eq!(root.head(), Some("kicad_sch"));
        assert_eq!(root.value_of("version"), Some("20250114"));
        let symbol = root.child("symbol").unwrap();
        assert_eq!(symbol.value_of("lib_id"), Some("Device:R"));
        assert_eq!(symbol.child("at").unwrap().arg(1), Some("50.8"));
        let value = symbol
            .children("property")
            .find(|p| p.arg(0) == Some("Value"))
            .unwrap();
        assert_eq!(value.arg(1), Some("10k \"precise\""));
        let span = value.items()[2].span();
        assert_eq!(&SCH[span], r#""10k \"precise\"""#);
    }

    #[test]
    fn an_edit_changes_one_atom_and_nothing_else() {
        let root = parse(SCH).unwrap();
        let value = root
            .child("symbol")
            .unwrap()
            .children("property")
            .find(|p| p.arg(0) == Some("Value"))
            .unwrap();
        let edited = splice(SCH, vec![(value.items()[2].span(), quote("4k7 \\ 1%"))]);
        assert_eq!(
            edited,
            SCH.replace(r#""10k \"precise\"""#, r#""4k7 \\ 1%""#)
        );
        let again = parse(&edited).unwrap();
        let value = again
            .child("symbol")
            .unwrap()
            .children("property")
            .find(|p| p.arg(0) == Some("Value"))
            .unwrap();
        assert_eq!(value.arg(1), Some("4k7 \\ 1%"));
    }

    #[test]
    fn broken_files_are_refused_not_guessed() {
        assert!(matches!(parse("(a (b)"), Err(SexprError::Unbalanced(0))));
        assert!(matches!(parse("(a))"), Err(SexprError::Unbalanced(3))));
        assert!(matches!(
            parse("(a \"open)"),
            Err(SexprError::OpenString(3))
        ));
        assert!(matches!(parse("(a)(b)"), Err(SexprError::Unbalanced(3))));
        assert_eq!(parse("  "), Err(SexprError::Empty));
        let deep = "(".repeat(MAX_DEPTH + 1) + &")".repeat(MAX_DEPTH + 1);
        assert_eq!(parse(&deep), Err(SexprError::TooDeep(MAX_DEPTH)));
    }
}
