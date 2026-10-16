//! Whether a device calls one place on a clock.
//!
//! Malware on a camera or a plug checks in with its controller at a fixed interval, with
//! some jitter; people and browsers do not. This scores the intervals between the
//! connections a device opens to one outside destination the way RITA does: a
//! symmetric spread (Bowley skewness near zero) and a narrow one (median absolute
//! deviation small against the median) together mean a clock.
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
    /// 0..=100: how clock-like the intervals are.
    pub score: u8,
}

/// Notes one opened connection; returns whether it was a new contact.
pub fn note(contacts: &mut VecDeque<u64>, at_ms: u64) -> bool {
    if contacts
        .back()
        .is_some_and(|&last| at_ms.saturating_sub(last) < SAME_CONTACT_MS)
    {
        return false;
    }
    if contacts.len() == KEPT {
        contacts.pop_front();
    }
    contacts.push_back(at_ms);
    true
}

/// The value at quantile `q` of sorted values, by linear interpolation.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let at = q * (sorted.len() - 1) as f64;
    let (low, high) = (at.floor() as usize, at.ceil() as usize);
    sorted[low] + (sorted[high] - sorted[low]) * (at - low as f64)
}

/// The rhythm of these contacts, once there are enough of them.
pub fn rhythm(contacts: &VecDeque<u64>) -> Option<Rhythm> {
    if contacts.len() < MIN_CONTACTS {
        return None;
    }
    let mut intervals: Vec<f64> = contacts
        .iter()
        .zip(contacts.iter().skip(1))
        .map(|(a, b)| b.saturating_sub(*a) as f64)
        .collect();
    intervals.sort_by(f64::total_cmp);
    let (q1, median, q3) = (
        quantile(&intervals, 0.25),
        quantile(&intervals, 0.5),
        quantile(&intervals, 0.75),
    );
    if !(SHORTEST_MS as f64..=LONGEST_MS as f64).contains(&median) {
        return None;
    }
    let skew = if q3 > q1 && median > q1 && median < q3 {
        (q1 + q3 - 2.0 * median) / (q3 - q1)
    } else {
        0.0
    };
    let mut deviations: Vec<f64> = intervals.iter().map(|d| (d - median).abs()).collect();
    deviations.sort_by(f64::total_cmp);
    let mad = quantile(&deviations, 0.5);
    let symmetric = 1.0 - skew.abs();
    let narrow = (1.0 - mad / median).max(0.0);
    Some(Rhythm {
        every_ms: median as u64,
        score: ((symmetric + narrow) / 2.0 * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8,
    })
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

    fn contacts(intervals: impl Iterator<Item = u64>) -> VecDeque<u64> {
        let mut kept = VecDeque::new();
        let mut at = 1_000_000;
        note(&mut kept, at);
        for interval in intervals {
            at += interval;
            note(&mut kept, at);
        }
        kept
    }

    #[test]
    fn a_jittered_clock_scores_high() {
        let mut seed = 7;
        // Every minute, give or take six seconds.
        let kept = contacts((0..40).map(|_| 54_000 + (noise(&mut seed) * 12_000.0) as u64));
        let rhythm = rhythm(&kept).unwrap();
        assert!(rhythm.score >= 90, "{rhythm:?}");
        assert!((54_000..=66_000).contains(&rhythm.every_ms), "{rhythm:?}");
        assert_eq!(kept.len(), KEPT);
    }

    #[test]
    fn a_person_browsing_does_not() {
        let mut seed = 11;
        // Exponential gaps averaging five minutes: no clock.
        for _ in 0..20 {
            let kept = contacts(
                (0..40).map(|_| (-(1.0 - noise(&mut seed)).ln() * 300_000.0) as u64 + 1_000),
            );
            if let Some(rhythm) = rhythm(&kept) {
                assert!(rhythm.score < 70, "{rhythm:?}");
            }
        }
    }

    #[test]
    fn bursts_and_too_few_contacts_are_not_scored() {
        let mut kept = VecDeque::new();
        for at in [1_000, 1_200, 1_900] {
            note(&mut kept, at);
        }
        assert_eq!(kept.len(), 1, "one contact");
        assert_eq!(rhythm(&contacts((0..5).map(|_| 60_000))), None);
        // A poll every two seconds is a stream, not a check-in.
        assert_eq!(rhythm(&contacts((0..40).map(|_| 2_000))), None);
    }
}
