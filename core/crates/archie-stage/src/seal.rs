//! Sealing an authorised command for the compositor, and opening one.
//!
//! The compositor listens on a local socket, and filesystem permissions are not the only
//! thing standing between "a process on this machine" and "a process that may click on
//! the operator's applications". Every command is one JSON line:
//!
//! ```text
//! {"v":1,"payload":"<command JSON>","mac":"<64 hex>"}
//! ```
//!
//! where `mac` is HMAC-SHA256 over `archie-stage/1\0` followed by the exact payload bytes.
//! The key is provisioned to the control plane and the compositor only. The compositor
//! verifies the MAC before parsing the payload, then requires a strictly increasing
//! sequence and an unexpired, short-lived command, so a captured line can neither be
//! altered nor replayed.
//!
//! [`open`] is the verifier. The compositor implements the same checks in C++; this one
//! exists so the format has a second, tested reader, and for test compositors.

use crate::action::{
    Key, KeyChord, Modifier, PointerButton, Region, StageAction, StageText, StrokePoint,
    MAX_COORDINATE, MAX_SCROLL_STEPS, MAX_STROKE_PATHS, MAX_STROKE_POINTS,
    MAX_FRAME_WIDTH, MAX_STROKE_POINTS_IN_ALL, MAX_TEXT_CHARS, MIN_FRAME_WIDTH,
};
use crate::gate::AuthorizedStageCommand;
use archie_types::Timestamp;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroizing;

pub const PROTOCOL_VERSION: u16 = 1;
/// The longest command line accepted, newline included.
pub const MAX_ENVELOPE_BYTES: usize = 32 * 1024;
/// The longest a command may live. A command is for now, not for a queue.
pub const MAX_COMMAND_TTL_MS: i64 = 10_000;
/// How far a command's issue time may be ahead of the verifier's clock.
const CLOCK_SKEW_MS: i64 = 2_000;
const DOMAIN: &[u8] = b"archie-stage/1\0";

/// The key shared by the control plane and the compositor.
pub struct CommandKey {
    key: hmac::Key,
}

impl CommandKey {
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, bytes),
        }
    }

    /// A fresh key, and its bytes for provisioning to the compositor.
    pub fn generate() -> Result<(Self, Zeroizing<[u8; 32]>), SealError> {
        let mut bytes = Zeroizing::new([0_u8; 32]);
        SystemRandom::new()
            .fill(bytes.as_mut())
            .map_err(|_| SealError::Randomness)?;
        Ok((Self::from_bytes(&bytes), bytes))
    }

    fn tag(&self, payload: &[u8]) -> hmac::Tag {
        let mut context = hmac::Context::with_key(&self.key);
        context.update(DOMAIN);
        context.update(payload);
        context.sign()
    }
}

impl fmt::Debug for CommandKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CommandKey(redacted)")
    }
}

/// An action as it travels to the compositor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WireAction {
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
        modifiers: Vec<Modifier>,
        key: Key,
    },
    Stroke {
        points: Vec<StrokePoint>,
        button: PointerButton,
    },
    Strokes {
        paths: Vec<Vec<StrokePoint>>,
        button: PointerButton,
    },
    Focus,
    Snapshot {
        /// Absent asks for the whole surface, which is what the compositor understood
        /// before there was a choice.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<Region>,
    },
    Frame {
        max_width: u32,
    },
    KeyDown {
        key: Key,
    },
    KeyUp {
        key: Key,
    },
    ButtonDown {
        button: PointerButton,
    },
    ButtonUp {
        button: PointerButton,
    },
}

impl From<&StageAction> for WireAction {
    fn from(action: &StageAction) -> Self {
        match action {
            StageAction::Move { x, y } => Self::Move { x: *x, y: *y },
            StageAction::Click { x, y, button } => Self::Click {
                x: *x,
                y: *y,
                button: *button,
            },
            StageAction::DoubleClick { x, y, button } => Self::DoubleClick {
                x: *x,
                y: *y,
                button: *button,
            },
            StageAction::Drag {
                x,
                y,
                to_x,
                to_y,
                button,
            } => Self::Drag {
                x: *x,
                y: *y,
                to_x: *to_x,
                to_y: *to_y,
                button: *button,
            },
            StageAction::Scroll {
                x,
                y,
                steps_x,
                steps_y,
            } => Self::Scroll {
                x: *x,
                y: *y,
                steps_x: *steps_x,
                steps_y: *steps_y,
            },
            StageAction::Text { text } => Self::Text { text: text.clone() },
            StageAction::Key { chord } => Self::Key {
                modifiers: chord.modifiers.iter().copied().collect(),
                key: chord.key,
            },
            StageAction::Focus => Self::Focus,
            StageAction::Stroke { points, button } => Self::Stroke {
                points: points.clone(),
                button: *button,
            },
            StageAction::Strokes { paths, button } => Self::Strokes {
                paths: paths.clone(),
                button: *button,
            },
            StageAction::Snapshot { region } => Self::Snapshot {
                region: region.filter(|region| !region.is_empty()),
            },
            StageAction::Frame { max_width } => Self::Frame { max_width: *max_width },
            StageAction::KeyDown { key } => Self::KeyDown { key: *key },
            StageAction::KeyUp { key } => Self::KeyUp { key: *key },
            StageAction::ButtonDown { button } => Self::ButtonDown { button: *button },
            StageAction::ButtonUp { button } => Self::ButtonUp { button: *button },
        }
    }
}

impl WireAction {
    fn well_formed(&self) -> bool {
        let point =
            |x: &i32, y: &i32| (0..=MAX_COORDINATE).contains(x) && (0..=MAX_COORDINATE).contains(y);
        match self {
            Self::Move { x, y } | Self::Click { x, y, .. } | Self::DoubleClick { x, y, .. } => {
                point(x, y)
            }
            Self::Drag {
                x, y, to_x, to_y, ..
            } => point(x, y) && point(to_x, to_y),
            Self::Scroll {
                x,
                y,
                steps_x,
                steps_y,
            } => {
                point(x, y)
                    && !(*steps_x == 0 && *steps_y == 0)
                    && steps_x.abs() <= MAX_SCROLL_STEPS
                    && steps_y.abs() <= MAX_SCROLL_STEPS
            }
            Self::Text { text } => (1..=MAX_TEXT_CHARS).contains(&text.chars()) && text.is_plain(),
            Self::Key { modifiers, key } => {
                let chord = KeyChord {
                    modifiers: modifiers.iter().copied().collect(),
                    key: *key,
                };
                chord.modifiers.len() == modifiers.len() && !chord.is_reserved()
            }
            Self::Stroke { points, .. } => {
                (2..=MAX_STROKE_POINTS).contains(&points.len())
                    && points.iter().all(|at| point(&at.x, &at.y))
                    && points.iter().any(|at| at != &points[0])
            }
            Self::Strokes { paths, .. } => {
                (1..=MAX_STROKE_PATHS).contains(&paths.len())
                    && paths.iter().map(Vec::len).sum::<usize>() <= MAX_STROKE_POINTS_IN_ALL
                    && paths.iter().all(|path| {
                        (2..=MAX_STROKE_POINTS).contains(&path.len())
                            && path.iter().all(|at| point(&at.x, &at.y))
                            && path.iter().any(|at| at != &path[0])
                    })
            }
            Self::Focus => true,
            // One key; a key alone is never one of the system's chords.
            Self::KeyDown { .. } | Self::KeyUp { .. } | Self::ButtonDown { .. } | Self::ButtonUp { .. } => true,
            // A part of a window is two corners of it, and it has to be a real rectangle
            // before it is sealed: the compositor checks it again against the live window.
            Self::Snapshot { region: None } => true,
            Self::Frame { max_width } => (MIN_FRAME_WIDTH..=MAX_FRAME_WIDTH).contains(max_width),
            Self::Snapshot {
                region: Some(region),
            } => {
                let (far_x, far_y) = region.far();
                !region.is_empty() && point(&region.x, &region.y) && point(&far_x, &far_y)
            }
        }
    }
}

/// A command as the compositor receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireCommand {
    pub v: u16,
    pub seq: u64,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub operation_id: u64,
    pub surface: u32,
    pub app_id: String,
    pub geometry_revision: u64,
    pub action: WireAction,
    pub trace_id: String,
}

#[derive(Serialize)]
struct EnvelopeOut<'a> {
    v: u16,
    payload: &'a str,
    mac: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeIn {
    v: u16,
    payload: String,
    mac: String,
}

/// One sealed command line, newline included. Never printed.
pub struct SealedCommand {
    sequence: u64,
    expires_at_ms: i64,
    line: Zeroizing<Vec<u8>>,
}

impl SealedCommand {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.line
    }
}

impl fmt::Debug for SealedCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SealedCommand(seq {}, {} bytes, redacted)",
            self.sequence,
            self.line.len()
        )
    }
}

/// Seals authorised commands in sequence.
#[derive(Debug)]
pub struct Sealer {
    key: CommandKey,
    next_sequence: u64,
}

impl Sealer {
    /// Start sealing at `first_sequence`, which must be above anything this compositor has
    /// already accepted. Zero is never a valid sequence.
    pub fn new(key: CommandKey, first_sequence: u64) -> Self {
        Self {
            key,
            next_sequence: first_sequence.max(1),
        }
    }

    /// Never seal below `sequence` again. Several control-plane processes can share one
    /// compositor, and its sequence check is global: a sealer that keeps up with the wall
    /// clock cannot fall behind another process's counter and have everything refused as
    /// a replay.
    pub fn advance_to(&mut self, sequence: u64) {
        self.next_sequence = self.next_sequence.max(sequence);
    }

    pub fn seal(
        &mut self,
        command: &AuthorizedStageCommand,
        now: Timestamp,
        ttl_ms: i64,
    ) -> Result<SealedCommand, SealError> {
        if ttl_ms <= 0 || ttl_ms > MAX_COMMAND_TTL_MS {
            return Err(SealError::Ttl);
        }
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(SealError::SequenceExhausted)?;

        let wire = WireCommand {
            v: PROTOCOL_VERSION,
            seq: sequence,
            issued_at_ms: now.as_millis(),
            expires_at_ms: now.as_millis() + ttl_ms,
            operation_id: command.operation_id(),
            surface: command.surface(),
            app_id: command.app_id().to_string(),
            geometry_revision: command.geometry_revision(),
            action: WireAction::from(command.action()),
            trace_id: command.trace_id().to_string(),
        };
        let payload =
            Zeroizing::new(serde_json::to_string(&wire).map_err(|_| SealError::Encoding)?);
        let mac = hex::encode(self.key.tag(payload.as_bytes()).as_ref());
        let mut line = Zeroizing::new(
            serde_json::to_vec(&EnvelopeOut {
                v: PROTOCOL_VERSION,
                payload: &payload,
                mac,
            })
            .map_err(|_| SealError::Encoding)?,
        );
        line.push(b'\n');
        if line.len() > MAX_ENVELOPE_BYTES {
            return Err(SealError::TooLarge);
        }
        Ok(SealedCommand {
            sequence,
            expires_at_ms: wire.expires_at_ms,
            line,
        })
    }
}

/// Verify and parse one command line, the way the compositor does.
pub fn open(
    key: &CommandKey,
    line: &[u8],
    last_sequence: u64,
    now: Timestamp,
) -> Result<WireCommand, SealError> {
    if line.len() > MAX_ENVELOPE_BYTES {
        return Err(SealError::TooLarge);
    }
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let envelope: EnvelopeIn = serde_json::from_slice(line).map_err(|_| SealError::Malformed)?;
    if envelope.v != PROTOCOL_VERSION {
        return Err(SealError::Version);
    }
    let mac = hex::decode(&envelope.mac).map_err(|_| SealError::Malformed)?;
    let mut message = Zeroizing::new(Vec::with_capacity(DOMAIN.len() + envelope.payload.len()));
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(envelope.payload.as_bytes());
    // Before the payload is parsed at all: an unauthenticated payload is not input.
    hmac::verify(&key.key, &message, &mac).map_err(|_| SealError::BadMac)?;

    let wire: WireCommand =
        serde_json::from_str(&envelope.payload).map_err(|_| SealError::Malformed)?;
    if wire.v != PROTOCOL_VERSION {
        return Err(SealError::Version);
    }
    if wire.seq <= last_sequence {
        return Err(SealError::Replayed);
    }
    if wire.expires_at_ms <= wire.issued_at_ms
        || wire.expires_at_ms - wire.issued_at_ms > MAX_COMMAND_TTL_MS
    {
        return Err(SealError::Ttl);
    }
    if now.as_millis() >= wire.expires_at_ms {
        return Err(SealError::Expired);
    }
    if wire.issued_at_ms > now.as_millis() + CLOCK_SKEW_MS {
        return Err(SealError::FromTheFuture);
    }
    if wire.app_id.is_empty()
        || wire.app_id.len() > 255
        || wire.trace_id.len() > 128
        || !wire.action.well_formed()
    {
        return Err(SealError::Malformed);
    }
    Ok(wire)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    #[error("the system random source failed")]
    Randomness,
    #[error("a command's lifetime must be 1..={MAX_COMMAND_TTL_MS} ms")]
    Ttl,
    #[error("the sequence space is exhausted; provision a new key")]
    SequenceExhausted,
    #[error("the command could not be encoded")]
    Encoding,
    #[error("the command line is over {MAX_ENVELOPE_BYTES} bytes")]
    TooLarge,
    #[error("the command line is not a well-formed stage command")]
    Malformed,
    #[error("the command speaks another protocol version")]
    Version,
    #[error("the command's MAC does not verify")]
    BadMac,
    #[error("the command's sequence is not above the last one accepted")]
    Replayed,
    #[error("the command has expired")]
    Expired,
    #[error("the command claims to have been issued in the future")]
    FromTheFuture,
}

impl SealError {
    /// A stable code for receipts and traces.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Randomness => "stage_key_randomness",
            Self::Ttl => "command_ttl_invalid",
            Self::SequenceExhausted => "command_sequence_exhausted",
            Self::Encoding => "command_encoding",
            Self::TooLarge => "command_too_large",
            Self::Malformed => "command_malformed",
            Self::Version => "command_version",
            Self::BadMac => "command_mac_invalid",
            Self::Replayed => "command_replayed",
            Self::Expired => "command_expired",
            Self::FromTheFuture => "command_from_the_future",
        }
    }
}
