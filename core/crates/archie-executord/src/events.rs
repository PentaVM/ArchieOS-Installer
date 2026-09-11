//! What happened, kept until someone asks.
//!
//! RFC-007 §7. The executor learns things nobody requested — a worker exited, a task was
//! frozen, reconciliation found a discrepancy — and `archied` needs them to keep its
//! scheduler honest.
//!
//! # Pulled, not pushed
//!
//! A pushed event needs a connection the executor holds open and writes to unprompted,
//! and this executor answers one caller at a time on purpose. A held subscription would
//! block every other command behind it.
//!
//! Pulling also survives the thing that pushing does not: an `archied` that restarts
//! misses nothing, because the events are still queued when it comes back.
//!
//! # A full queue is reported, never quietly emptied
//!
//! The queue is bounded, so an `archied` that stops asking for long enough will lose
//! events. The dangerous version of that is losing them silently: a consumer that
//! believes it has seen everything keeps a stale picture indefinitely, and a task that
//! finished an hour ago still holds a scheduler slot that does not exist.
//!
//! So overflow sets a flag that survives until the next drain, and a consumer that sees
//! it falls back to `Inventory`, which is authoritative. The queue drops the *oldest*
//! events rather than refusing new ones, because the newest are the ones whose subject is
//! still relevant — and either way the flag says the stream has a hole in it.

use std::collections::VecDeque;

use archie_execution::protocol::Event;

/// How many events the executor will hold.
///
/// Sized so a busy node can run for many scheduling passes without overflowing, and small
/// enough that a forgotten queue is not a memory leak wearing a useful name.
pub const MAX_QUEUED_EVENTS: usize = 4096;

/// Events waiting to be collected.
#[derive(Debug)]
pub struct EventQueue {
    events: VecDeque<Event>,
    capacity: usize,
    /// Set when something was dropped, cleared only by a drain that reports it.
    overflowed: bool,
    /// How many were lost in total, for the log line that should follow.
    dropped: u64,
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::with_capacity(MAX_QUEUED_EVENTS)
    }
}

impl EventQueue {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: VecDeque::new(),
            // A zero-capacity queue would drop everything and report overflow forever,
            // which is a configuration mistake rather than a policy. One is the floor.
            capacity: capacity.max(1),
            overflowed: false,
            dropped: 0,
        }
    }

    /// Record something that happened.
    pub fn push(&mut self, event: Event) {
        if self.events.len() >= self.capacity {
            // Drop the oldest. The newest events are the ones whose subject is still
            // relevant — a task that finished thirty seconds ago matters more to the
            // scheduler than one that finished an hour ago — and the flag says the stream
            // is incomplete either way.
            self.events.pop_front();
            self.overflowed = true;
            self.dropped += 1;
        }
        self.events.push_back(event);
    }

    /// Take everything, and say whether anything was lost.
    ///
    /// The flag is cleared here and only here: it must survive until it has actually been
    /// reported, or an overflow that happened between two drains would vanish unnoticed.
    pub fn drain(&mut self) -> (Vec<Event>, bool) {
        let overflowed = self.overflowed;
        self.overflowed = false;
        (self.events.drain(..).collect(), overflowed)
    }

    /// How many are waiting.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Total events lost since the executor started.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Whether anything is waiting that a human should look at rather than a scheduler.
    pub fn needs_attention(&self) -> bool {
        self.events.iter().any(Event::needs_attention)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_execution::lifecycle::ExecutionState;
    use archie_execution::protocol::TaskStatus;
    use archie_types::TaskId;

    fn finished(pid: u32) -> Event {
        Event::Finished(TaskStatus {
            task_id: TaskId::new(),
            state: ExecutionState::Completed,
            pid: Some(pid),
            exit_code: Some(0),
            exit_signal: None,
            failure_reason: None,
        })
    }

    #[test]
    fn events_come_back_in_the_order_they_happened() {
        // A scheduler applying "started" after "finished" would resurrect a dead task.
        let mut queue = EventQueue::default();
        for pid in 1..=3 {
            queue.push(finished(pid));
        }

        let (events, overflowed) = queue.drain();
        assert!(!overflowed);
        let pids: Vec<_> = events
            .iter()
            .map(|e| match e {
                Event::Finished(status) => status.pid.unwrap(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(pids, vec![1, 2, 3]);
        assert!(queue.is_empty());
    }

    #[test]
    fn draining_twice_does_not_return_the_same_event_twice() {
        // An event delivered twice would have the scheduler free a slot it already freed.
        let mut queue = EventQueue::default();
        queue.push(finished(1));

        assert_eq!(queue.drain().0.len(), 1);
        assert_eq!(queue.drain().0.len(), 0);
    }

    #[test]
    fn overflow_drops_the_oldest_and_says_so() {
        // Losing events is survivable. Losing them silently is not: a consumer that
        // believes it has seen everything keeps a stale picture indefinitely.
        let mut queue = EventQueue::with_capacity(3);
        for pid in 1..=5 {
            queue.push(finished(pid));
        }

        let (events, overflowed) = queue.drain();
        assert!(
            overflowed,
            "the consumer must be told the stream has a hole"
        );
        assert_eq!(events.len(), 3);

        let pids: Vec<_> = events
            .iter()
            .map(|e| match e {
                Event::Finished(status) => status.pid.unwrap(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            pids,
            vec![3, 4, 5],
            "the newest survive; a task that finished thirty seconds ago matters more to \
             the scheduler than one that finished an hour ago"
        );
        assert_eq!(queue.dropped(), 2);
    }

    #[test]
    fn the_overflow_flag_survives_until_it_has_been_reported() {
        // An overflow between two drains that nobody was told about is exactly the silent
        // loss the flag exists to prevent.
        let mut queue = EventQueue::with_capacity(1);
        queue.push(finished(1));
        queue.push(finished(2));

        assert!(queue.drain().1, "reported on the drain that follows it");
        assert!(!queue.drain().1, "and cleared only once it has been");
    }

    #[test]
    fn a_quiet_queue_reports_no_overflow() {
        let mut queue = EventQueue::with_capacity(10);
        queue.push(finished(1));
        let (events, overflowed) = queue.drain();
        assert_eq!(events.len(), 1);
        assert!(!overflowed);
        assert_eq!(queue.dropped(), 0);
    }

    #[test]
    fn a_zero_capacity_queue_is_treated_as_a_mistake_not_a_policy() {
        // It would drop everything and report overflow forever, which is a configuration
        // error rather than a decision anyone made.
        let mut queue = EventQueue::with_capacity(0);
        queue.push(finished(1));
        assert_eq!(queue.len(), 1);
        assert!(!queue.drain().1);
    }

    #[test]
    fn a_discrepancy_is_flagged_for_a_human_rather_than_a_scheduler() {
        let mut queue = EventQueue::default();
        queue.push(finished(1));
        assert!(!queue.needs_attention());

        queue.push(Event::Discrepancy {
            task_id: TaskId::new(),
            recorded: ExecutionState::Running,
            detail: "no process in the task cgroup".into(),
        });
        assert!(queue.needs_attention());
    }

    #[test]
    fn the_count_of_lost_events_accumulates_across_drains() {
        // The flag says "there is a hole"; this says how big it has been, which is what
        // tells an operator whether archied is polling often enough.
        let mut queue = EventQueue::with_capacity(1);
        queue.push(finished(1));
        queue.push(finished(2));
        queue.drain();
        assert_eq!(queue.dropped(), 1);

        queue.push(finished(3));
        queue.push(finished(4));
        queue.drain();
        assert_eq!(queue.dropped(), 2);
    }
}
