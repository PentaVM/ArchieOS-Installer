//! S-LINKs: what one model knows, available to the others while they work.
//!
//! An S-LINK is a typed, bounded, versioned channel from one producer to any number of
//! consumers (`_dev/PLAN-CORTEX-ROUTER-SLINKS-2026-10-20.md`, §5). It holds the latest
//! state plus a ring of the recent revisions:
//!
//! - publishing never waits for a reader;
//! - [`link::Reader::latest`] returns the newest complete revision at once;
//! - [`link::Reader::since`] returns the revisions after one the reader holds, or a gap
//!   when the ring moved past it, after which the reader takes the latest state. The
//!   contract is state, so a lost delta corrupts nothing.
//!
//! Between processes ([`shm`], Linux) the payload sits in a sealed-size `memfd` under a
//! seqlock, an `eventfd` wakes readers, and both descriptors travel over a Unix socket.
//!
//! Every message has a [`Header`] with the link's name, revision, when its source observed
//! it, a SHA-256 of the payload and a [`PrivacyGrade`]; payloads refuse unknown fields and
//! oversized content ([`links`]). Screen and camera content is `Restricted`: it does not
//! leave this machine ([`Message::may_reach`]).
//!
//! **An S-LINK carries data, never authority.** A `task.intent` is a proposal; acting on it
//! still goes Cerebra → Policy → lease → Stage. Nothing here grants, schedules or executes.

pub mod join;
pub mod link;
pub mod links;
#[cfg(target_os = "linux")]
pub mod shm;
pub mod text;

pub use archie_types::model::{Locality, PrivacyGrade};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{link}: {reason}")]
    Invalid { link: String, reason: String },
    #[error("{link}: {bytes} bytes, at most {limit}")]
    TooLarge {
        link: String,
        bytes: usize,
        limit: usize,
    },
    #[error("{link}: the payload does not match its digest")]
    Digest { link: String },
    #[error("message: {0}")]
    Format(#[from] serde_json::Error),
    #[error("shared memory: {0}")]
    Shared(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A payload type: one per link, with its limits.
pub trait Payload: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    /// The link's name, e.g. `scene.text`.
    const LINK: &'static str;
    /// The serialized payload's size limit.
    const MAX_BYTES: usize;
    /// The least restrictive grade this content may carry.
    fn grade(&self) -> PrivacyGrade;
    /// Content limits beyond the schema (counts, lengths, finite numbers).
    fn validate(&self) -> Result<(), String>;
}

/// What every message says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub link: String,
    pub revision: u64,
    /// When the source observed what the payload describes (its clock, milliseconds).
    pub observed_at_ms: u64,
    pub grade: PrivacyGrade,
    /// SHA-256 of the payload's JSON, hex.
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message<P> {
    pub header: Header,
    pub payload: P,
}

fn digest_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl<P: Payload> Message<P> {
    /// A validated message: the payload within its limits, digested and graded.
    pub fn new(revision: u64, observed_at_ms: u64, payload: P) -> Result<Self, Error> {
        payload.validate().map_err(|reason| Error::Invalid {
            link: P::LINK.into(),
            reason,
        })?;
        let bytes = serde_json::to_vec(&payload)?;
        if bytes.len() > P::MAX_BYTES {
            return Err(Error::TooLarge {
                link: P::LINK.into(),
                bytes: bytes.len(),
                limit: P::MAX_BYTES,
            });
        }
        Ok(Self {
            header: Header {
                link: P::LINK.into(),
                revision,
                observed_at_ms,
                grade: payload.grade(),
                digest: digest_of(&bytes),
            },
            payload,
        })
    }

    /// Checks a message that came from elsewhere: the link, the limits, the digest, and
    /// that its grade is not looser than its content allows.
    pub fn verify(&self) -> Result<(), Error> {
        let link = P::LINK.to_owned();
        if self.header.link != link {
            return Err(Error::Invalid {
                link,
                reason: format!("message for {}", self.header.link),
            });
        }
        self.payload.validate().map_err(|reason| Error::Invalid {
            link: link.clone(),
            reason,
        })?;
        let bytes = serde_json::to_vec(&self.payload)?;
        if bytes.len() > P::MAX_BYTES {
            return Err(Error::TooLarge {
                link,
                bytes: bytes.len(),
                limit: P::MAX_BYTES,
            });
        }
        if digest_of(&bytes) != self.header.digest {
            return Err(Error::Digest { link });
        }
        if self.header.grade < self.payload.grade() {
            return Err(Error::Invalid {
                link,
                reason: "graded looser than its content".into(),
            });
        }
        Ok(())
    }

    /// Whether this message may be sent to a consumer at `locality`.
    pub fn may_reach(&self, locality: Locality) -> bool {
        self.header.grade.permits(locality)
    }

    /// Reads a message from JSON bytes, refusing anything that does not verify.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        let message: Self = serde_json::from_slice(bytes)?;
        message.verify()?;
        Ok(message)
    }
}
