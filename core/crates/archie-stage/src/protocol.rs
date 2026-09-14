//! What the compositor tells the control plane, and the client that listens.
//!
//! Events are JSON lines. The compositor is the only authority on which surfaces exist,
//! where they are, who holds the input and whether a command ran, so the control plane
//! keeps a mirror of that — [`StageWorld`] — and prepares proposals against it. It never
//! guesses a geometry revision: a proposal carries the revision of the last `surface`
//! event, and the compositor refuses it if that is no longer current.
//!
//! Every event is bounded and validated before it is believed. A compositor is trusted
//! for what it reports, not for how large its reports are.

use crate::seal::SealedCommand;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// The longest event line accepted, newline included.
pub const MAX_EVENT_BYTES: usize = 16 * 1024;
const MAX_DAMAGE_RECTS: usize = 64;
const MAX_CODE_CHARS: usize = 64;
/// The widest or tallest capture believed.
pub const MAX_SNAPSHOT_SIDE: u32 = 16_384;

/// Who holds the stage's input. Mirrors `archie::visual::AuthorityMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityMode {
    Autonomous,
    Operator,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// One message from the compositor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum StageEvent {
    /// A surface appeared, moved or resized.
    Surface {
        surface: u32,
        app_id: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        geometry_revision: u64,
    },
    Unmapped {
        surface: u32,
    },
    Focus {
        surface: Option<u32>,
    },
    /// Regions of a surface that changed, in surface-local pixels. The cheapest honest
    /// signal that something happened, for perception to look at instead of full frames.
    Damage {
        surface: u32,
        rects: Vec<DamageRect>,
    },
    Authority {
        mode: AuthorityMode,
        reason: String,
    },
    /// What became of a command: `executed`, `cancelled`, or a refusal code. A line the
    /// compositor could not verify is answered with `seq` 0, and only to its sender.
    Receipt {
        seq: u64,
        outcome: String,
        observed_revision: u64,
    },
    /// Where the capture a snapshot command asked for was written. Sent only to the
    /// connection that sent the command, before its receipt.
    Snapshot {
        seq: u64,
        surface: u32,
        width: u32,
        height: u32,
        sha256: String,
        path: String,
    },
}

fn is_code(text: &str) -> bool {
    !text.is_empty()
        && text.chars().count() <= MAX_CODE_CHARS
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl StageEvent {
    fn validate(&self) -> bool {
        match self {
            Self::Surface {
                app_id,
                width,
                height,
                ..
            } => {
                !app_id.is_empty()
                    && app_id.len() <= 255
                    && app_id.bytes().all(|b| b.is_ascii_graphic())
                    && *width > 0
                    && *height > 0
            }
            Self::Unmapped { .. } | Self::Focus { .. } => true,
            Self::Damage { rects, .. } => {
                rects.len() <= MAX_DAMAGE_RECTS && rects.iter().all(|r| r.width > 0 && r.height > 0)
            }
            Self::Authority { reason, .. } => is_code(reason),
            Self::Receipt { outcome, .. } => is_code(outcome),
            Self::Snapshot {
                width,
                height,
                sha256,
                path,
                ..
            } => {
                (1..=MAX_SNAPSHOT_SIDE).contains(width)
                    && (1..=MAX_SNAPSHOT_SIDE).contains(height)
                    && sha256.len() == 64
                    && sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    && !path.is_empty()
                    && path.len() <= 4096
            }
        }
    }
}

/// A surface, as the control plane last heard of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSurface {
    pub surface: u32,
    pub app_id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub geometry_revision: u64,
}

/// The control plane's mirror of the stage.
#[derive(Debug, Clone)]
pub struct StageWorld {
    surfaces: BTreeMap<u32, StageSurface>,
    focus: Option<u32>,
    authority: AuthorityMode,
    last_receipt: Option<(u64, String)>,
}

impl Default for StageWorld {
    fn default() -> Self {
        Self {
            surfaces: BTreeMap::new(),
            focus: None,
            // Until the compositor says otherwise, nobody has handed the agent anything.
            authority: AuthorityMode::Paused,
            last_receipt: None,
        }
    }
}

impl StageWorld {
    pub fn apply(&mut self, event: &StageEvent) {
        match event {
            StageEvent::Surface {
                surface,
                app_id,
                x,
                y,
                width,
                height,
                geometry_revision,
            } => {
                self.surfaces.insert(
                    *surface,
                    StageSurface {
                        surface: *surface,
                        app_id: app_id.clone(),
                        x: *x,
                        y: *y,
                        width: *width,
                        height: *height,
                        geometry_revision: *geometry_revision,
                    },
                );
            }
            StageEvent::Unmapped { surface } => {
                self.surfaces.remove(surface);
                if self.focus == Some(*surface) {
                    self.focus = None;
                }
            }
            StageEvent::Focus { surface } => self.focus = *surface,
            StageEvent::Damage { .. } => {}
            StageEvent::Authority { mode, .. } => self.authority = *mode,
            StageEvent::Receipt { seq, outcome, .. } => {
                self.last_receipt = Some((*seq, outcome.clone()));
            }
            // A capture is for the caller that asked; it changes nothing in the mirror.
            StageEvent::Snapshot { .. } => {}
        }
    }

    pub fn surface(&self, id: u32) -> Option<&StageSurface> {
        self.surfaces.get(&id)
    }

    /// Every surface, in id order.
    pub fn surfaces(&self) -> impl Iterator<Item = &StageSurface> {
        self.surfaces.values()
    }

    /// Surfaces belonging to an application, in id order.
    pub fn surfaces_of<'a>(
        &'a self,
        app_id: &'a str,
    ) -> impl Iterator<Item = &'a StageSurface> + 'a {
        self.surfaces.values().filter(move |s| s.app_id == app_id)
    }

    pub fn focus(&self) -> Option<u32> {
        self.focus
    }

    pub fn authority(&self) -> AuthorityMode {
        self.authority
    }

    /// Whether the agent may send anything right now, as far as the control plane knows.
    /// The compositor checks again; this only avoids sealing commands it would refuse.
    pub fn agent_may_act(&self) -> bool {
        self.authority == AuthorityMode::Autonomous
    }

    pub fn last_receipt(&self) -> Option<(u64, &str)> {
        self.last_receipt
            .as_ref()
            .map(|(seq, outcome)| (*seq, outcome.as_str()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("stage socket error: {0}")]
    Io(#[from] std::io::Error),
    #[error("an event line exceeded {MAX_EVENT_BYTES} bytes; the stream is no longer trusted")]
    LineTooLong,
    #[error("the compositor sent an event this client does not understand")]
    Malformed,
}

/// Talks to the compositor over any byte stream: a Unix socket in production, an in-memory
/// pipe in tests.
pub struct StageClient<R, W> {
    reader: BufReader<R>,
    writer: W,
    poisoned: bool,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> StageClient<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
            poisoned: false,
        }
    }

    pub async fn send(&mut self, command: &SealedCommand) -> Result<(), ClientError> {
        self.writer.write_all(command.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// The next event, or `None` when the compositor closed the stream.
    ///
    /// An oversized line poisons the client: after one, there is no way to know where the
    /// next message starts, so every later call fails too.
    pub async fn next_event(&mut self) -> Result<Option<StageEvent>, ClientError> {
        if self.poisoned {
            return Err(ClientError::LineTooLong);
        }
        let mut line = Vec::new();
        let limit = (MAX_EVENT_BYTES + 1) as u64;
        let read = (&mut self.reader)
            .take(limit)
            .read_until(b'\n', &mut line)
            .await?;
        if read == 0 {
            return Ok(None);
        }
        if line.len() > MAX_EVENT_BYTES || !line.ends_with(b"\n") {
            if line.len() > MAX_EVENT_BYTES {
                self.poisoned = true;
                return Err(ClientError::LineTooLong);
            }
            // The stream ended mid-line.
            return Ok(None);
        }
        let event: StageEvent =
            serde_json::from_slice(&line[..line.len() - 1]).map_err(|_| ClientError::Malformed)?;
        if !event.validate() {
            return Err(ClientError::Malformed);
        }
        Ok(Some(event))
    }
}
