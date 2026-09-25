//! What the agent can do with an application.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use zeroize::Zeroizing;

/// The longest text one action may type, in characters. Matches the compositor's bound.
pub const MAX_TEXT_CHARS: usize = 2_000;
/// The largest scroll one action may make, in wheel steps per axis.
pub const MAX_SCROLL_STEPS: i32 = 60;
/// The most points one stroke may carry. A longer line is more than one stroke.
pub const MAX_STROKE_POINTS: usize = 256;

/// How many separate lines one drawing command may carry.
///
/// A line costs the same to send whether it is long or short, and the round trip is most of
/// what it costs: a drawing of forty lines spent a third of its time waiting for receipts.
/// Several lines under one authorisation is the same work with the waiting taken out. It is
/// still one operation — one lease, one receipt — and every point of every line is still
/// checked against the window as it is now, which is what made a single line safe.
pub const MAX_STROKE_PATHS: usize = 64;

/// How many points one drawing command may carry across all of its lines.
///
/// A sealed command may not be longer than thirty-two thousand bytes, and a point costs
/// about twenty of them written out, so this is what fits with room to spare. A drawing
/// longer than that is simply sent as two commands, which costs one round trip.
pub const MAX_STROKE_POINTS_IN_ALL: usize = 1024;
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
    Snapshot,
    Stroke,
    /// Keeping a key down, for as long as a game needs "walk forward". A separate grant:
    /// an application that takes keys does not thereby take held keys.
    Hold,
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
            Self::Stroke => "stroke",
            Self::Focus => "focus",
            Self::Snapshot => "snapshot",
            Self::Hold => "hold",
        }
    }

    /// What the action does in policy terms. A capture changes nothing in the
    /// application, so it reads; everything else operates it.
    pub fn policy_action(&self) -> archie_policy::Action {
        match self {
            Self::Snapshot => archie_policy::Action::Read,
            _ => archie_policy::Action::Execute,
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

/// The marks a keyboard has without a shift, beside its letters and digits: bracket left
/// and right, minus, equal, semicolon, apostrophe, comma, period, slash, backslash and
/// grave. They are written by code point because two of them have to be escaped in a
/// literal, and an escape is a thing to get wrong quietly.
fn is_mark(c: char) -> bool {
    matches!(
        c as u32,
        0x5B | 0x5D | 0x2D | 0x3D | 0x3B | 0x27 | 0x2C | 0x2E | 0x2F | 0x5C | 0x60
    )
}

impl Key {
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(named) = NamedKey::parse(text) {
            return Some(Self::Named(named));
        }
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            // A letter, a digit, or one of the marks a keyboard has without a shift.
            (Some(c), None) if c.is_ascii_lowercase() || c.is_ascii_digit() || is_mark(c) => {
                Some(Self::Char(c))
            }
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

    /// `enter`, `ctrl+s`, `ctrl+shift+z`: modifiers first, the key last, joined by `+`.
    /// Parsing says nothing about whether a chord may be sent; the gate decides that.
    pub fn parse(spec: &str) -> Option<Self> {
        let mut parts: Vec<&str> = spec.split('+').collect();
        let mut chord = Self::key(Key::parse(parts.pop()?)?);
        for part in parts {
            chord = chord.with(match part {
                "ctrl" => Modifier::Ctrl,
                "shift" => Modifier::Shift,
                "alt" => Modifier::Alt,
                "super" => Modifier::Super,
                _ => return None,
            });
        }
        Some(chord)
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
    /// Press, follow a way through the surface, and let go: one continuous line, which is
    /// what drawing is. A drag is the two-point case of it.
    Stroke {
        points: Vec<StrokePoint>,
        button: PointerButton,
    },
    /// Several lines, drawn one after another without letting go of the authorisation: the
    /// pointer presses, draws, lifts, travels to the next line and presses again. One
    /// operation, one receipt, every point checked.
    Strokes {
        paths: Vec<Vec<StrokePoint>>,
        button: PointerButton,
    },
    Focus,
    /// Capture what the surface shows: all of it, or one part of it.
    Snapshot {
        region: Option<Region>,
    },
    /// Press one key and keep it down until `KeyUp`. The compositor lets it go by itself
    /// after five seconds without it being pressed again, when a person takes the stage,
    /// when the focus leaves the surface, or when the connection that pressed it closes.
    KeyDown {
        key: Key,
    },
    KeyUp {
        key: Key,
    },
}

/// One point of a stroke, in the surface's own pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrokePoint {
    pub x: i32,
    pub y: i32,
}

/// The part of a surface a capture is about, in the surface's own pixels.
///
/// Looking at the box a control is in, rather than at the whole window, is both faster and
/// less seen: the compositor renders the window either way, but it encodes and hands over
/// only what was asked for, and the agent is given no pixels it did not need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The corner past the bottom-right, which is how the compositor is told the size.
    pub fn far(&self) -> (i32, i32) {
        (
            self.x.saturating_add_unsigned(self.width),
            self.y.saturating_add_unsigned(self.height),
        )
    }

    /// A region with no pixels in it asks for nothing, and is never sent.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The same region kept inside a surface of that size, or nothing left of it.
    pub fn clamped(&self, width: u32, height: u32) -> Option<Self> {
        let x = self.x.max(0);
        let y = self.y.max(0);
        let (far_x, far_y) = self.far();
        let right = far_x.min(i32::try_from(width).unwrap_or(i32::MAX));
        let bottom = far_y.min(i32::try_from(height).unwrap_or(i32::MAX));
        if right <= x || bottom <= y {
            return None;
        }
        Some(Self {
            x,
            y,
            width: (right - x) as u32,
            height: (bottom - y) as u32,
        })
    }

    /// The smallest region holding both.
    pub fn union(&self, other: &Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let (a, b) = self.far();
        let (c, d) = other.far();
        Self {
            x,
            y,
            width: (a.max(c) - x).max(0) as u32,
            height: (b.max(d) - y).max(0) as u32,
        }
    }

    /// The same region with `margin` pixels of context on every side.
    pub fn grown(&self, margin: u32) -> Self {
        Self {
            x: self.x.saturating_sub_unsigned(margin),
            y: self.y.saturating_sub_unsigned(margin),
            width: self.width.saturating_add(margin.saturating_mul(2)),
            height: self.height.saturating_add(margin.saturating_mul(2)),
        }
    }

    /// Whether this region holds all of that one.
    pub fn holds(&self, other: &Self) -> bool {
        let (a, b) = self.far();
        let (c, d) = other.far();
        other.x >= self.x && other.y >= self.y && c <= a && d <= b
    }
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
            // The same authority as one line: a drawing is not a new kind of touch.
            Self::Stroke { .. } | Self::Strokes { .. } => ActionKind::Stroke,
            Self::Focus => ActionKind::Focus,
            Self::Snapshot { .. } => ActionKind::Snapshot,
            Self::KeyDown { .. } | Self::KeyUp { .. } => ActionKind::Hold,
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
            // Every point of a line is a place the pointer is put.
            Self::Stroke { points, .. } => points.iter().map(|at| (at.x, at.y)).collect(),
            Self::Strokes { paths, .. } => paths.iter().flatten().map(|at| (at.x, at.y)).collect(),
            // A capture touches nothing, but a part of a surface is still two corners of
            // it, and they are checked like any other coordinates.
            Self::Snapshot {
                region: Some(region),
            } => vec![(region.x, region.y), region.far()],
            Self::Text { .. }
            | Self::Key { .. }
            | Self::KeyDown { .. }
            | Self::KeyUp { .. }
            | Self::Focus
            | Self::Snapshot { .. } => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_parse_modifiers_then_one_key() {
        assert_eq!(
            KeyChord::parse("ctrl+shift+z"),
            Some(
                KeyChord::key(Key::Char('z'))
                    .with(Modifier::Ctrl)
                    .with(Modifier::Shift)
            )
        );
        assert_eq!(
            KeyChord::parse("enter"),
            Some(KeyChord::key(Key::Named(NamedKey::Enter)))
        );
        assert_eq!(KeyChord::parse("ctrl+"), None, "a chord needs a key");
        assert_eq!(KeyChord::parse("hyper+a"), None);
        assert_eq!(KeyChord::parse("a+ctrl"), None, "the key comes last");
        assert_eq!(KeyChord::parse(""), None);
    }

    #[test]
    fn keys_parse_names_and_single_characters_only() {
        assert_eq!(Key::parse("enter"), Some(Key::Named(NamedKey::Enter)));
        assert_eq!(Key::parse("f12"), Some(Key::Named(NamedKey::F12)));
        assert_eq!(Key::parse("s"), Some(Key::Char('s')));
        assert_eq!(Key::parse("7"), Some(Key::Char('7')));
        assert_eq!(Key::parse("["), Some(Key::Char('[')));
        assert_eq!(Key::parse("."), Some(Key::Char('.')));
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
