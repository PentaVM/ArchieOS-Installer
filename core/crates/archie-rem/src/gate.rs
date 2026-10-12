//! Whether REM may start, and whether it must stop.

use serde::{Deserialize, Serialize};

/// The hours REM may run in, as minutes after local midnight. The end is not included,
/// and a window may cross midnight (22:00 to 05:00).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NightWindow {
    pub start_minute: u16,
    pub end_minute: u16,
}

impl NightWindow {
    /// From `"02:00"` and `"06:00"`.
    pub fn parse(start: &str, end: &str) -> Option<Self> {
        Some(Self {
            start_minute: minute_of(start)?,
            end_minute: minute_of(end)?,
        })
    }

    pub fn contains(&self, minute: u16) -> bool {
        if self.start_minute <= self.end_minute {
            self.start_minute <= minute && minute < self.end_minute
        } else {
            minute >= self.start_minute || minute < self.end_minute
        }
    }
}

/// `"02:30"` as minutes after midnight.
pub fn minute_of(clock: &str) -> Option<u16> {
    let (hours, minutes) = clock.split_once(':')?;
    let (hours, minutes): (u16, u16) = (hours.parse().ok()?, minutes.parse().ok()?);
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

/// What the machine is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conditions {
    /// Minutes after local midnight.
    pub local_minute: u16,
    pub on_mains: bool,
    /// How long since a person last touched the machine. Zero when that is not known:
    /// a node that cannot tell whether someone is there behaves as if someone is.
    pub idle_seconds: u64,
}

/// Why REM is not starting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    OutsideNightWindow,
    OnBattery,
    PersonActive { idle_seconds: u64, needed: u64 },
}

impl std::fmt::Display for Wait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideNightWindow => f.write_str("outside the night window"),
            Self::OnBattery => f.write_str("the machine is on battery"),
            Self::PersonActive {
                idle_seconds,
                needed,
            } => {
                write!(
                    f,
                    "someone used the machine {idle_seconds} s ago (needs {needed} s untouched)"
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    pub window: NightWindow,
    /// How long the machine must have been untouched before REM starts.
    pub idle_seconds: u64,
}

impl Gate {
    /// REM starts only when every condition holds; the first that does not is the reason.
    pub fn may_start(&self, now: &Conditions) -> Result<(), Wait> {
        if !self.window.contains(now.local_minute) {
            return Err(Wait::OutsideNightWindow);
        }
        if !now.on_mains {
            return Err(Wait::OnBattery);
        }
        if now.idle_seconds < self.idle_seconds {
            return Err(Wait::PersonActive {
                idle_seconds: now.idle_seconds,
                needed: self.idle_seconds,
            });
        }
        Ok(())
    }

    /// Once running, REM stops when the person is back or the power is pulled. The end of
    /// the night window does not stop a cycle that is under way: the trainer has its own
    /// wall budget, and stopping at 06:00 sharp would throw away a night's training for a
    /// clock.
    pub fn must_stop(&self, now: &Conditions, idle_when_started: u64) -> bool {
        !now.on_mains || now.idle_seconds < idle_when_started.min(self.idle_seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NIGHT: Gate = Gate {
        window: NightWindow {
            start_minute: 120,
            end_minute: 360,
        },
        idle_seconds: 1800,
    };

    fn at(clock: &str, on_mains: bool, idle_seconds: u64) -> Conditions {
        Conditions {
            local_minute: minute_of(clock).unwrap(),
            on_mains,
            idle_seconds,
        }
    }

    #[test]
    fn rem_starts_at_night_on_mains_with_nobody_there_and_says_why_otherwise() {
        assert_eq!(NIGHT.may_start(&at("03:00", true, 3600)), Ok(()));
        assert_eq!(
            NIGHT.may_start(&at("14:00", true, 3600)),
            Err(Wait::OutsideNightWindow)
        );
        assert_eq!(
            NIGHT.may_start(&at("06:00", true, 3600)),
            Err(Wait::OutsideNightWindow)
        );
        assert_eq!(
            NIGHT.may_start(&at("03:00", false, 3600)),
            Err(Wait::OnBattery)
        );
        assert_eq!(
            NIGHT.may_start(&at("03:00", true, 600)),
            Err(Wait::PersonActive {
                idle_seconds: 600,
                needed: 1800
            })
        );
    }

    #[test]
    fn a_node_that_cannot_tell_whether_someone_is_there_does_not_start() {
        assert!(NIGHT.may_start(&at("03:00", true, 0)).is_err());
    }

    #[test]
    fn a_window_may_cross_midnight() {
        let late = NightWindow::parse("22:00", "05:00").unwrap();
        assert!(late.contains(minute_of("23:30").unwrap()));
        assert!(late.contains(minute_of("04:59").unwrap()));
        assert!(!late.contains(minute_of("05:00").unwrap()));
        assert!(!late.contains(minute_of("12:00").unwrap()));
        assert_eq!(NightWindow::parse("25:00", "05:00"), None);
    }

    #[test]
    fn a_touch_or_a_pulled_plug_stops_it_and_the_clock_does_not() {
        assert!(
            NIGHT.must_stop(&at("03:10", true, 4), 3600),
            "the person is back"
        );
        assert!(
            NIGHT.must_stop(&at("03:10", false, 4200), 3600),
            "on battery now"
        );
        assert!(!NIGHT.must_stop(&at("03:10", true, 4200), 3600));
        assert!(
            !NIGHT.must_stop(&at("06:30", true, 16_000), 3600),
            "past the window, still training"
        );
    }
}
