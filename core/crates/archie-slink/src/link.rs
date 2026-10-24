//! A link inside one process: the latest message and a ring of recent ones.
//!
//! The producer swaps an `Arc` and pushes it on the ring; readers clone an `Arc`. Both hold
//! a lock for the length of a pointer copy, so publishing does not wait on a slow reader
//! (a reader never holds the lock while it works on a message).

use crate::{Error, Message, Payload};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

struct State<P> {
    revision: u64,
    latest: Option<Arc<Message<P>>>,
    ring: VecDeque<Arc<Message<P>>>,
}

struct Shared<P> {
    capacity: usize,
    state: Mutex<State<P>>,
}

/// The link itself; hands out a publisher and readers.
pub struct Link<P> {
    shared: Arc<Shared<P>>,
}

impl<P> Clone for Link<P> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

/// What a reader gets after a revision it holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Since<P> {
    /// Every revision after the one asked about, oldest first (empty: nothing new).
    Deltas(Vec<Arc<Message<P>>>),
    /// The ring moved past it: some revisions are gone. Take the latest state.
    Gap { latest: Arc<Message<P>> },
}

impl<P: Payload> Link<P> {
    /// A link keeping the last `ring` revisions for readers that fall behind.
    pub fn new(ring: usize) -> Self {
        Self {
            shared: Arc::new(Shared {
                capacity: ring.max(1),
                state: Mutex::new(State {
                    revision: 0,
                    latest: None,
                    ring: VecDeque::new(),
                }),
            }),
        }
    }

    /// Validates `payload`, numbers it and makes it the latest. Returns its revision.
    pub fn publish(&self, observed_at_ms: u64, payload: P) -> Result<u64, Error> {
        // Validation and digesting happen before the lock.
        let next = self.peek_revision() + 1;
        let message = Arc::new(Message::new(next, observed_at_ms, payload)?);
        let mut state = self.lock()?;
        // Another publisher may have gone first: renumber rather than reorder.
        let message = if state.revision + 1 == next {
            message
        } else {
            let mut renumbered = (*message).clone();
            renumbered.header.revision = state.revision + 1;
            Arc::new(renumbered)
        };
        state.revision = message.header.revision;
        state.latest = Some(message.clone());
        state.ring.push_back(message);
        while state.ring.len() > self.shared.capacity {
            state.ring.pop_front();
        }
        Ok(state.revision)
    }

    fn peek_revision(&self) -> u64 {
        self.shared.state.lock().map_or(0, |s| s.revision)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State<P>>, Error> {
        self.shared.state.lock().map_err(|_| Error::Invalid {
            link: P::LINK.into(),
            reason: "a publisher panicked".into(),
        })
    }

    /// The newest complete revision, without waiting.
    pub fn latest(&self) -> Option<Arc<Message<P>>> {
        self.lock().ok()?.latest.clone()
    }

    pub fn revision(&self) -> u64 {
        self.peek_revision()
    }

    /// The revisions after `after`, or a gap when the ring no longer holds them all.
    pub fn since(&self, after: u64) -> Since<P> {
        let Ok(state) = self.lock() else {
            return Since::Deltas(Vec::new());
        };
        if after >= state.revision {
            return Since::Deltas(Vec::new());
        }
        let oldest = state.ring.front().map_or(u64::MAX, |m| m.header.revision);
        match &state.latest {
            Some(latest) if after + 1 < oldest => Since::Gap {
                latest: latest.clone(),
            },
            _ => Since::Deltas(
                state
                    .ring
                    .iter()
                    .filter(|m| m.header.revision > after)
                    .cloned()
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::SocialFocus;

    fn focus(who: &str) -> SocialFocus {
        serde_json::from_value(serde_json::json!({
            "people": [{"who": who, "strength": 0.8}],
            "reason": "asked about the meeting"
        }))
        .unwrap()
    }

    #[test]
    fn readers_get_the_latest_and_the_deltas_they_missed() {
        let link: Link<SocialFocus> = Link::new(3);
        assert!(link.latest().is_none());
        assert_eq!(link.since(0), Since::Deltas(Vec::new()));
        for (i, who) in ["ana", "beto", "carla"].iter().enumerate() {
            assert_eq!(
                link.publish(i as u64 * 10, focus(who)).unwrap(),
                i as u64 + 1
            );
        }
        assert_eq!(link.latest().unwrap().payload.people[0].who, "carla");
        let Since::Deltas(missed) = link.since(1) else {
            panic!("the ring still holds revision 2")
        };
        let revisions: Vec<u64> = missed.iter().map(|m| m.header.revision).collect();
        assert_eq!(revisions, [2, 3]);
        assert_eq!(link.since(3), Since::Deltas(Vec::new()));
    }

    #[test]
    fn a_reader_behind_the_ring_gets_a_gap_and_the_state() {
        let link: Link<SocialFocus> = Link::new(2);
        for who in ["a", "b", "c", "d"] {
            link.publish(0, focus(who)).unwrap();
        }
        match link.since(1) {
            Since::Gap { latest } => {
                assert_eq!(latest.header.revision, 4);
                assert_eq!(latest.payload.people[0].who, "d");
            }
            other => panic!("expected a gap, got {other:?}"),
        }
        let Since::Deltas(recent) = link.since(2) else {
            panic!("revisions 3 and 4 are in the ring")
        };
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn publishers_on_threads_never_repeat_a_revision() {
        let link: Link<SocialFocus> = Link::new(1000);
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let link = link.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        link.publish(i, focus(&format!("p{t}"))).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let Since::Deltas(all) = link.since(0) else {
            panic!("the ring holds every revision")
        };
        let revisions: Vec<u64> = all.iter().map(|m| m.header.revision).collect();
        assert_eq!(revisions, (1..=200).collect::<Vec<u64>>());
    }

    #[test]
    fn invalid_payloads_are_never_published() {
        let link: Link<SocialFocus> = Link::new(4);
        let mut bad = focus("x");
        bad.people[0].strength = f32::NAN;
        assert!(link.publish(0, bad).is_err());
        assert_eq!(link.revision(), 0);
    }
}
