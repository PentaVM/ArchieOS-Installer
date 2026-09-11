//! Detectors: turning evidence into findings, auditably.
//!
//! A detector is the opposite of a magic classifier. It declares its inputs, its window,
//! its baseline and its named thresholds, and every finding it emits carries the
//! detector *version* that produced it. Two consequences, and both are the point:
//!
//! 1. A finding from six months ago can be re-derived, because the threshold that
//!    produced it is recorded rather than remembered.
//! 2. A model may propose a detector and a human may accept it, but nothing retunes a
//!    live threshold on its own. Changing one is a version bump with an author.
//!
//! # Three results, not two
//!
//! `Fired`, `Cleared`, and `Unavailable`. The third matters: a detector whose inputs
//! were missing must say so, because a gap in findings and a period of genuine quiet
//! look identical otherwise, and only one of them means "go and fix the feed".
//!
//! # Shadow mode
//!
//! A detector in [`DetectorState::Shadow`] evaluates against live traffic and records
//! findings that never alert and never change the display. It is how a new threshold
//! earns trust before it is given any.

use crate::evidence::IndependenceReport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How much a finding is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorState {
    /// Findings alert, and may change the scene if the detector is permitted to.
    Enabled,
    /// Findings are recorded and nothing else. No alert, no scene change.
    Shadow,
    Disabled,
}

impl DetectorState {
    /// Whether a finding from this detector may reach an operator.
    pub fn may_alert(&self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Whether the detector runs at all.
    pub fn evaluates(&self) -> bool {
        matches!(self, Self::Enabled | Self::Shadow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Informational,
    Low,
    Moderate,
    High,
    Severe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceBand {
    Reported,
    Probable,
    Corroborated,
    Confirmed,
    Disputed,
}

/// A named number the detector compares against, kept as data so a finding can be
/// explained by quoting the threshold it crossed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Threshold {
    pub value: f64,
    /// Free-text unit. Present so an explanation reads "3.4 sigma" rather than "3.4".
    pub unit: &'static str,
}

impl Threshold {
    pub fn new(value: f64, unit: &'static str) -> Self {
        Self { value, unit }
    }
}

/// The declarative part of a detector: everything an auditor needs.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectorSpec {
    pub id: &'static str,
    /// Semantic version. Any threshold or logic change requires a bump, so a retuned
    /// detector never rewrites the history of the old one.
    pub version: &'static str,
    pub title: &'static str,
    pub state: DetectorState,
    /// Evaluation window.
    pub window_ms: i64,
    /// Comparison period for anomaly detectors. "Unusual" is meaningless without saying
    /// unusual against what.
    pub baseline_ms: Option<i64>,
    pub thresholds: BTreeMap<&'static str, Threshold>,
    /// Independent sources required before the detector may fire at all.
    pub minimum_independent_sources: usize,
    pub default_severity: Severity,
    /// Whether a finding may take over the observatory display.
    ///
    /// Opt-in, deliberately: a detector that can seize the screen is a detector that can
    /// be used to hide something else.
    pub may_change_scene: bool,
    /// Ordinary innocent explanations, so an operator reading an alert can rule them out
    /// before acting.
    pub false_positive_notes: &'static [&'static str],
}

impl DetectorSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("detector id must not be empty".into());
        }
        let parts: Vec<&str> = self.version.split('.').collect();
        if parts.len() != 3 || parts.iter().any(|p| p.parse::<u32>().is_err()) {
            return Err(format!(
                "detector {} version {:?} must be semantic (major.minor.patch)",
                self.id, self.version
            ));
        }
        if self.window_ms <= 0 {
            return Err("window_ms must be positive".into());
        }
        if let Some(baseline) = self.baseline_ms {
            if baseline <= self.window_ms {
                return Err(format!(
                    "detector {}: baseline_ms must exceed window_ms, or the window is \
                     being compared against itself",
                    self.id
                ));
            }
        }
        // A detector that can seize the display must document how it is wrong.
        if self.may_change_scene && self.false_positive_notes.is_empty() {
            return Err(format!(
                "detector {} may change the scene but documents no false positives",
                self.id
            ));
        }
        Ok(())
    }

    pub fn threshold(&self, name: &str) -> Option<Threshold> {
        self.thresholds.get(name).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingResult {
    Fired,
    Cleared,
    /// Inputs were missing. Recorded so a gap in findings is distinguishable from quiet.
    Unavailable,
}

/// One evaluation of a detector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub detector_id: String,
    /// The version that produced this. Without it the finding becomes uninterpretable
    /// the moment the detector is retuned.
    pub detector_version: String,
    pub created_at_ms: i64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub result: FindingResult,
    pub severity: Severity,
    /// Which threshold was crossed and by how much, in plain language.
    pub explanation: String,
    pub evidence_refs: Vec<String>,
    /// Actual values against the thresholds, so a marginal firing can be judged.
    pub measured: BTreeMap<String, f64>,
    pub confidence_band: ConfidenceBand,
    /// False when the detector was in shadow. A shadow finding is a record, not an alert.
    pub alerting: bool,
    /// Whether this finding is permitted to change the observatory scene.
    pub may_change_scene: bool,
}

impl Finding {
    fn base(spec: &DetectorSpec, now_ms: i64, result: FindingResult) -> Self {
        Self {
            detector_id: spec.id.to_string(),
            detector_version: spec.version.to_string(),
            created_at_ms: now_ms,
            window_start_ms: now_ms - spec.window_ms,
            window_end_ms: now_ms,
            result,
            severity: spec.default_severity,
            explanation: String::new(),
            evidence_refs: Vec::new(),
            measured: BTreeMap::new(),
            confidence_band: ConfidenceBand::Reported,
            // A shadow detector never alerts, whatever it found.
            alerting: result == FindingResult::Fired && spec.state.may_alert(),
            may_change_scene: result == FindingResult::Fired
                && spec.state.may_alert()
                && spec.may_change_scene,
        }
    }

    /// Whether this finding should reach an operator.
    pub fn is_actionable(&self) -> bool {
        self.result == FindingResult::Fired && self.alerting
    }
}

/// A window of counts, and the baseline it is judged against.
#[derive(Debug, Clone, PartialEq)]
pub struct RateSample {
    /// Count observed in the current window.
    pub current: f64,
    /// Counts from equivalent earlier windows.
    pub baseline: Vec<f64>,
}

impl RateSample {
    pub fn mean(&self) -> Option<f64> {
        if self.baseline.is_empty() {
            return None;
        }
        Some(self.baseline.iter().sum::<f64>() / self.baseline.len() as f64)
    }

    /// Sample standard deviation. `None` below two points, where it is undefined.
    pub fn stddev(&self) -> Option<f64> {
        if self.baseline.len() < 2 {
            return None;
        }
        let mean = self.mean()?;
        let variance = self
            .baseline
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (self.baseline.len() - 1) as f64;
        Some(variance.sqrt())
    }

    /// How many standard deviations the current window sits above the baseline.
    ///
    /// `None` when the baseline is too short or has no spread. A flat baseline makes
    /// every deviation infinite, which would fire on the first non-zero value forever;
    /// refusing to produce a number is the honest answer.
    pub fn sigma(&self) -> Option<f64> {
        let mean = self.mean()?;
        let stddev = self.stddev()?;
        if stddev <= f64::EPSILON {
            return None;
        }
        Some((self.current - mean) / stddev)
    }
}

/// Fires when independent sources converge on one occurrence.
///
/// The meta-detector, and the one that earns the screen: it says nothing about what
/// happened, only that several origins that do not copy each other agree that something
/// did.
pub fn multi_source_corroboration_spec() -> DetectorSpec {
    let mut thresholds = BTreeMap::new();
    thresholds.insert("independent_sources", Threshold::new(3.0, "sources"));
    DetectorSpec {
        id: "fusion.multi-source-corroborated",
        version: "1.0.0",
        title: "Independent sources converging on one occurrence",
        state: DetectorState::Shadow,
        window_ms: 3_600_000,
        baseline_ms: None,
        thresholds,
        minimum_independent_sources: 2,
        default_severity: Severity::Moderate,
        may_change_scene: true,
        false_positive_notes: &[
            "Sources that appear independent may share an unrecorded upstream; the \
             collapse is only as good as the provenance chains the adapters populate.",
            "Several outlets independently monitoring the same public feed (a police \
             scanner, an official account) are one observation, not several.",
            "Scheduled events — exercises, launches, announced tests — produce genuine \
             multi-source convergence with no news value.",
        ],
    }
}

/// Evaluate corroboration convergence over one cluster.
pub fn evaluate_corroboration(
    spec: &DetectorSpec,
    independence: &IndependenceReport,
    evidence_refs: &[String],
    now_ms: i64,
) -> Finding {
    let mut finding = Finding::base(spec, now_ms, FindingResult::Cleared);
    let observed = independence.independent_count() as f64;
    finding
        .measured
        .insert("independent_sources".into(), observed);
    finding.evidence_refs = evidence_refs.to_vec();

    let Some(threshold) = spec.threshold("independent_sources") else {
        let mut unavailable = Finding::base(spec, now_ms, FindingResult::Unavailable);
        unavailable.explanation = "detector has no independent_sources threshold configured".into();
        return unavailable;
    };

    // Below the floor the detector may not speak at all, even if the threshold is lower.
    if independence.independent_count() < spec.minimum_independent_sources {
        finding.explanation = format!(
            "{} independent source(s); the detector requires at least {} before it may fire",
            independence.independent_count(),
            spec.minimum_independent_sources
        );
        finding.confidence_band = ConfidenceBand::Reported;
        return finding;
    }

    if observed >= threshold.value {
        let mut fired = Finding::base(spec, now_ms, FindingResult::Fired);
        fired
            .measured
            .insert("independent_sources".into(), observed);
        fired.evidence_refs = evidence_refs.to_vec();
        fired.confidence_band = ConfidenceBand::Corroborated;
        fired.severity = spec.default_severity;
        fired.explanation = format!(
            "{observed:.0} independent {} agree, at or above the threshold of {:.0} {}",
            if observed == 1.0 { "source" } else { "sources" },
            threshold.value,
            threshold.unit
        );
        return fired;
    }

    finding.confidence_band = ConfidenceBand::Corroborated;
    finding.explanation = format!(
        "{observed:.0} independent sources, below the threshold of {:.0} {}",
        threshold.value, threshold.unit
    );
    finding
}

/// Fires when a rate departs from its own recent baseline.
///
/// Generic on purpose: outage counts, scanner volume, transit rates and airlift density
/// are the same shape of question, and one well-tested implementation is better than
/// four hand-rolled ones.
pub fn baseline_deviation_spec(id: &'static str, title: &'static str, sigma: f64) -> DetectorSpec {
    let mut thresholds = BTreeMap::new();
    thresholds.insert("sigma", Threshold::new(sigma, "sigma"));
    thresholds.insert("minimum_baseline_points", Threshold::new(8.0, "windows"));
    DetectorSpec {
        id,
        version: "1.0.0",
        title,
        state: DetectorState::Shadow,
        window_ms: 900_000,
        baseline_ms: Some(30 * 86_400_000),
        thresholds,
        minimum_independent_sources: 1,
        default_severity: Severity::Moderate,
        may_change_scene: false,
        false_positive_notes: &[
            "A provider outage looks identical to a collapse in the thing being measured. \
             Check provider health before believing a drop.",
            "Seasonal and diurnal cycles are not removed; a baseline shorter than the \
             cycle will fire every day at the same hour.",
            "A baseline containing a previous anomaly inflates the standard deviation and \
             hides the next one.",
        ],
    }
}

/// Evaluate a rate against its baseline.
pub fn evaluate_baseline_deviation(
    spec: &DetectorSpec,
    sample: &RateSample,
    evidence_refs: &[String],
    now_ms: i64,
) -> Finding {
    let minimum_points = spec
        .threshold("minimum_baseline_points")
        .map(|t| t.value as usize)
        .unwrap_or(8);

    // Too little history is `Unavailable`, not `Cleared`. Saying "nothing detected" when
    // the detector could not run is how a broken feed reads as a calm world.
    if sample.baseline.len() < minimum_points {
        let mut finding = Finding::base(spec, now_ms, FindingResult::Unavailable);
        finding.explanation = format!(
            "baseline has {} window(s); at least {minimum_points} are required before a \
             deviation means anything",
            sample.baseline.len()
        );
        return finding;
    }

    let Some(sigma) = sample.sigma() else {
        let mut finding = Finding::base(spec, now_ms, FindingResult::Unavailable);
        finding.explanation =
            "baseline has no spread, so a deviation cannot be expressed in sigma".into();
        return finding;
    };

    let threshold = spec
        .threshold("sigma")
        .unwrap_or(Threshold::new(3.0, "sigma"));

    let mut measured = BTreeMap::new();
    measured.insert("sigma".to_string(), sigma);
    measured.insert("current".to_string(), sample.current);
    if let Some(mean) = sample.mean() {
        measured.insert("baseline_mean".to_string(), mean);
    }

    let exceeded = sigma.abs() >= threshold.value;
    let mut finding = Finding::base(
        spec,
        now_ms,
        if exceeded {
            FindingResult::Fired
        } else {
            FindingResult::Cleared
        },
    );
    finding.measured = measured;
    finding.evidence_refs = evidence_refs.to_vec();
    finding.confidence_band = ConfidenceBand::Probable;
    finding.severity = if sigma.abs() >= threshold.value * 2.0 {
        Severity::High
    } else {
        spec.default_severity
    };
    finding.explanation = format!(
        "{:.2} {} from a baseline mean of {:.2} over {} window(s); threshold {:.2}",
        sigma,
        threshold.unit,
        sample.mean().unwrap_or(0.0),
        sample.baseline.len(),
        threshold.value
    );
    finding
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{assess_independence, ObservationRef, SourceRef};

    const NOW: i64 = 1_700_000_000_000;

    fn observation(id: &str, source: SourceRef, signature: &str) -> ObservationRef {
        ObservationRef {
            id: id.into(),
            source,
            observed_at: NOW,
            content_signature: signature.into(),
            location: None,
        }
    }

    fn independence_from(sources: &[&str]) -> IndependenceReport {
        let observations: Vec<ObservationRef> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| observation(&format!("o{i}"), SourceRef::direct(*s), "claim"))
            .collect();
        assess_independence(&observations)
    }

    // ─────────────────────────────────────────────────────────── spec validity

    #[test]
    fn the_shipped_specs_validate() {
        assert!(multi_source_corroboration_spec().validate().is_ok());
        assert!(baseline_deviation_spec("test.rate", "Test", 3.0)
            .validate()
            .is_ok());
    }

    #[test]
    fn a_non_semantic_version_is_rejected() {
        let mut spec = multi_source_corroboration_spec();
        spec.version = "1.0";
        assert!(spec.validate().is_err());
    }

    #[test]
    fn a_baseline_must_be_longer_than_the_window_it_judges() {
        // Otherwise the window is being compared against itself.
        let mut spec = baseline_deviation_spec("t", "T", 3.0);
        spec.baseline_ms = Some(spec.window_ms);
        assert!(spec.validate().is_err());
    }

    #[test]
    fn a_detector_that_can_seize_the_screen_must_document_how_it_is_wrong() {
        let mut spec = multi_source_corroboration_spec();
        assert!(spec.may_change_scene);
        assert!(!spec.false_positive_notes.is_empty());

        spec.false_positive_notes = &[];
        assert!(
            spec.validate().is_err(),
            "a scene-changing detector with no documented false positives must be refused"
        );
    }

    #[test]
    fn detectors_ship_in_shadow_mode() {
        // A new threshold earns trust before it is given any.
        assert_eq!(
            multi_source_corroboration_spec().state,
            DetectorState::Shadow
        );
        assert_eq!(
            baseline_deviation_spec("t", "T", 3.0).state,
            DetectorState::Shadow
        );
    }

    // ───────────────────────────────────────────────────────────── shadow mode

    #[test]
    fn a_shadow_detector_records_but_never_alerts() {
        let spec = multi_source_corroboration_spec();
        assert_eq!(spec.state, DetectorState::Shadow);

        let independence = independence_from(&["acled", "ucdp", "gdelt"]);
        let finding = evaluate_corroboration(&spec, &independence, &["a".into()], NOW);

        assert_eq!(finding.result, FindingResult::Fired, "it did detect");
        assert!(!finding.alerting, "but it must not alert from shadow");
        assert!(!finding.is_actionable());
        assert!(
            !finding.may_change_scene,
            "and it certainly must not take the screen"
        );
    }

    #[test]
    fn the_same_detection_alerts_once_enabled() {
        let mut spec = multi_source_corroboration_spec();
        spec.state = DetectorState::Enabled;

        let independence = independence_from(&["acled", "ucdp", "gdelt"]);
        let finding = evaluate_corroboration(&spec, &independence, &[], NOW);

        assert_eq!(finding.result, FindingResult::Fired);
        assert!(finding.alerting);
        assert!(finding.is_actionable());
        assert!(finding.may_change_scene);
    }

    #[test]
    fn a_cleared_finding_never_alerts_even_when_enabled() {
        let mut spec = multi_source_corroboration_spec();
        spec.state = DetectorState::Enabled;
        let independence = independence_from(&["acled", "ucdp"]); // 2, threshold is 3
        let finding = evaluate_corroboration(&spec, &independence, &[], NOW);
        assert_eq!(finding.result, FindingResult::Cleared);
        assert!(!finding.alerting);
    }

    #[test]
    fn a_disabled_detector_does_not_evaluate() {
        assert!(!DetectorState::Disabled.evaluates());
        assert!(DetectorState::Shadow.evaluates());
        assert!(DetectorState::Enabled.evaluates());
        assert!(!DetectorState::Shadow.may_alert());
        assert!(DetectorState::Enabled.may_alert());
    }

    // ──────────────────────────────────────────────────────────── provenance

    #[test]
    fn every_finding_carries_the_version_that_produced_it() {
        let spec = multi_source_corroboration_spec();
        let finding = evaluate_corroboration(&spec, &independence_from(&["a"]), &[], NOW);
        assert_eq!(finding.detector_id, "fusion.multi-source-corroborated");
        assert_eq!(finding.detector_version, "1.0.0");
    }

    #[test]
    fn a_finding_explains_which_threshold_it_measured_against() {
        let spec = multi_source_corroboration_spec();
        let finding = evaluate_corroboration(&spec, &independence_from(&["a", "b", "c"]), &[], NOW);
        assert!(finding.explanation.contains('3'), "{}", finding.explanation);
        assert_eq!(finding.measured["independent_sources"], 3.0);
    }

    #[test]
    fn a_finding_carries_its_evidence() {
        let spec = multi_source_corroboration_spec();
        let refs = vec!["obs-1".to_string(), "obs-2".to_string()];
        let finding =
            evaluate_corroboration(&spec, &independence_from(&["a", "b", "c"]), &refs, NOW);
        assert_eq!(finding.evidence_refs, refs);
    }

    #[test]
    fn a_finding_round_trips_through_json() {
        let spec = multi_source_corroboration_spec();
        let finding = evaluate_corroboration(&spec, &independence_from(&["a", "b", "c"]), &[], NOW);
        let json = serde_json::to_string(&finding).unwrap();
        assert_eq!(serde_json::from_str::<Finding>(&json).unwrap(), finding);
    }

    // ─────────────────────────────────────────────────────────── corroboration

    #[test]
    fn a_single_source_cannot_make_the_detector_fire() {
        // Volume is not consensus. Twenty records from one origin stay below the floor.
        let spec = multi_source_corroboration_spec();
        let observations: Vec<ObservationRef> = (0..20)
            .map(|i| {
                observation(
                    &format!("o{i}"),
                    SourceRef::via("outlet.x", ["wire.single"]),
                    "one-claim",
                )
            })
            .collect();
        let independence = assess_independence(&observations);

        let finding = evaluate_corroboration(&spec, &independence, &[], NOW);
        assert_eq!(finding.result, FindingResult::Cleared);
        assert!(finding.explanation.contains("at least 2"));
    }

    #[test]
    fn convergence_of_independent_origins_fires() {
        let spec = multi_source_corroboration_spec();
        let independence = independence_from(&["acled", "ucdp", "gdelt", "reliefweb"]);
        let finding = evaluate_corroboration(&spec, &independence, &[], NOW);
        assert_eq!(finding.result, FindingResult::Fired);
        assert_eq!(finding.confidence_band, ConfidenceBand::Corroborated);
    }

    // ────────────────────────────────────────────────────── baseline deviation

    #[test]
    fn a_short_baseline_is_unavailable_not_clear() {
        // "Nothing detected" when the detector could not run is how a broken feed reads
        // as a calm world.
        let spec = baseline_deviation_spec("t", "T", 3.0);
        let sample = RateSample {
            current: 100.0,
            baseline: vec![1.0, 2.0, 3.0],
        };
        let finding = evaluate_baseline_deviation(&spec, &sample, &[], NOW);
        assert_eq!(finding.result, FindingResult::Unavailable);
        assert!(finding.explanation.contains("at least 8"));
        assert!(!finding.alerting);
    }

    #[test]
    fn a_flat_baseline_cannot_express_a_deviation() {
        // Zero variance makes every deviation infinite, which would fire forever.
        let spec = baseline_deviation_spec("t", "T", 3.0);
        let sample = RateSample {
            current: 5.0,
            baseline: vec![10.0; 12],
        };
        let finding = evaluate_baseline_deviation(&spec, &sample, &[], NOW);
        assert_eq!(finding.result, FindingResult::Unavailable);
        assert!(finding.explanation.contains("no spread"));
    }

    #[test]
    fn a_normal_window_clears() {
        let spec = baseline_deviation_spec("t", "T", 3.0);
        let sample = RateSample {
            current: 11.0,
            baseline: vec![10.0, 11.0, 9.0, 10.5, 10.0, 9.5, 11.0, 10.0, 10.2, 9.8],
        };
        let finding = evaluate_baseline_deviation(&spec, &sample, &[], NOW);
        assert_eq!(finding.result, FindingResult::Cleared);
    }

    #[test]
    fn a_genuine_spike_fires_and_reports_its_sigma() {
        let spec = baseline_deviation_spec("t", "T", 3.0);
        let sample = RateSample {
            current: 60.0,
            baseline: vec![10.0, 11.0, 9.0, 10.5, 10.0, 9.5, 11.0, 10.0, 10.2, 9.8],
        };
        let finding = evaluate_baseline_deviation(&spec, &sample, &[], NOW);
        assert_eq!(finding.result, FindingResult::Fired);
        assert!(finding.measured["sigma"] > 3.0);
        assert!(finding.explanation.contains("sigma"));
        // A very large excursion escalates severity.
        assert_eq!(finding.severity, Severity::High);
    }

    #[test]
    fn a_collapse_fires_as_readily_as_a_spike() {
        // An outage is a drop. A detector that only watches for increases misses every
        // failure it was built for.
        let spec = baseline_deviation_spec("t", "T", 3.0);
        let sample = RateSample {
            current: 0.0,
            baseline: vec![
                100.0, 102.0, 98.0, 101.0, 99.0, 100.5, 97.0, 103.0, 100.0, 99.5,
            ],
        };
        let finding = evaluate_baseline_deviation(&spec, &sample, &[], NOW);
        assert_eq!(finding.result, FindingResult::Fired);
        assert!(
            finding.measured["sigma"] < -3.0,
            "a drop is a negative sigma"
        );
    }

    #[test]
    fn statistics_are_correct_on_a_known_sample() {
        let sample = RateSample {
            current: 10.0,
            baseline: vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0],
        };
        assert_eq!(sample.mean(), Some(5.0));
        // Sample standard deviation of that set is ~2.138.
        let stddev = sample.stddev().unwrap();
        assert!((stddev - 2.1381).abs() < 0.001, "got {stddev}");
        let sigma = sample.sigma().unwrap();
        assert!((sigma - 2.3386).abs() < 0.001, "got {sigma}");
    }

    #[test]
    fn statistics_refuse_to_answer_without_enough_points() {
        assert_eq!(
            RateSample {
                current: 1.0,
                baseline: vec![]
            }
            .mean(),
            None
        );
        assert_eq!(
            RateSample {
                current: 1.0,
                baseline: vec![5.0]
            }
            .stddev(),
            None,
            "standard deviation is undefined for a single point"
        );
    }

    #[test]
    fn a_baseline_detector_does_not_take_the_screen() {
        // Rate deviations are frequent and often innocent; they annotate, they do not
        // seize the observatory.
        assert!(!baseline_deviation_spec("t", "T", 3.0).may_change_scene);
    }

    #[test]
    fn every_shipped_detector_documents_its_false_positives() {
        for spec in [
            multi_source_corroboration_spec(),
            baseline_deviation_spec("t", "T", 3.0),
        ] {
            assert!(
                !spec.false_positive_notes.is_empty(),
                "detector {} documents no ordinary innocent explanations",
                spec.id
            );
        }
    }
}
