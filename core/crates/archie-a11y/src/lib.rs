//! # archie-a11y
//!
//! What an application says about its own controls.
//!
//! Every toolkit already publishes its buttons, fields and menu items, with their names,
//! their roles and where they are, so that a screen reader can work. That is a structured
//! source in IRIS's sense: it needs no pixels, no model and no guess, and it answers in
//! milliseconds. Archie reads it for the same reason a screen reader does — to find the
//! control a task names — and for nothing else.
//!
//! It perceives; it decides nothing. What it returns is a proposal like any other: the
//! stage gate still authorises the click, and the window still has to react to it.
//!
//! On Linux this is AT-SPI over D-Bus. Elsewhere the crate compiles to the same API,
//! reporting that nothing declares anything.

#![forbid(unsafe_code)]

use archie_iris::{NodeKind, Rect};

#[cfg(unix)]
mod atspi;

#[cfg(unix)]
pub use atspi::Accessibility;

/// A control an application declares, in the coordinates of its own window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// What the application calls it: "Guardar", "Nota rápida".
    pub name: String,
    pub kind: NodeKind,
    pub bounds: Rect,
    /// Where it sits on the bus, which is as stable as the toolkit's own object.
    pub path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DeclareError {
    #[error("there is no accessibility bus to ask")]
    NoBus,
    #[error("the accessibility bus refused or did not answer: {0}")]
    Bus(String),
    #[error("no application on that bus belongs to process {0}")]
    NoApplication(u32),
}

/// The AT-SPI roles Archie can act on, and what they are in a scene.
///
/// Anything else an application declares is not a target: a label, a panel or a filler is
/// not something to click, and calling one a button because it was the only match would be
/// exactly the sort of guess this path exists to avoid.
pub fn kind_of_role(role: &str) -> Option<NodeKind> {
    match role {
        "push button" | "button" | "toggle button" => Some(NodeKind::Button),
        "text" | "entry" | "password text" | "spin button" => Some(NodeKind::Input),
        "link" => Some(NodeKind::Link),
        "check box" | "check menu item" | "radio button" => Some(NodeKind::Checkbox),
        "page tab" => Some(NodeKind::Tab),
        "menu item" | "menu" => Some(NodeKind::MenuItem),
        _ => None,
    }
}

/// Lower case, without accents and without repeated spaces: how two names written by
/// different hands are compared.
fn folded(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = true;
    for character in text.chars() {
        let plain = match character {
            'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
            'ñ' | 'Ñ' => 'n',
            other => other.to_ascii_lowercase(),
        };
        if plain.is_whitespace() || plain == '_' {
            if !space {
                out.push(' ');
                space = true;
            }
        } else if plain.is_alphanumeric() {
            out.push(plain);
            space = false;
        }
    }
    out.trim_end().to_string()
}

/// The one control a description names, or none.
///
/// A description is what a task wrote — "el botón Guardar" — and a name is what the
/// application wrote — "Guardar". A control is the answer when its name appears in the
/// description, or the description in its name, and its kind is the kind asked for. Two
/// controls that both fit are no answer at all: an ambiguous window is what the grounding
/// model is for.
pub fn control_named<'a>(
    controls: &'a [Control],
    description: &str,
    kind: NodeKind,
) -> Option<&'a Control> {
    let wanted = folded(description);
    if wanted.is_empty() {
        return None;
    }
    let mut fitting: Vec<&Control> = controls
        .iter()
        .filter(|control| {
            if control.kind != kind || control.name.trim().is_empty() {
                return false;
            }
            let name = folded(&control.name);
            !name.is_empty() && (wanted.contains(&name) || name.contains(&wanted))
        })
        .collect();
    if fitting.len() <= 1 {
        return fitting.first().copied();
    }
    // Several fit. If the description spelled one of them out, the longest name it spells
    // is the one it meant: "el botón Guardar como" names the second of two buttons, not
    // the first. Anything else — two controls with the same name, or a description short
    // enough to fit inside several — stays ambiguous, and an ambiguous window is what a
    // grounding model is for.
    fitting.retain(|control| wanted.contains(&folded(&control.name)));
    fitting.sort_by_key(|control| std::cmp::Reverse(folded(&control.name).chars().count()));
    let longest = fitting.first()?;
    let length = folded(&longest.name).chars().count();
    match fitting.get(1) {
        Some(next) if folded(&next.name).chars().count() == length => None,
        _ => Some(longest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(name: &str, kind: NodeKind) -> Control {
        Control {
            name: name.into(),
            kind,
            bounds: Rect::new(10, 20, 30, 40).unwrap(),
            path: format!("/org/a11y/atspi/accessible/{name}"),
        }
    }

    #[test]
    fn only_things_worth_clicking_have_a_kind() {
        assert_eq!(kind_of_role("push button"), Some(NodeKind::Button));
        assert_eq!(kind_of_role("entry"), Some(NodeKind::Input));
        assert_eq!(kind_of_role("page tab"), Some(NodeKind::Tab));
        assert_eq!(
            kind_of_role("label"),
            None,
            "a label is not something to click, however well it matches"
        );
        assert_eq!(kind_of_role("filler"), None);
        assert_eq!(kind_of_role("frame"), None);
    }

    #[test]
    fn a_task_and_an_application_can_write_the_same_name_differently() {
        let controls = [
            control("Guardar", NodeKind::Button),
            control("Cancelar", NodeKind::Button),
            control("Nota rápida", NodeKind::Input),
        ];
        assert_eq!(
            control_named(&controls, "el botón Guardar", NodeKind::Button).map(|c| c.name.as_str()),
            Some("Guardar")
        );
        assert_eq!(
            control_named(&controls, "GUARDAR", NodeKind::Button).map(|c| c.name.as_str()),
            Some("Guardar")
        );
        assert_eq!(
            control_named(&controls, "el campo nota rapida", NodeKind::Input)
                .map(|c| c.name.as_str()),
            Some("Nota rápida"),
            "accents and case are how it was written, not what it is"
        );
        assert!(
            control_named(&controls, "el botón Guardar", NodeKind::Input).is_none(),
            "the right name in the wrong kind of control is not the target"
        );
        assert!(
            control_named(&controls, "el botón que sea", NodeKind::Button).is_none(),
            "a description that fits nothing fits nothing"
        );
    }

    #[test]
    fn two_controls_that_both_fit_are_no_answer() {
        let controls = [
            control("Guardar", NodeKind::Button),
            control("Guardar como", NodeKind::Button),
        ];
        assert!(
            control_named(&controls, "el botón guardar como", NodeKind::Button).is_some(),
            "one of them is exactly what was asked for"
        );
        assert_eq!(
            control_named(&controls, "guardar", NodeKind::Button).map(|c| c.name.as_str()),
            Some("Guardar"),
            "and here too, exactly"
        );
        let both = [
            control("Abrir", NodeKind::Button),
            control("Abrir", NodeKind::Button),
        ];
        assert!(
            control_named(&both, "el botón de abrir algo", NodeKind::Button).is_none(),
            "an ambiguous window is what a grounding model is for"
        );
    }
}
