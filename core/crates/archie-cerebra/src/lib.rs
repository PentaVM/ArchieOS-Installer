//! # archie-cerebra
//!
//! The cognitive loop: the thing that decides whether anything should happen.
//!
//! Every decision this crate sequences already existed as a tested function — the floor
//! in `archie-memory`, the capsule in `archie-vault`, disqualification in
//! `archie-router`, admission in `archie-memory`, the gate in `archie-mcp`. Nothing
//! called them in order. This is that sequencer, and `docs/CEREBRA.md` is the design it
//! implements.
//!
//! # What this crate is not
//!
//! [`Cerebra::consider`] is a deterministic sequencer. It ends by handing
//! back a context capsule and the identifier of the resource that should receive it; the
//! optional [`advisory::AdvisoryInterpreter`] continuation performs bounded inference
//! through `archie-model-runtime`. Tool execution remains in `archie-mcp`. I/O is
//! separate from `consider`, whose ordering tests need no network.
//!
//! That separation is the architecture, not a convenience. Everything can call an LLM.
//! What distinguishes Archie is the sequence of decisions around the call.
//!
//! # Order
//!
//! ```text
//! observe ─→ floor ─→ contextualise ─→ route ─→ …
//!              │
//!              └── most passes end here, and that is correct
//! ```
//!
//! The floor is consulted *before* context is assembled. Deciding whether to speak is
//! cheap and touches nothing private; deciding what to say means decrypting vault
//! entries. In the other order, every pause mid-sentence would have Archie read
//! someone's private notes in order to conclude it should stay quiet. A pass ending in
//! silence leaves no capsule behind, because none was made.
//!
//! # Silence is an outcome
//!
//! [`Intent::StaySilent`] carries a reason and appears in the trace like anything else.
//! A loop whose only expressible result is an action produces an assistant that always
//! acts, and the failure mode of an always-on personal intelligence is not that it does
//! too little.

#![forbid(unsafe_code)]

pub mod advisory;
pub mod clarification;
pub mod observation;
pub mod parameters;
pub mod pass;
pub mod pillars;
pub mod sentry_bridge;
pub mod vault_io;
pub mod wanda_bridge;
pub mod wanda_watcher;

pub use clarification::{ClarificationAssessment, ClarificationDecision, ClarificationSession};
pub use observation::{Observation, Trigger};
pub use pass::{Cerebra, CerebraConfig, CerebraError, Intent, Pass, RememberSummary};
