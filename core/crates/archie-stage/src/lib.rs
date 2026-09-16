//! # archie-stage
//!
//! The agent's hands on the stage.
//!
//! ArchieOS hosts applications in its own compositor, the Archie Stage, and the agent
//! operates them the way a person would: it moves a pointer, clicks, drags, scrolls and
//! types. This crate is the control plane's side of that: what an action is, which ones
//! policy allows for which application, and how an allowed action reaches the compositor
//! so that nothing else can impersonate it.
//!
//! # The path
//!
//! ```text
//! Cerebra ─ StageProposal ─→ StageGate (app policy + archie-policy Lease)
//!                                  │ AuthorizedStageCommand   (no public constructor)
//!                                  ▼
//!                               Sealer ─ HMAC-SHA256, sequence, expiry
//!                                  │ one JSON line over a local socket
//!                                  ▼
//!                  Archie Stage compositor (display/stage, C++)
//!                    re-checks: authority, surface, app, geometry revision, bounds
//!                                  │ StageEvent lines: surfaces, focus, damage,
//!                                  ▼ authority changes, receipts
//!                               StageClient
//! ```
//!
//! # Rules this crate enforces
//!
//! * **A lease is required for every action.** The gate calls
//!   [`archie_policy::Lease::authorises_one_operation`] against the resource
//!   `stage:<app_id>`. The lease is checked, not consumed; consuming it is the caller's
//!   job once the compositor's receipt confirms the action ran.
//! * **Unknown applications get nothing.** An application missing from the stage policy
//!   is refused, whatever a lease says.
//! * **Text is graded, and never leaves in a trace.** A secret is refused outright:
//!   credentials reach an application through the vault's broker, never through typed
//!   keystrokes an agent composed. Typed text cannot contain control characters, so a
//!   newline — an Enter key in a terminal — is a key action the policy can put behind
//!   approval, not a character smuggled inside a string. `Debug` and receipts show
//!   character counts only.
//! * **Some chords are the system's.** `Super` and `Ctrl+Alt` combinations belong to the
//!   compositor and the kernel (virtual terminals, emergency chords) and are never sent.
//! * **Only the gate's output can be sealed, and only a sealed command runs.** The
//!   compositor verifies the MAC, a strictly increasing sequence and the expiry before it
//!   looks at the command at all.
//!
//! # What this crate does not decide
//!
//! Who holds the stage. Operator takeover, the pause chord and the protected surfaces
//! live in the compositor (`display/visual/include/archie/visual/stage.hpp`), because
//! only the compositor sees physical input — and an agent that could clear a takeover
//! from the control plane would be an agent that cannot be stopped.

#![forbid(unsafe_code)]

pub mod action;
pub mod gate;
pub mod operate;
pub mod policy;
pub mod protocol;
pub mod seal;

pub use action::{
    ActionKind, Key, KeyChord, Modifier, NamedKey, PointerButton, StageAction, StageText,
};
pub use gate::{
    stage_resource, AuthorizedStageCommand, StageAdmission, StageGate, StageProposal, StageReceipt,
    StageRefusal,
};
pub use operate::{
    drain, learn, wait_until, ActOutcome, ActRequest, LookOutcome, OperateError, Operator,
    Snapshot, MAX_CAPTURE_BYTES,
};
pub use policy::{AppPolicy, PolicyError, StagePolicy};
pub use protocol::{
    AuthorityMode, ClientError, DamageRect, Repainted, StageClient, StageEvent, StageSurface,
    StageWorld, MAX_EVENT_BYTES, MAX_SNAPSHOT_SIDE,
};
pub use seal::{
    open, CommandKey, SealError, SealedCommand, Sealer, WireAction, WireCommand,
    MAX_COMMAND_TTL_MS, MAX_ENVELOPE_BYTES, PROTOCOL_VERSION,
};
