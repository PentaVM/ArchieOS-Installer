//! Independence: deciding how many sources actually said a thing.
//!
//! This is the load-bearing question of the whole world model, and it is easy to get
//! wrong in a way that looks like success. Twelve outlets carrying one wire story
//! produce twelve records. Counting those as twelve confirmations is how a single
//! agency report acquires the authority of a consensus — and a globe that renders it
//! that way is worse than one showing nothing, because it is confidently wrong.
//!
//! Two collapses happen here, and they are different:
//!
//! * **Republication.** Records whose provenance chains reach the same primary source
//!   are one source, however many hops they took.
//! * **Duplication.** Records from the same source describing the same thing are one
//!   record, however many times the feed repeated it.
//!
//! What survives both is the number that may be called corroboration.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Where a record came from, and what it came through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    /// The immediate upstream Archie fetched from.
    pub provider_id: String,
    /// Sources behind that one, **nearest first**, so the last element is the primary.
    ///
    /// An aggregator that republishes a national agency lists the agency here. Without
    /// this chain there is no way to tell a primary source from a repeater, and the
    /// whole independence calculation collapses into counting URLs.
    #[serde(default)]
    pub upstream: Vec<String>,
}

impl SourceRef {
    pub fn direct(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            upstream: Vec::new(),
        }
    }

    pub fn via(
        provider_id: impl Into<String>,
        upstream: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            upstream: upstream.into_iter().map(str::to_string).collect(),
        }
    }

    /// The primary source: the furthest point the provenance chain reaches.
    ///
    /// This, not `provider_id`, is the unit of independence.
    pub fn root(&self) -> &str {
        self.upstream
            .last()
            .map(String::as_str)
            .unwrap_or(&self.provider_id)
    }

    /// Whether two records ultimately trace to the same origin.
    pub fn shares_origin_with(&self, other: &SourceRef) -> bool {
        self.root() == other.root()
    }
}

/// A record, reduced to what independence counting needs.
///
/// `PartialEq` but not `Eq`: the location carries floats, and float equality is not an
/// equivalence relation. Records are compared by id and content signature, never by
/// coordinate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationRef {
    pub id: String,
    pub source: SourceRef,
    /// Milliseconds since the Unix epoch.
    pub observed_at: i64,
    /// A normalized fingerprint of what the record says.
    ///
    /// Two records with the same signature are asserting the same thing. Producing a
    /// good signature is the normalizer's job; this module only compares them.
    pub content_signature: String,
    pub location: Option<GeoPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
}

impl GeoPoint {
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    /// Great-circle distance in kilometres.
    ///
    /// Haversine rather than a planar approximation: events cluster near coastlines and
    /// borders, and a flat-earth metric is wrong by tens of kilometres at high latitudes
    /// — exactly where a naive implementation would merge two distinct events.
    pub fn distance_km(&self, other: &GeoPoint) -> f64 {
        const EARTH_RADIUS_KM: f64 = 6371.0088;
        let (lat1, lat2) = (self.latitude.to_radians(), other.latitude.to_radians());
        let delta_lat = (other.latitude - self.latitude).to_radians();
        let delta_lon = (other.longitude - self.longitude).to_radians();

        let a = (delta_lat / 2.0).sin().powi(2)
            + lat1.cos() * lat2.cos() * (delta_lon / 2.0).sin().powi(2);
        // atan2 rather than asin: numerically stable for antipodal points.
        2.0 * a.sqrt().atan2((1.0 - a).max(0.0).sqrt()) * EARTH_RADIUS_KM
    }
}

/// The result of counting independence over a set of records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndependenceReport {
    /// Distinct primary sources, sorted. This is the corroboration count.
    pub independent_sources: Vec<String>,
    /// Records kept after collapsing duplicates, in input order.
    pub representative_ids: Vec<String>,
    /// Records dropped as republication, mapped to the record they collapsed into.
    pub collapsed: BTreeMap<String, String>,
    /// Total records considered, before any collapse.
    pub considered: usize,
}

impl IndependenceReport {
    /// The number that may be called corroboration.
    pub fn independent_count(&self) -> usize {
        self.independent_sources.len()
    }

    /// Whether this meets the schema's bar for an event to claim `corroborated` state.
    pub fn is_corroborated(&self) -> bool {
        self.independent_count() >= 2
    }

    /// How many records were dropped as repetition.
    pub fn collapsed_count(&self) -> usize {
        self.collapsed.len()
    }

    /// A one-line explanation for the display.
    ///
    /// Deliberately states both numbers. "12 reports" and "12 reports from 1 source" are
    /// very different claims, and the second is the honest one.
    pub fn explain(&self) -> String {
        let sources = self.independent_count();
        let plural = if sources == 1 { "source" } else { "sources" };
        if self.collapsed.is_empty() {
            format!(
                "{} report(s) from {sources} independent {plural}",
                self.considered
            )
        } else {
            format!(
                "{} report(s) from {sources} independent {plural} ({} collapsed as republication)",
                self.considered,
                self.collapsed.len()
            )
        }
    }
}

/// Count independent sources, collapsing republication and repetition.
///
/// Records are kept in input order, and the first record reaching a given
/// (primary source, content) pair is the representative. Determinism matters: the same
/// input must always yield the same representative, or an event's citation changes
/// between refreshes for no reason a reader can see.
pub fn assess_independence(observations: &[ObservationRef]) -> IndependenceReport {
    let mut seen: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut representative_ids = Vec::new();
    let mut collapsed = BTreeMap::new();
    let mut roots: BTreeSet<String> = BTreeSet::new();

    for observation in observations {
        let root = observation.source.root().to_string();
        roots.insert(root.clone());

        let key = (root, observation.content_signature.clone());
        match seen.get(&key) {
            Some(first) => {
                collapsed.insert(observation.id.clone(), first.clone());
            }
            None => {
                seen.insert(key, observation.id.clone());
                representative_ids.push(observation.id.clone());
            }
        }
    }

    IndependenceReport {
        independent_sources: roots.into_iter().collect(),
        representative_ids,
        collapsed,
        considered: observations.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(id: &str, source: SourceRef, signature: &str) -> ObservationRef {
        ObservationRef {
            id: id.into(),
            source,
            observed_at: 1_700_000_000_000,
            content_signature: signature.into(),
            location: None,
        }
    }

    #[test]
    fn a_direct_source_is_its_own_root() {
        let source = SourceRef::direct("usgs.earthquakes");
        assert_eq!(source.root(), "usgs.earthquakes");
    }

    #[test]
    fn the_root_is_the_far_end_of_the_provenance_chain() {
        // An outlet republishing an agency: the agency is the primary source.
        let source = SourceRef::via("outlet.example", ["wire.agency", "gov.agency"]);
        assert_eq!(source.root(), "gov.agency");
    }

    #[test]
    fn twelve_outlets_carrying_one_wire_story_are_one_source() {
        // THE case this module exists for. Counting these as twelve confirmations is how
        // a single agency report acquires the authority of a consensus.
        let observations: Vec<ObservationRef> = (0..12)
            .map(|i| {
                observation(
                    &format!("obs-{i}"),
                    SourceRef::via("outlet.example", ["wire.reuters"]),
                    "explosion-at-port-x",
                )
            })
            .collect();

        let report = assess_independence(&observations);
        assert_eq!(report.considered, 12);
        assert_eq!(
            report.independent_count(),
            1,
            "twelve republications of one wire story are one source"
        );
        assert!(!report.is_corroborated());
        assert_eq!(report.collapsed_count(), 11);
        assert_eq!(report.representative_ids, vec!["obs-0".to_string()]);
    }

    #[test]
    fn genuinely_independent_sources_are_counted_separately() {
        let observations = vec![
            observation("a", SourceRef::direct("acled"), "clash-at-town-y"),
            observation("b", SourceRef::direct("ucdp"), "clash-at-town-y"),
            observation("c", SourceRef::direct("gdelt"), "clash-at-town-y"),
        ];
        let report = assess_independence(&observations);
        assert_eq!(report.independent_count(), 3);
        assert!(report.is_corroborated());
        assert!(report.collapsed.is_empty());
        assert_eq!(
            report.independent_sources,
            vec!["acled".to_string(), "gdelt".to_string(), "ucdp".to_string()]
        );
    }

    #[test]
    fn one_source_repeating_itself_is_still_one_source() {
        // A feed that re-emits the same record every poll must not accumulate authority.
        let observations: Vec<ObservationRef> = (0..5)
            .map(|i| {
                observation(
                    &format!("poll-{i}"),
                    SourceRef::direct("usgs.earthquakes"),
                    "m6.1-at-depth-10",
                )
            })
            .collect();
        let report = assess_independence(&observations);
        assert_eq!(report.independent_count(), 1);
        assert_eq!(report.representative_ids.len(), 1);
        assert_eq!(report.collapsed_count(), 4);
    }

    #[test]
    fn one_source_reporting_two_different_things_keeps_both_records() {
        // Collapse is per (source, content). The same agency reporting two distinct
        // events must not lose one of them.
        let observations = vec![
            observation("a", SourceRef::direct("usgs.earthquakes"), "quake-1"),
            observation("b", SourceRef::direct("usgs.earthquakes"), "quake-2"),
        ];
        let report = assess_independence(&observations);
        assert_eq!(report.independent_count(), 1, "still one source");
        assert_eq!(
            report.representative_ids.len(),
            2,
            "but two distinct records"
        );
        assert!(report.collapsed.is_empty());
    }

    #[test]
    fn the_same_content_from_different_roots_is_corroboration_not_duplication() {
        let observations = vec![
            observation(
                "a",
                SourceRef::via("outlet.one", ["wire.reuters"]),
                "same-claim",
            ),
            observation("b", SourceRef::via("outlet.two", ["wire.ap"]), "same-claim"),
        ];
        let report = assess_independence(&observations);
        assert_eq!(report.independent_count(), 2);
        assert!(report.is_corroborated());
        assert_eq!(report.representative_ids.len(), 2);
    }

    #[test]
    fn a_mixed_set_counts_only_distinct_origins() {
        // Eight records, three origins. The honest answer is three.
        let mut observations = Vec::new();
        for i in 0..5 {
            observations.push(observation(
                &format!("wire-{i}"),
                SourceRef::via(
                    if i % 2 == 0 { "outlet.a" } else { "outlet.b" },
                    ["wire.reuters"],
                ),
                "event-z",
            ));
        }
        observations.push(observation("acled", SourceRef::direct("acled"), "event-z"));
        observations.push(observation("ucdp", SourceRef::direct("ucdp"), "event-z"));
        observations.push(observation(
            "acled-2",
            SourceRef::direct("acled"),
            "event-z",
        ));

        let report = assess_independence(&observations);
        assert_eq!(report.considered, 8);
        assert_eq!(report.independent_count(), 3);
        assert!(report.is_corroborated());
        assert!(report.explain().contains("3 independent sources"));
    }

    #[test]
    fn the_explanation_states_both_numbers() {
        // "12 reports" and "12 reports from 1 source" are very different claims.
        let observations: Vec<ObservationRef> = (0..12)
            .map(|i| {
                observation(
                    &format!("o{i}"),
                    SourceRef::via("outlet.x", ["wire.one"]),
                    "c",
                )
            })
            .collect();
        let explanation = assess_independence(&observations).explain();
        assert!(explanation.contains("12 report"));
        assert!(explanation.contains("1 independent source"));
        assert!(explanation.contains("collapsed"));
    }

    #[test]
    fn an_empty_set_is_not_corroborated() {
        let report = assess_independence(&[]);
        assert_eq!(report.independent_count(), 0);
        assert!(!report.is_corroborated());
        assert_eq!(report.considered, 0);
    }

    #[test]
    fn collapse_is_deterministic() {
        // The representative must not change between refreshes, or an event's citation
        // moves for no reason a reader can see.
        let observations = vec![
            observation("first", SourceRef::direct("s"), "c"),
            observation("second", SourceRef::direct("s"), "c"),
            observation("third", SourceRef::direct("s"), "c"),
        ];
        for _ in 0..10 {
            let report = assess_independence(&observations);
            assert_eq!(report.representative_ids, vec!["first".to_string()]);
            assert_eq!(report.collapsed["second"], "first");
            assert_eq!(report.collapsed["third"], "first");
        }
    }

    #[test]
    fn shared_origin_is_detected_across_different_immediate_providers() {
        let a = SourceRef::via("outlet.one", ["wire.reuters"]);
        let b = SourceRef::via("outlet.two", ["wire.reuters"]);
        let c = SourceRef::direct("acled");
        assert!(a.shares_origin_with(&b));
        assert!(!a.shares_origin_with(&c));
    }

    // ─────────────────────────────────────────────────────────────── geometry

    #[test]
    fn distance_is_zero_for_the_same_point() {
        let p = GeoPoint::new(51.5, -0.12);
        assert!(p.distance_km(&p) < 1e-9);
    }

    #[test]
    fn distance_matches_a_known_great_circle() {
        // London to Paris is about 344 km.
        let london = GeoPoint::new(51.5074, -0.1278);
        let paris = GeoPoint::new(48.8566, 2.3522);
        let d = london.distance_km(&paris);
        assert!((d - 344.0).abs() < 5.0, "expected ~344 km, got {d}");
    }

    #[test]
    fn distance_is_symmetric() {
        let a = GeoPoint::new(-33.87, 151.21);
        let b = GeoPoint::new(35.68, 139.69);
        let forward = a.distance_km(&b);
        let backward = b.distance_km(&a);
        assert!((forward - backward).abs() < 1e-6);
    }

    #[test]
    fn distance_is_correct_at_high_latitude() {
        // A planar approximation is wrong by tens of kilometres here, which is exactly
        // where it would wrongly merge two distinct events.
        let a = GeoPoint::new(78.0, 15.0);
        let b = GeoPoint::new(78.0, 20.0);
        let d = a.distance_km(&b);
        // 5 degrees of longitude at 78N is about 115 km, not the ~555 km it is at the equator.
        assert!((d - 115.0).abs() < 10.0, "expected ~115 km, got {d}");
    }

    #[test]
    fn antipodal_points_do_not_produce_nan() {
        let a = GeoPoint::new(0.0, 0.0);
        let b = GeoPoint::new(0.0, 180.0);
        let d = a.distance_km(&b);
        assert!(d.is_finite(), "antipodal distance must be finite, got {d}");
        assert!((d - 20015.0).abs() < 50.0, "expected ~20015 km, got {d}");
    }
}
