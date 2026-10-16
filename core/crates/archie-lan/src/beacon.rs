//! Whether a device calls one place on a clock.
//!
//! Malware on a camera or a plug checks in with its controller at a fixed interval, with
//! some jitter, and sends about the same each time; people and browsers do neither. This
//! scores the connections a device opens to one outside destination the way RITA does,
//! twice: the intervals between them, and the bytes sent between one and the next. For each,
//! a symmetric spread (Bowley skewness near zero) and a narrow one (median absolute
//! deviation small against the median) together mean a clock. The score is the mean of the
//! two halves, or the intervals alone while the sizes are not known.
//!
//! Plenty of honest devices keep a clock too (NTP, a vendor cloud's keep-alive), so a
//! rhythm alone is recorded, not told; `crate::audit` tells it only for a destination the
//! device started using after it was learned.

use std::collections::VecDeque;

/// Connections opened this close together are one contact (a browser opens several).
pub const SAME_CONTACT_MS: u64 = 1_000;
/// Contacts kept per device and destination: 32 intervals.
pub const KEPT: usize = 33;
/// Contacts before a rhythm is scored.
pub const MIN_CONTACTS: usize = 12;
/// Intervals outside this are not a check-in clock.
const SHORTEST_MS: u64 = 10_000;
const LONGEST_MS: u64 = 24 * 60 * 60 * 1000;

/// A device's rhythm to one destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rhythm {
    /// The median interval between contacts.
    pub every_ms: u64,
    /// 0..=100: how clock-like the contacts are.
    pub score: u8,
    /// The median bytes sent from one contact to the next, when known.
    pub bytes: Option<u64>,
}

/// The contacts of one device with one destination.
#[derive(Debug, Clone, Default)]
pub struct Contacts {
    times: VecDeque<u64>,
    /// Bytes sent between one contact and the next, from the device's running total.
    sizes: VecDeque<u64>,
    /// That total at the last contact.
    total: Option<u64>,
}

impl Contacts {
    /// Notes one opened connection, with the bytes the device has sent to the destination
    /// so far when they are counted; returns whether it was a new contact.
    pub fn note(&mut self, at_ms: u64, total: Option<u64>) -> bool {
        if self
            .times
            .back()
            .is_some_and(|&last| at_ms.saturating_sub(last) < SAME_CONTACT_MS)
        {
            return false;
        }
        if self.times.len() == KEPT {
            self.times.pop_front();
        }
        self.times.push_back(at_ms);
        match (self.total, total) {
            (Some(before), Some(now)) if now >= before => {
                if self.sizes.len() == KEPT - 1 {
                    self.sizes.pop_front();
                }
                self.sizes.push_back(now - before);
            }
            // A total that went missing or backwards (a restart): sizes start again.
            _ => self.sizes.clear(),
        }
        self.total = total;
        true
    }

    /// When the last contact was.
    pub fn last(&self) -> Option<u64> {
        self.times.back().copied()
    }

    /// The rhythm of these contacts, once there are enough of them.
    pub fn rhythm(&self) -> Option<Rhythm> {
        if self.times.len() < MIN_CONTACTS {
            return None;
        }
        let intervals: Vec<f64> = self
            .times
            .iter()
            .zip(self.times.iter().skip(1))
            .map(|(a, b)| b.saturating_sub(*a) as f64)
            .collect();
        let (median, timing) = clockwork(intervals);
        if !(SHORTEST_MS as f64..=LONGEST_MS as f64).contains(&median) {
            return None;
        }
        let sized = (self.sizes.len() + 1 >= MIN_CONTACTS)
            .then(|| clockwork(self.sizes.iter().map(|&s| s as f64).collect()))
            .filter(|(median, _)| *median > 0.0);
        let score = sized.map_or(timing, |(_, sizing)| (timing + sizing) / 2.0);
        Some(Rhythm {
            every_ms: median as u64,
            score: (score * 100.0).round().clamp(0.0, 100.0) as u8,
            bytes: sized.map(|(median, _)| median as u64),
        })
    }
}

/// The value at quantile `q` of sorted values, by linear interpolation.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let at = q * (sorted.len() - 1) as f64;
    let (low, high) = (at.floor() as usize, at.ceil() as usize);
    sorted[low] + (sorted[high] - sorted[low]) * (at - low as f64)
}

/// The median of `values` and how clock-like they are, 0..=1: the mean of a symmetric
/// spread (1 − |Bowley skewness|) and a narrow one (1 − MAD / median).
fn clockwork(mut values: Vec<f64>) -> (f64, f64) {
    values.sort_by(f64::total_cmp);
    let (q1, median, q3) = (
        quantile(&values, 0.25),
        quantile(&values, 0.5),
        quantile(&values, 0.75),
    );
    let skew = if q3 > q1 && median > q1 && median < q3 {
        (q1 + q3 - 2.0 * median) / (q3 - q1)
    } else {
        0.0
    };
    let mut deviations: Vec<f64> = values.iter().map(|v| (v - median).abs()).collect();
    deviations.sort_by(f64::total_cmp);
    let mad = quantile(&deviations, 0.5);
    let symmetric = 1.0 - skew.abs();
    let narrow = if median > 0.0 {
        (1.0 - mad / median).max(0.0)
    } else {
        0.0
    };
    (median, (symmetric + narrow) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so the tests need no crate.
    fn noise(seed: &mut u64) -> f64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed % 1_000_000) as f64 / 1_000_000.0
    }

    /// Contacts at these intervals, each after sending these many bytes (or no count).
    fn contacts(steps: impl Iterator<Item = (u64, Option<u64>)>) -> Contacts {
        let mut kept = Contacts::default();
        let (mut at, mut total) = (1_000_000, Some(0));
        kept.note(at, total);
        for (interval, sent) in steps {
            at += interval;
            total = sent.and_then(|sent| total.map(|t| t + sent));
            kept.note(at, total);
        }
        kept
    }

    #[test]
    fn a_jittered_clock_scores_high() {
        let mut seed = 7;
        // Every minute, give or take six seconds; sizes not counted.
        let kept = contacts((0..40).map(|_| (54_000 + (noise(&mut seed) * 12_000.0) as u64, None)));
        let rhythm = kept.rhythm().unwrap();
        assert!(rhythm.score >= 90, "{rhythm:?}");
        assert!((54_000..=66_000).contains(&rhythm.every_ms), "{rhythm:?}");
        assert_eq!(rhythm.bytes, None);
        assert_eq!(kept.times.len(), KEPT);
    }

    #[test]
    fn the_same_check_in_each_time_keeps_it_high_and_varied_sizes_lower_it() {
        let mut seed = 5;
        // Malware's check-in: every minute, about 420 bytes.
        let fixed = contacts((0..40).map(|_| {
            (
                54_000 + (noise(&mut seed) * 12_000.0) as u64,
                Some(400 + (noise(&mut seed) * 40.0) as u64),
            )
        }));
        let fixed = fixed.rhythm().unwrap();
        // Still above the line `crate::audit` tells at (85).
        assert!(fixed.score >= 85, "{fixed:?}");
        assert!((400..=440).contains(&fixed.bytes.unwrap()), "{fixed:?}");
        // A feed polled every minute whose requests differ a lot in size: not told.
        let varied = contacts((0..40).map(|_| {
            (
                54_000 + (noise(&mut seed) * 12_000.0) as u64,
                Some((-(1.0 - noise(&mut seed)).ln() * 20_000.0) as u64 + 1),
            )
        }));
        let varied = varied.rhythm().unwrap();
        assert!(varied.score < 85, "{varied:?}");
    }

    #[test]
    fn a_person_browsing_does_not() {
        let mut seed = 11;
        // Exponential gaps averaging five minutes: no clock.
        for _ in 0..20 {
            let kept = contacts((0..40).map(|_| {
                (
                    (-(1.0 - noise(&mut seed)).ln() * 300_000.0) as u64 + 1_000,
                    None,
                )
            }));
            if let Some(rhythm) = kept.rhythm() {
                assert!(rhythm.score < 70, "{rhythm:?}");
            }
        }
    }

    #[test]
    fn bursts_and_too_few_contacts_are_not_scored() {
        let mut kept = Contacts::default();
        for at in [1_000, 1_200, 1_900] {
            kept.note(at, None);
        }
        assert_eq!(kept.times.len(), 1, "one contact");
        assert_eq!(contacts((0..5).map(|_| (60_000, None))).rhythm(), None);
        // A poll every two seconds is a stream, not a check-in.
        assert_eq!(contacts((0..40).map(|_| (2_000, None))).rhythm(), None);
    }

    #[test]
    fn a_total_that_goes_backwards_starts_the_sizes_again() {
        let mut kept = Contacts::default();
        kept.note(0, Some(1_000));
        kept.note(60_000, Some(1_400));
        assert_eq!(kept.sizes, [400]);
        kept.note(120_000, Some(100));
        assert!(kept.sizes.is_empty());
    }
}
