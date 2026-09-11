//! Metrics.
//!
//! A small counter/gauge registry with no external dependency. v0.1 exposes metrics
//! through the control socket; a Prometheus exporter can be layered on later without
//! changing how call sites record values.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// A point-in-time view of every recorded metric.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricSnapshot {
    /// Monotonically increasing totals.
    pub counters: BTreeMap<String, u64>,
    /// Values that can go up and down.
    pub gauges: BTreeMap<String, i64>,
}

impl MetricSnapshot {
    pub fn counter(&self, name: &str) -> u64 {
        self.counters.get(name).copied().unwrap_or(0)
    }

    pub fn gauge(&self, name: &str) -> i64 {
        self.gauges.get(name).copied().unwrap_or(0)
    }
}

/// A cheap, cloneable metrics registry.
///
/// Cloning shares the same underlying storage, so the registry can be handed to every
/// subsystem without threading a reference through every signature.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    counters: Arc<RwLock<BTreeMap<String, Arc<AtomicU64>>>>,
    gauges: Arc<RwLock<BTreeMap<String, Arc<AtomicI64>>>>,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `value` to a counter, creating it at zero if needed.
    pub fn increment(&self, name: &str, value: u64) {
        if let Some(counter) = self.counters.read().ok().and_then(|m| m.get(name).cloned()) {
            counter.fetch_add(value, Ordering::Relaxed);
            return;
        }
        // Slow path: take the write lock and re-check, since another thread may have
        // created the counter between our read and write.
        if let Ok(mut map) = self.counters.write() {
            map.entry(name.to_string())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .fetch_add(value, Ordering::Relaxed);
        }
    }

    pub fn count(&self, name: &str) {
        self.increment(name, 1);
    }

    /// Set a gauge to an absolute value.
    pub fn set_gauge(&self, name: &str, value: i64) {
        if let Some(gauge) = self.gauges.read().ok().and_then(|m| m.get(name).cloned()) {
            gauge.store(value, Ordering::Relaxed);
            return;
        }
        if let Ok(mut map) = self.gauges.write() {
            map.entry(name.to_string())
                .or_insert_with(|| Arc::new(AtomicI64::new(0)))
                .store(value, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> MetricSnapshot {
        let counters = self
            .counters
            .read()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.load(Ordering::Relaxed)))
                    .collect()
            })
            .unwrap_or_default();
        let gauges = self
            .gauges
            .read()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.load(Ordering::Relaxed)))
                    .collect()
            })
            .unwrap_or_default();
        MetricSnapshot { counters, gauges }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate() {
        let m = Metrics::new();
        m.count("boot.attempts");
        m.count("boot.attempts");
        m.increment("boot.attempts", 3);
        assert_eq!(m.snapshot().counter("boot.attempts"), 5);
    }

    #[test]
    fn unknown_metrics_read_as_zero() {
        let snapshot = Metrics::new().snapshot();
        assert_eq!(snapshot.counter("nope"), 0);
        assert_eq!(snapshot.gauge("nope"), 0);
    }

    #[test]
    fn gauges_move_in_both_directions() {
        let m = Metrics::new();
        m.set_gauge("fabric.peers", 3);
        assert_eq!(m.snapshot().gauge("fabric.peers"), 3);
        m.set_gauge("fabric.peers", 1);
        assert_eq!(m.snapshot().gauge("fabric.peers"), 1);
        m.set_gauge("fabric.peers", -1);
        assert_eq!(m.snapshot().gauge("fabric.peers"), -1);
    }

    #[test]
    fn clones_share_storage() {
        let a = Metrics::new();
        let b = a.clone();
        a.count("shared");
        b.count("shared");
        assert_eq!(a.snapshot().counter("shared"), 2);
    }

    #[test]
    fn concurrent_increments_do_not_lose_counts() {
        let m = Metrics::new();
        let mut handles = Vec::new();
        for _ in 0..8 {
            let m = m.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..1_000 {
                    m.count("hot");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(m.snapshot().counter("hot"), 8_000);
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let m = Metrics::new();
        m.count("a");
        m.set_gauge("b", 7);
        let snapshot = m.snapshot();
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_str::<MetricSnapshot>(&json).unwrap(),
            snapshot
        );
    }
}
