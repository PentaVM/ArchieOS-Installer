//! Grouping observations into candidate events.
//!
//! Several sources reporting the same occurrence arrive as separate records, minutes
//! apart, with coordinates that disagree by kilometres. Deciding they describe one event
//! is what turns a feed into a picture.
//!
//! The bar is deliberately conservative. Merging two distinct events into one loses an
//! event and cannot be undone by a reader; splitting one event into two is visible and
//! recoverable. So when the evidence is ambiguous this splits.
//!
//! Clustering produces *candidates*, never conclusions. A cluster becomes an Event only
//! after independence counting, and it may turn out to be one source repeating itself.

use crate::evidence::{assess_independence, IndependenceReport, ObservationRef};
use std::collections::BTreeSet;

/// How close two observations must be to belong to the same occurrence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClusterWindow {
    /// Maximum time between two observations of one occurrence.
    pub window_ms: i64,
    /// Maximum great-circle distance, in kilometres.
    pub radius_km: f64,
    /// Whether an observation with no location may join a located cluster.
    ///
    /// Defaults to false. A report with no coordinates could be anywhere, and attaching
    /// it to a located event asserts a precision nobody has.
    pub allow_unlocated: bool,
}

impl Default for ClusterWindow {
    fn default() -> Self {
        Self {
            // An hour and fifty kilometres: wide enough that a wire report filed forty
            // minutes after a local one still joins, tight enough that two separate
            // incidents in one city do not merge.
            window_ms: 3_600_000,
            radius_km: 50.0,
            allow_unlocated: false,
        }
    }
}

impl ClusterWindow {
    pub fn validate(&self) -> Result<(), String> {
        if self.window_ms <= 0 {
            return Err("window_ms must be positive".into());
        }
        if !(self.radius_km.is_finite() && self.radius_km >= 0.0) {
            return Err("radius_km must be a non-negative number".into());
        }
        Ok(())
    }

    /// Whether two observations are close enough to belong together.
    fn links(&self, a: &ObservationRef, b: &ObservationRef) -> bool {
        if (a.observed_at - b.observed_at).abs() > self.window_ms {
            return false;
        }
        match (&a.location, &b.location) {
            (Some(x), Some(y)) => x.distance_km(y) <= self.radius_km,
            // Neither is located: time alone has to carry it, which it can for feeds
            // that are inherently non-geographic.
            (None, None) => true,
            // One located, one not. Joining them would assert a location the second
            // record does not have.
            _ => self.allow_unlocated,
        }
    }
}

/// A group of observations that may describe one occurrence.
#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    /// Observation ids, in input order.
    pub observation_ids: Vec<String>,
    pub earliest_ms: i64,
    pub latest_ms: i64,
    /// Independence over the members. A large cluster from one source is still one
    /// source, and this is where that becomes visible.
    pub independence: IndependenceReport,
}

impl Cluster {
    pub fn len(&self) -> usize {
        self.observation_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.observation_ids.is_empty()
    }

    pub fn duration_ms(&self) -> i64 {
        self.latest_ms - self.earliest_ms
    }

    /// Whether this cluster meets the bar for an event to claim corroboration.
    pub fn is_corroborated(&self) -> bool {
        self.independence.is_corroborated()
    }

    /// The state an Event built from this cluster may claim.
    ///
    /// Never better than the evidence: a cluster of one source is `suspected` however
    /// many records it holds.
    pub fn suggested_event_state(&self) -> &'static str {
        if self.independence.independent_count() >= 2 {
            "corroborated"
        } else {
            "suspected"
        }
    }
}

/// Group observations into candidate occurrences.
///
/// Single-linkage: an observation joins a cluster if it links to *any* member. That is
/// the right choice for reports of one event drifting in over an hour, and its known
/// weakness — chaining across a corridor of near-misses — is bounded by the window being
/// small relative to the phenomena.
///
/// Deterministic: input order decides cluster order and membership order, so the same
/// input always produces the same clusters.
pub fn cluster(observations: &[ObservationRef], window: &ClusterWindow) -> Vec<Cluster> {
    let mut assigned: Vec<Option<usize>> = vec![None; observations.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();

    for i in 0..observations.len() {
        if assigned[i].is_some() {
            continue;
        }
        // Start a new cluster and grow it transitively.
        let group_index = groups.len();
        groups.push(vec![i]);
        assigned[i] = Some(group_index);

        let mut frontier = vec![i];
        while let Some(current) = frontier.pop() {
            for (j, candidate) in observations.iter().enumerate() {
                if assigned[j].is_some() {
                    continue;
                }
                if window.links(&observations[current], candidate) {
                    assigned[j] = Some(group_index);
                    groups[group_index].push(j);
                    frontier.push(j);
                }
            }
        }
    }

    groups
        .into_iter()
        .map(|mut indices| {
            // Restore input order within the cluster; the frontier walk perturbs it.
            indices.sort_unstable();
            let members: Vec<ObservationRef> =
                indices.iter().map(|i| observations[*i].clone()).collect();

            let earliest_ms = members.iter().map(|o| o.observed_at).min().unwrap_or(0);
            let latest_ms = members.iter().map(|o| o.observed_at).max().unwrap_or(0);

            Cluster {
                observation_ids: members.iter().map(|o| o.id.clone()).collect(),
                earliest_ms,
                latest_ms,
                independence: assess_independence(&members),
            }
        })
        .collect()
}

/// Clusters that at least two independent sources support.
///
/// The set worth surfacing on a display without a caveat.
pub fn corroborated_clusters(
    observations: &[ObservationRef],
    window: &ClusterWindow,
) -> Vec<Cluster> {
    cluster(observations, window)
        .into_iter()
        .filter(Cluster::is_corroborated)
        .collect()
}

/// Every distinct primary source across a set of clusters.
pub fn contributing_sources(clusters: &[Cluster]) -> Vec<String> {
    let unique: BTreeSet<String> = clusters
        .iter()
        .flat_map(|c| c.independence.independent_sources.iter().cloned())
        .collect();
    unique.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{GeoPoint, SourceRef};

    const BASE: i64 = 1_700_000_000_000;

    fn at(id: &str, source: &str, minutes: i64, point: Option<GeoPoint>) -> ObservationRef {
        ObservationRef {
            id: id.into(),
            source: SourceRef::direct(source),
            observed_at: BASE + minutes * 60_000,
            content_signature: format!("sig-{id}"),
            location: point,
        }
    }

    #[test]
    fn the_default_window_is_valid() {
        assert!(ClusterWindow::default().validate().is_ok());
    }

    #[test]
    fn an_invalid_window_is_rejected() {
        let window = ClusterWindow {
            window_ms: 0,
            ..Default::default()
        };
        assert!(window.validate().is_err());

        let nan = ClusterWindow {
            radius_km: f64::NAN,
            ..Default::default()
        };
        assert!(nan.validate().is_err());
    }

    #[test]
    fn reports_of_one_occurrence_group_together() {
        let point = GeoPoint::new(50.45, 30.52);
        let observations = vec![
            at("local", "acled", 0, Some(point)),
            at("wire", "reuters", 25, Some(GeoPoint::new(50.46, 30.53))),
            at("db", "ucdp", 40, Some(GeoPoint::new(50.44, 30.50))),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 3);
        assert_eq!(clusters[0].independence.independent_count(), 3);
        assert!(clusters[0].is_corroborated());
        assert_eq!(clusters[0].suggested_event_state(), "corroborated");
    }

    #[test]
    fn occurrences_far_apart_in_space_do_not_merge() {
        let observations = vec![
            at("kyiv", "acled", 0, Some(GeoPoint::new(50.45, 30.52))),
            at("lviv", "acled", 5, Some(GeoPoint::new(49.84, 24.03))),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters.len(), 2, "500 km apart is not one event");
    }

    #[test]
    fn occurrences_far_apart_in_time_do_not_merge() {
        let point = GeoPoint::new(50.45, 30.52);
        let observations = vec![
            at("morning", "acled", 0, Some(point)),
            at("evening", "acled", 600, Some(point)),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters.len(), 2, "ten hours apart is not one event");
    }

    #[test]
    fn an_unlocated_report_does_not_join_a_located_cluster_by_default() {
        // Attaching a report with no coordinates to a located event asserts a precision
        // nobody has.
        let observations = vec![
            at("located", "acled", 0, Some(GeoPoint::new(50.45, 30.52))),
            at("floating", "gdelt", 10, None),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn unlocated_reports_may_join_when_explicitly_permitted() {
        let window = ClusterWindow {
            allow_unlocated: true,
            ..Default::default()
        };
        let observations = vec![
            at("located", "acled", 0, Some(GeoPoint::new(50.45, 30.52))),
            at("floating", "gdelt", 10, None),
        ];
        assert_eq!(cluster(&observations, &window).len(), 1);
    }

    #[test]
    fn non_geographic_feeds_cluster_on_time_alone() {
        let observations = vec![
            at("a", "cisa.kev", 0, None),
            at("b", "first.epss", 10, None),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].independence.independent_count(), 2);
    }

    #[test]
    fn a_big_cluster_from_one_source_is_still_not_corroborated() {
        // The failure this whole module guards against: volume mistaken for consensus.
        let point = GeoPoint::new(50.45, 30.52);
        let observations: Vec<ObservationRef> = (0..20)
            .map(|i| {
                let mut o = at(&format!("rep-{i}"), "outlet.x", i, Some(point));
                o.source = SourceRef::via("outlet.x", ["wire.single"]);
                o.content_signature = "one-claim".into();
                o
            })
            .collect();

        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters.len(), 1);
        assert_eq!(
            clusters[0].len(),
            20,
            "all twenty reports are in the cluster"
        );
        assert_eq!(
            clusters[0].independence.independent_count(),
            1,
            "but they are one source"
        );
        assert!(!clusters[0].is_corroborated());
        assert_eq!(clusters[0].suggested_event_state(), "suspected");
    }

    #[test]
    fn single_linkage_joins_through_an_intermediate() {
        // A and C are 80 km apart, beyond the 50 km radius, but B sits between them.
        // Single linkage joins all three, which is correct for reports drifting in
        // around one occurrence.
        let observations = vec![
            at("a", "s1", 0, Some(GeoPoint::new(50.00, 30.00))),
            at("b", "s2", 5, Some(GeoPoint::new(50.36, 30.00))),
            at("c", "s3", 10, Some(GeoPoint::new(50.72, 30.00))),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 3);
    }

    #[test]
    fn clustering_is_deterministic() {
        let point = GeoPoint::new(50.45, 30.52);
        let observations = vec![
            at("a", "s1", 0, Some(point)),
            at("b", "s2", 5, Some(point)),
            at("c", "s3", 400, Some(point)),
        ];
        let first = cluster(&observations, &ClusterWindow::default());
        for _ in 0..10 {
            assert_eq!(cluster(&observations, &ClusterWindow::default()), first);
        }
    }

    #[test]
    fn cluster_members_keep_input_order() {
        let point = GeoPoint::new(50.45, 30.52);
        let observations = vec![
            at("first", "s1", 0, Some(point)),
            at("second", "s2", 5, Some(point)),
            at("third", "s3", 10, Some(point)),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(
            clusters[0].observation_ids,
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );
    }

    #[test]
    fn cluster_reports_its_time_span() {
        let point = GeoPoint::new(50.45, 30.52);
        let observations = vec![
            at("a", "s1", 0, Some(point)),
            at("b", "s2", 30, Some(point)),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters[0].duration_ms(), 30 * 60_000);
        assert_eq!(clusters[0].earliest_ms, BASE);
    }

    #[test]
    fn an_empty_input_produces_no_clusters() {
        assert!(cluster(&[], &ClusterWindow::default()).is_empty());
    }

    #[test]
    fn only_corroborated_clusters_are_returned_by_the_filtered_call() {
        let point = GeoPoint::new(50.45, 30.52);
        let far = GeoPoint::new(40.0, 10.0);
        let observations = vec![
            // corroborated: two independent sources, same place and time
            at("a", "acled", 0, Some(point)),
            at("b", "ucdp", 5, Some(point)),
            // lone report, far away
            at("c", "gdelt", 0, Some(far)),
        ];
        let corroborated = corroborated_clusters(&observations, &ClusterWindow::default());
        assert_eq!(corroborated.len(), 1);
        assert_eq!(corroborated[0].len(), 2);
    }

    #[test]
    fn contributing_sources_are_deduplicated_and_sorted() {
        let point = GeoPoint::new(50.45, 30.52);
        let observations = vec![
            at("a", "ucdp", 0, Some(point)),
            at("b", "acled", 5, Some(point)),
            at("c", "acled", 400, Some(point)),
        ];
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(
            contributing_sources(&clusters),
            vec!["acled".to_string(), "ucdp".to_string()]
        );
    }

    #[test]
    fn a_large_input_completes_and_stays_correct() {
        // 600 observations across 6 well-separated places. Guards against the frontier
        // walk mis-assigning members as the input grows.
        let mut observations = Vec::new();
        for place in 0..6 {
            let point = GeoPoint::new(10.0 * place as f64, 20.0 * place as f64);
            for i in 0..100 {
                observations.push(at(
                    &format!("p{place}-o{i}"),
                    &format!("source-{}", i % 3),
                    i % 30,
                    Some(point),
                ));
            }
        }
        let clusters = cluster(&observations, &ClusterWindow::default());
        assert_eq!(clusters.len(), 6, "six distinct places, six clusters");
        for c in &clusters {
            assert_eq!(c.len(), 100);
            assert_eq!(c.independence.independent_count(), 3);
        }
    }
}
