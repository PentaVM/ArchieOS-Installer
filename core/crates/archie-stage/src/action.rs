//! What the agent can do with an application.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use zeroize::Zeroizing;

/// The longest text one action may type, in characters. Matches the compositor's bound.
pub const MAX_TEXT_CHARS: usize = 2_000;
/// The largest scroll one action may make, in wheel steps per axis.
pub const MAX_SCROLL_STEPS: i32 = 60;
/// The largest surface-local coordinate accepted. No output is this large.
pub const MAX_COORDINATE: i32 = 32_767;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

/// The kinds of action a policy grants, independent of their parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Move,
    Click,
    DoubleClick,
    Drag,
    Scroll,
    Text,
    Key,
    Focus,
}

impl ActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Click => "click",
            Self::DoubleClick => "double_click",
            Self::Drag => "drag",
            Self::Scroll => "scroll",
            Self::Text => "text",
            Self::Key => "key",
            Self::Focus => "focus",
        }
    }
}

/// Text the agent will type. Holds the characters for as long as it takes to seal them,
/// wipes them on drop, and never prints them.
#[derive(Clone, PartialEq, Eq)]
pub struct StageText(Zeroizing<String>);

impl StageText {
    pub fn new(text: impl Into<String>) -> Self {
        Self(Zeroizing::new(text.into()))
    }

    pub fn chars(&self) -> usize {
        self.0.chars().count()
    }

    /// The characters, for sealing. Nothing else in this crate reads them.
    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }

    /// Whether every character is one a person could type into a field without it meaning
    /// something else: no control characters, no bidirectional overrides.
    pub fn is_plain(&self) -> bool {
        self.0.chars().all(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
                )
        })
    }
}

impl fmt::Debug for StageText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StageText({} chars, redacted)", self.chars())
    }
}

// Serialised only inside a sealed command, which is the one place the characters must go.
impl Serialize for StageText {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for StageText {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

/// Keys with a name. Everything else a chord may end in is a single ASCII letter or digit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Space,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl NamedKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enter => "enter",
            Self::Tab => "tab",
            Self::Escape => "escape",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::Insert => "insert",
            Self::Home => "home",
            Self::End => "end",
            Self::PageUp => "page_up",
            Self::PageDown => "page_down",
            Self::ArrowUp => "arrow_up",
            Self::ArrowDown => "arrow_down",
            Self::ArrowLeft => "arrow_left",
            Self::ArrowRight => "arrow_right",
            Self::Space => "space",
            Self::F1 => "f1",
            Self::F2 => "f2",
            Self::F3 => "f3",
            Self::F4 => "f4",
            Self::F5 => "f5",
            Self::F6 => "f6",
            Self::F7 => "f7",
            Self::F8 => "f8",
            Self::F9 => "f9",
            Self::F10 => "f10",
            Self::F11 => "f11",
            Self::F12 => "f12",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        const ALL: [NamedKey; 27] = [
            NamedKey::Enter,
            NamedKey::Tab,
            NamedKey::Escape,
            NamedKey::Backspace,
            NamedKey::Delete,
            NamedKey::Insert,
            NamedKey::Home,
            NamedKey::End,
            NamedKey::PageUp,
            NamedKey::PageDown,
            NamedKey::ArrowUp,
            NamedKey::ArrowDown,
            NamedKey::ArrowLeft,
            NamedKey::ArrowRight,
            NamedKey::Space,
            NamedKey::F1,
            NamedKey::F2,
            NamedKey::F3,
            NamedKey::F4,
            NamedKey::F5,
            NamedKey::F6,
            NamedKey::F7,
            NamedKey::F8,
            NamedKey::F9,
            NamedKey::F10,
            NamedKey::F11,
            NamedKey::F12,
        ];
        ALL.into_iter().find(|key| key.as_str() == text)
    }
}

/// The key a chord ends in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Named(NamedKey),
    /// An ASCII lowercase letter or digit.
    Char(char),
}

impl Key {
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(named) = NamedKey::parse(text) {
            return Some(Self::Named(named));
        }
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_lowercase() || c.is_ascii_digit() => Some(Self::Char(c)),
            _ => None,
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Named(named) => named.as_str().to_string(),
            Self::Char(c) => c.to_string(),
        }
    }
}

impl Serialize for Key {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.name())
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Key::parse(&text).ok_or_else(|| serde::de::Error::custom("not a key this stage can press"))
    }
}

/// A key, with the modifiers held while it is pressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyChord {
    #[serde(default)]
    pub modifiers: BTreeSet<Modifier>,
    pub key: Key,
}

impl KeyChord {
    pub fn key(key: Key) -> Self {
        Self {
            modifiers: BTreeSet::new(),
            key,
        }
    }

    pub fn with(mut self, modifier: Modifier) -> Self {
        self.modifiers.insert(modifier);
        self
    }

    /// Keys pressed, counting modifiers. At most five by construction.
    pub fn key_count(&self) -> usize {
        self.modifiers.len() + 1
    }

    /// Whether this chord belongs to the system rather than to any application.
    ///
    /// `Super` is the compositor's. `Ctrl+Alt` reaches virtual-terminal switching and the
    /// emergency chords on the way down, whatever application has focus.
    pub fn is_reserved(&self) -> bool {
        self.modifiers.contains(&Modifier::Super)
            || (self.modifiers.contains(&Modifier::Ctrl) && self.modifiers.contains(&Modifier::Alt))
    }
}

/// One action, with its parameters. Coordinates are in the surface's own logical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageAction {
    Move {
        x: i32,
        y: i32,
    },
    Click {
        x: i32,
        y: i32,
        button: PointerButton,
    },
    DoubleClick {
        x: i32,
        y: i32,
        button: PointerButton,
    },
    Drag {
        x: i32,
        y: i32,
        to_x: i32,
        to_y: i32,
        button: PointerButton,
    },
    Scroll {
        x: i32,
        y: i32,
        steps_x: i32,
        steps_y: i32,
    },
    Text {
        text: StageText,
    },
    Key {
        chord: KeyChord,
    },
    Focus,
}

impl StageAction {
    pub fn kind(&self) -> ActionKind {
        match self {
            Self::Move { .. } => ActionKind::Move,
            Self::Click { .. } => ActionKind::Click,
            Self::DoubleClick { .. } => ActionKind::DoubleClick,
            Self::Drag { .. } => ActionKind::Drag,
            Self::Scroll { .. } => ActionKind::Scroll,
            Self::Text { .. } => ActionKind::Text,
            Self::Key { .. } => ActionKind::Key,
            Self::Focus => ActionKind::Focus,
        }
    }

    /// Every surface-local point this action names.
    pub fn points(&self) -> Vec<(i32, i32)> {
        match self {
            Self::Move { x, y }
            | Self::Click { x, y, .. }
            | Self::DoubleClick { x, y, .. }
            | Self::Scroll { x, y, .. } => vec![(*x, *y)],
            Self::Drag {
                x, y, to_x, to_y, ..
            } => vec![(*x, *y), (*to_x, *to_y)],
            Self::Text { .. } | Self::Key { .. } | Self::Focus => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_names_and_single_characters_only() {
        assert_eq!(Key::parse("enter"), Some(Key::Named(NamedKey::Enter)));
        assert_eq!(Key::parse("f12"), Some(Key::Named(NamedKey::F12)));
        assert_eq!(Key::parse("s"), Some(Key::Char('s')));
        assert_eq!(Key::parse("7"), Some(Key::Char('7')));
        for bad in ["S", "ss", "", "ñ", "ctrl", "super", "\n"] {
            assert_eq!(Key::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn super_and_ctrl_alt_are_the_systems() {
        assert!(KeyChord::key(Key::Char('l'))
            .with(Modifier::Super)
            .is_reserved());
        assert!(KeyChord::key(Key::Named(NamedKey::Delete))
            .with(Modifier::Ctrl)
            .with(Modifier::Alt)
            .is_reserved());
        assert!(!KeyChord::key(Key::Char('s'))
            .with(Modifier::Ctrl)
            .is_reserved());
        assert!(!KeyChord::key(Key::Named(NamedKey::F4))
            .with(Modifier::Alt)
            .is_reserved());
    }

    #[test]
    fn text_never_prints_and_knows_what_is_not_plain() {
        let text = StageText::new("hunter2 ñandú");
        assert_eq!(format!("{text:?}"), "StageText(13 chars, redacted)");
        assert!(text.is_plain());
        assert!(!StageText::new("ls\n").is_plain());
        assert!(!StageText::new("a\u{1b}[2J").is_plain());
        assert!(!StageText::new("txt\u{202E}exe").is_plain());
    }
}
