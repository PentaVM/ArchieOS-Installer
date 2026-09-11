//! Time handling.
//!
//! ArchieOS stores every timestamp as milliseconds since the Unix epoch, UTC.
//! Rationale: it is unambiguous, monotonic under comparison, sorts correctly as an
//! BIGINT in PostgreSQL, and survives a round trip through JSON without a formatting
//! dependency. Human-readable RFC 3339 is a *rendering* concern, produced only for
//! logs and the CLI.

use serde::{Deserialize, Serialize};
use std::fmt;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

/// Milliseconds since the Unix epoch, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// Current wall-clock time.
    pub fn now() -> Self {
        Self((OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64)
    }

    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Milliseconds elapsed from `self` to `later`; negative if `later` precedes `self`.
    pub const fn elapsed_to(self, later: Timestamp) -> i64 {
        later.0 - self.0
    }

    pub const fn plus_millis(self, millis: i64) -> Self {
        Self(self.0 + millis)
    }

    /// Render as RFC 3339 for humans. Falls back to the raw epoch value if the
    /// timestamp is outside the representable range rather than panicking.
    pub fn to_rfc3339(self) -> String {
        OffsetDateTime::from_unix_timestamp_nanos((self.0 as i128) * 1_000_000)
            .ok()
            .and_then(|dt| dt.format(&Rfc3339).ok())
            .unwrap_or_else(|| format!("epoch-millis:{}", self.0))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_2020() {
        // 2020-01-01T00:00:00Z in millis. Guards against a clock/units regression.
        assert!(Timestamp::now().as_millis() > 1_577_836_800_000);
    }

    #[test]
    fn renders_rfc3339() {
        assert_eq!(
            Timestamp::from_millis(0).to_rfc3339(),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn elapsed_is_signed() {
        let a = Timestamp::from_millis(1_000);
        let b = Timestamp::from_millis(1_500);
        assert_eq!(a.elapsed_to(b), 500);
        assert_eq!(b.elapsed_to(a), -500);
    }

    #[test]
    fn out_of_range_does_not_panic() {
        assert!(Timestamp::from_millis(i64::MAX)
            .to_rfc3339()
            .starts_with("epoch-millis:"));
    }
}
