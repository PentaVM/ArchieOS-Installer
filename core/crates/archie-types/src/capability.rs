//! Capabilities: what a node can *do*.
//!
//! The fabric is capability-based rather than machine-based. Archie Core never asks
//! "is this the box with the 3070 in it"; it asks "who advertises `compute.cuda` with
//! at least 8 GiB of accelerator memory". That indirection is the whole reason adding
//! hardware expands Archie without an architectural rewrite.
//!
//! A capability name is a dotted, lowercase path: `family.specific[.detail]`. The
//! leading segment is the family, and families are open — a future accelerator only
//! needs a new name under `compute.`, not a new concept.

use crate::error::{Result, TypeError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// A validated capability name, e.g. `compute.cuda`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Capability(String);

impl Capability {
    /// Parse and validate. Requires at least two segments so that every capability
    /// belongs to a family; a bare `compute` is meaningless to the scheduler.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        Self::validate(&value).map_err(|reason| TypeError::InvalidCapability {
            name: value.clone(),
            reason,
        })?;
        Ok(Self(value))
    }

    fn validate(value: &str) -> std::result::Result<(), &'static str> {
        if value.is_empty() {
            return Err("must not be empty");
        }
        if value.len() > 128 {
            return Err("must be at most 128 characters");
        }
        let segments: Vec<&str> = value.split('.').collect();
        if segments.len() < 2 {
            return Err("must have at least two dotted segments, e.g. compute.cpu");
        }
        for segment in segments {
            if segment.is_empty() {
                return Err("segments must not be empty");
            }
            let mut chars = segment.chars();
            match chars.next() {
                Some(c) if c.is_ascii_lowercase() => {}
                _ => return Err("each segment must start with a lowercase letter"),
            }
            for c in chars {
                if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
                    return Err("segments allow only [a-z0-9_-]");
                }
            }
        }
        Ok(())
    }

    /// Construct from a string known at compile time to be valid.
    ///
    /// # Panics
    /// Panics if the name is invalid. Only used for the `well_known` constants below,
    /// which are covered by a test that parses every one of them.
    pub fn known(value: &'static str) -> Self {
        match Self::parse(value) {
            Ok(cap) => cap,
            Err(err) => panic!("invalid built-in capability {value:?}: {err}"),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first segment, e.g. `compute` for `compute.cuda`.
    pub fn family(&self) -> &str {
        self.0.split('.').next().unwrap_or(&self.0)
    }

    /// Whether `self` is `other` or a more specific form of it.
    ///
    /// `compute.cuda.sm86` satisfies a requirement for `compute.cuda`, but
    /// `compute.cudax` does not — the check is segment-wise, not a substring match.
    pub fn satisfies(&self, required: &Capability) -> bool {
        if self.0 == required.0 {
            return true;
        }
        self.0.len() > required.0.len()
            && self.0.starts_with(&required.0)
            && self.0.as_bytes()[required.0.len()] == b'.'
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Capability {
    type Err = TypeError;
    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl TryFrom<String> for Capability {
    type Error = TypeError;
    fn try_from(value: String) -> Result<Self> {
        Self::parse(value)
    }
}

impl From<Capability> for String {
    fn from(value: Capability) -> String {
        value.0
    }
}

/// An ordered, de-duplicated set of capabilities.
///
/// Ordering is deliberate: two nodes with the same capabilities must produce the same
/// serialized manifest, so that manifest digests are comparable and signatures are
/// reproducible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    pub fn new() -> Self {
        Self(BTreeSet::new())
    }

    pub fn insert(&mut self, capability: Capability) -> bool {
        self.0.insert(capability)
    }

    pub fn contains(&self, capability: &Capability) -> bool {
        self.0.contains(capability)
    }

    /// True if any held capability satisfies `required`, honouring specificity.
    pub fn provides(&self, required: &Capability) -> bool {
        self.0.iter().any(|held| held.satisfies(required))
    }

    /// Every requirement in `required` that this set does not provide.
    pub fn missing<'a, I>(&self, required: I) -> Vec<Capability>
    where
        I: IntoIterator<Item = &'a Capability>,
    {
        required
            .into_iter()
            .filter(|req| !self.provides(req))
            .cloned()
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Capabilities present in `self` but not in `other`. Used to report what appeared
    /// or disappeared between boots.
    pub fn difference(&self, other: &CapabilitySet) -> Vec<Capability> {
        self.0.difference(&other.0).cloned().collect()
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for CapabilitySet {
    type Item = Capability;
    type IntoIter = std::collections::btree_set::IntoIter<Capability>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// The capability names ArchieOS itself understands.
///
/// Nodes may advertise names outside this list; the scheduler treats unknown names as
/// opaque and simply matches them literally. These constants exist so that control-plane
/// code never spells a well-known name as a bare string literal.
pub mod well_known {
    use super::Capability;

    macro_rules! caps {
        ($($fn_name:ident => $value:literal),* $(,)?) => {
            $(
                pub fn $fn_name() -> Capability {
                    Capability::known($value)
                }
            )*

            /// Every well-known capability, for validation and documentation.
            pub fn all() -> Vec<Capability> {
                vec![$(Capability::known($value)),*]
            }
        };
    }

    caps! {
        compute_cpu => "compute.cpu",
        compute_cuda => "compute.cuda",
        compute_rocm => "compute.rocm",
        compute_tenstorrent => "compute.tenstorrent",
        compute_inference => "compute.inference",
        compute_embedding => "compute.embedding",
        compute_reranking => "compute.reranking",

        storage_object => "storage.object",
        storage_vector => "storage.vector",
        storage_knowledge => "storage.knowledge",

        network_internet => "network.internet",
        network_lan => "network.lan",

        sensor_camera => "sensor.camera",
        sensor_imu => "sensor.imu",
        sensor_thermal => "sensor.thermal",

        simulation_fea => "simulation.fea",
        simulation_cfd => "simulation.cfd",
        simulation_multibody => "simulation.multibody",

        cad_parametric => "cad.parametric",

        robot_actuator => "robot.actuator",
        radio_lora => "radio.lora",
        bus_can => "bus.can",

        // ── world observation ────────────────────────────────────────────────
        // Each name means "this node can ingest and normalize that class of
        // observation", never "this node has that data". A capability is a
        // competence; what has actually been observed lives in the knowledge store.
        world_aviation => "world.aviation",
        world_maritime => "world.maritime",
        world_orbital => "world.orbital",
        world_seismic => "world.seismic",
        world_wildfire => "world.wildfire",
        world_weather => "world.weather",
        world_spaceweather => "world.spaceweather",
        world_disaster => "world.disaster",
        world_conflict => "world.conflict",
        world_gnss => "world.gnss",
        world_airspace => "world.airspace",
        world_infrastructure => "world.infrastructure",
        world_imagery => "world.imagery",

        // ── cyber situational awareness (passive/defensive only) ─────────────
        // These describe the ability to OBSERVE exposure and routing health.
        // There is deliberately no capability for scanning, exploitation or any
        // other active measure: see docs/SAFETY-CYBER.md.
        cyber_vulnerability => "cyber.vulnerability",
        cyber_exposure => "cyber.exposure",
        cyber_routing => "cyber.routing",
        cyber_outage => "cyber.outage",
        cyber_censorship => "cyber.censorship",
        cyber_threatintel => "cyber.threatintel",

        // ── markets and macro ────────────────────────────────────────────────
        market_equities => "market.equities",
        market_commodities => "market.commodities",
        market_fx => "market.fx",
        market_macro => "market.macro",
        market_filings => "market.filings",

        // ── media and provenance ─────────────────────────────────────────────
        // `media.provenance` is the public professional source graph: bylines,
        // affiliations, corrections. It is explicitly NOT a person-tracking
        // capability; the schema forbids private-life fields outright.
        media_news => "media.news",
        media_corpus => "media.corpus",
        media_provenance => "media.provenance",

        // ── fusion: turning observations into evidence ───────────────────────
        // The differentiator. Anyone can put dots on a globe; these are the
        // competences that connect a dot to prior knowledge and to other sources.
        fusion_geocoding => "fusion.geocoding",
        fusion_entity_resolution => "fusion.entity-resolution",
        fusion_deduplication => "fusion.deduplication",
        fusion_corroboration => "fusion.corroboration",
        fusion_detector => "fusion.detector",

        // ── engineering: simulation and CAD ──────────────────────────────────
        simulation_thermal => "simulation.thermal",
        simulation_combustion => "simulation.combustion",
        simulation_turbomachinery => "simulation.turbomachinery",
        simulation_surrogate => "simulation.surrogate",
        simulation_optimization => "simulation.optimization",

        cad_lattice => "cad.lattice",
        cad_airfoil => "cad.airfoil",
        cad_export_step => "cad.export.step",
        cad_export_stl => "cad.export.stl",
    }
}

/// Capability families and what each one means.
///
/// A family is the first dotted segment. Keeping the descriptions here rather than
/// scattered through documentation means `archie topology` can explain an unfamiliar
/// capability without the reader leaving the terminal.
pub fn family_description(family: &str) -> Option<&'static str> {
    Some(match family {
        "compute" => "processing resources: CPU, accelerators, inference, embedding",
        "storage" => "durable stores: objects, vectors, the knowledge graph",
        "network" => "reachability: LAN, Internet egress",
        "sensor" => "physical sensing attached to this node",
        "simulation" => "physics solvers this node can run",
        "cad" => "geometry generation and export",
        "robot" => "actuation. Never granted without a verified safety boundary",
        "radio" => "low-rate radio links",
        "bus" => "deterministic field buses",
        "world" => "ingestion and normalization of a class of world observation",
        "cyber" => "passive, defensive cyber situational awareness",
        "market" => "financial and macroeconomic data ingestion",
        "media" => "news, corpora, and the public professional source graph",
        "fusion" => "turning raw observations into corroborated evidence",
        _ => return None,
    })
}

/// Whether a capability family may only be granted with explicit human authorization.
///
/// Actuation is the obvious one. `cyber.exposure` is here because, although the data is
/// public, an operator should make a deliberate decision before their node starts
/// pulling third-party exposure records under their credentials and terms of service.
pub fn family_requires_authorization(family: &str) -> bool {
    matches!(family, "robot" | "cyber")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_has_a_description() {
        // An operator seeing `fusion.corroboration` in `archie topology` should not have
        // to leave the terminal to find out what it means.
        let mut families: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for capability in well_known::all() {
            families.insert(capability.family().to_string());
        }
        for family in &families {
            assert!(
                family_description(family).is_some(),
                "capability family {family:?} has no description"
            );
        }
        assert!(
            families.len() >= 12,
            "expected a broad taxonomy, got {families:?}"
        );
    }

    #[test]
    fn actuation_and_cyber_exposure_require_authorization() {
        // Actuation moves things. Cyber pulls third-party records under the operator's
        // own terms of service. Both are decisions a human should make on purpose.
        assert!(family_requires_authorization("robot"));
        assert!(family_requires_authorization("cyber"));
        // Observation of the physical world is not, by itself, a privileged act.
        assert!(!family_requires_authorization("world"));
        assert!(!family_requires_authorization("compute"));
        assert!(!family_requires_authorization("market"));
    }

    #[test]
    fn the_taxonomy_has_no_active_measure_capability() {
        // Wave F is passive defensive awareness. There is deliberately no capability
        // naming a scan, an exploit, or any other active measure — if one ever appears
        // in this list, it was not added by following the safety model.
        let forbidden = [
            "scan",
            "exploit",
            "attack",
            "bruteforce",
            "brute-force",
            "intrude",
            "payload",
            "weaponize",
            "c2",
            "implant",
        ];
        for capability in well_known::all() {
            let name = capability.as_str();
            for word in forbidden {
                assert!(
                    !name.contains(word),
                    "capability {name:?} names an active measure, which the safety model forbids"
                );
            }
        }
    }

    #[test]
    fn every_well_known_capability_parses() {
        // `Capability::known` panics on bad input; calling `all()` exercises all of them.
        let all = well_known::all();
        assert!(all.len() > 15);
        for cap in &all {
            assert!(Capability::parse(cap.as_str()).is_ok());
        }
    }

    #[test]
    fn requires_at_least_two_segments() {
        assert!(Capability::parse("compute").is_err());
        assert!(Capability::parse("compute.cpu").is_ok());
    }

    #[test]
    fn rejects_malformed_names() {
        for bad in [
            "",
            "Compute.Cpu",
            "compute..cpu",
            "compute.",
            ".cpu",
            "compute.c pu",
            "9compute.cpu",
        ] {
            assert!(
                Capability::parse(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn family_is_first_segment() {
        assert_eq!(Capability::known("compute.cuda").family(), "compute");
    }

    #[test]
    fn specificity_is_segment_wise() {
        let held = Capability::known("compute.cuda.sm86");
        assert!(held.satisfies(&Capability::known("compute.cuda")));
        assert!(held.satisfies(&Capability::known("compute.cuda.sm86")));
        // Prefix-but-not-segment must NOT match, or `compute.cudax` would masquerade
        // as CUDA support.
        assert!(!Capability::known("compute.cudax").satisfies(&Capability::known("compute.cuda")));
        // Less specific does not satisfy more specific.
        assert!(
            !Capability::known("compute.cuda").satisfies(&Capability::known("compute.cuda.sm86"))
        );
    }

    #[test]
    fn set_provides_honours_specificity() {
        let set: CapabilitySet = [Capability::known("compute.cuda.sm86")]
            .into_iter()
            .collect();
        assert!(set.provides(&Capability::known("compute.cuda")));
        assert!(!set.provides(&Capability::known("compute.rocm")));
    }

    #[test]
    fn set_reports_missing_requirements() {
        let set: CapabilitySet = [Capability::known("compute.cpu")].into_iter().collect();
        let cuda = Capability::known("compute.cuda");
        let cpu = Capability::known("compute.cpu");
        let missing = set.missing([&cpu, &cuda]);
        assert_eq!(missing, vec![cuda]);
    }

    #[test]
    fn set_serialization_is_ordered_and_stable() {
        let a: CapabilitySet = ["storage.vector", "compute.cpu", "network.lan"]
            .into_iter()
            .map(Capability::known)
            .collect();
        let b: CapabilitySet = ["network.lan", "compute.cpu", "storage.vector"]
            .into_iter()
            .map(Capability::known)
            .collect();
        // Insertion order must not affect the wire form, or manifest digests would
        // differ between two identical machines.
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn difference_reports_what_changed() {
        let before: CapabilitySet = ["compute.cpu", "compute.cuda"]
            .into_iter()
            .map(Capability::known)
            .collect();
        let after: CapabilitySet = ["compute.cpu"].into_iter().map(Capability::known).collect();
        assert_eq!(
            before.difference(&after),
            vec![Capability::known("compute.cuda")]
        );
        assert!(after.difference(&before).is_empty());
    }
}
