//! What GeoFabric would draw, made visible to the observatory.
//!
//! `archie-geofabric`'s selection logic is pure — no network, no clock, no filesystem —
//! but it is not a build constant: [`FabricPlan::enabling`] reads one environment
//! variable per free-with-account provider, so the plan this reports can differ between
//! two processes of the identical binary if an operator has set one up. That is a
//! feature, not noise to caching around, so this module reads the environment fresh on
//! every request rather than caching a value that is allowed to change — the ordinary
//! way an env-var-configured daemon picks up new configuration is a restart, and this
//! makes that restart actually take effect instead of serving a snapshot frozen from
//! before it happened.
//!
//! # What this is not
//!
//! The globe the renderer actually draws is a fixed Blue Marble texture; nothing here
//! streams a tile. `BandSelection` reports what [`FabricPlan::source_for`] would choose
//! at each scale — the provider-selection half of GeoFabric, working and tested — not a
//! claim that the pixels on screen came from it. Conflating the two would be exactly the
//! kind of fake photorealism the architecture forbids, so the snapshot is honest about
//! being a selection table and the display panel that reads it says so too.

use archie_geofabric::{provider, Cost, FabricPlan, GeoDataClass, GeoProviderId, PROVIDERS};
use serde::Serialize;

/// A named scale, standing in for a live camera-distance-to-LOD mapping the renderer
/// does not compute yet. Each `lod` is a real level-of-detail value GeoFabric selects
/// on — the same numbers `GeoProviderCapabilities::useful_lod` is defined against — not
/// an invented zoom percentage.
struct ScaleBand {
    name: &'static str,
    lod: u8,
}

const SCALE_BANDS: [ScaleBand; 4] = [
    ScaleBand {
        name: "planet",
        lod: 4,
    },
    ScaleBand {
        name: "region",
        lod: 10,
    },
    ScaleBand {
        name: "city",
        lod: 16,
    },
    ScaleBand {
        name: "street",
        lod: 20,
    },
];

/// What GeoFabric selects for one scale, per data class.
#[derive(Debug, Clone, Serialize)]
pub struct BandSelection {
    pub band: &'static str,
    pub lod: u8,
    pub terrain: Option<GeoProviderId>,
    pub buildings: Option<GeoProviderId>,
    pub photoreal: Option<GeoProviderId>,
}

/// A provider, as far as this display needs to say who it is and what it may be used
/// for — not the full capability record, which stays in `archie-geofabric` where the
/// rules that matter are actually enforced.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummary {
    pub id: GeoProviderId,
    pub authority: &'static str,
    pub product: &'static str,
    pub cost_class: &'static str,
    pub license: &'static str,
    pub attribution: &'static str,
    /// Why this entry is not in `enabled`. `None` for every entry that *is* enabled —
    /// there is nothing to explain about a provider that is simply on.
    ///
    /// This is the field that makes Google's absence a decision rather than a mystery.
    /// The master directive is explicit that Google may exist only as an optional
    /// display provider and that Archie must not be architected around it; a panel
    /// that just quietly never shows Google leaves an operator to guess whether that
    /// was a choice or an oversight. This says which.
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeoFabricSnapshot {
    pub schema_version: u16,
    /// Whether anything in `enabled` bills. Never true here: a free-with-account
    /// provider that widens in below is still free, and nothing ever calls
    /// [`FabricPlan::enabling_billed_source`]. Carried as a field rather than left
    /// implicit, so a renderer that ever wires in a billed provider has to change a
    /// visible fact instead of silently drifting from it.
    pub free_only: bool,
    /// Whether every account this build knows how to use is actually configured —
    /// true when nothing is left in `disabled` for a missing environment variable.
    /// Distinct from `free_only`, which stays true either way: a provider waiting on
    /// an account still costs nothing, it is just not on yet.
    pub fully_configured: bool,
    pub enabled: Vec<ProviderSummary>,
    /// Everything `archie-geofabric` knows about that is not enabled here, and why —
    /// billed providers like Google Photorealistic, and free-with-account providers
    /// whose token is not configured. Present so "not shown" and "not known about" stay
    /// visibly different things.
    pub disabled: Vec<ProviderSummary>,
    pub selection: Vec<BandSelection>,
}

fn cost_class_name(cost: Cost) -> &'static str {
    match cost {
        Cost::Owned => "owned",
        Cost::FreeAndOpen => "free_and_open",
        Cost::FreeWithAccount { .. } => "free_with_account",
        Cost::Metered { .. } => "metered",
    }
}

fn summarize(id: GeoProviderId, reason: Option<&'static str>) -> ProviderSummary {
    let capabilities = provider(id);
    ProviderSummary {
        id,
        authority: capabilities.authority,
        product: capabilities.product,
        cost_class: cost_class_name(capabilities.cost),
        license: capabilities.license,
        attribution: capabilities.attribution,
        reason,
    }
}

/// Why a provider that exists in the catalogue is not in the shipping plan.
///
/// `FabricPlan::free_only()` only says yes or no; this is the sentence that goes with
/// no. Written from the same `Cost` the plan itself switches on, so it cannot name a
/// reason the plan disagrees with.
fn exclusion_reason(cost: Cost) -> &'static str {
    match cost {
        Cost::Owned | Cost::FreeAndOpen => {
            "enabled in the shipping plan; this reason should be unreachable"
        }
        Cost::FreeWithAccount { env_var } => env_var,
        Cost::Metered { note } => note,
    }
}

/// Build the snapshot fresh. Reads no secret and touches no network: the only I/O is
/// checking whether a named environment variable is *set*, done by
/// [`FabricPlan::enabling`] itself, never by reading its value.
pub fn snapshot() -> GeoFabricSnapshot {
    let mut plan = FabricPlan::free_only();

    // Widen past the zero-config default for any account the operator has actually set
    // up. Walking `PROVIDERS` for the cost class rather than naming
    // `CesiumOsmBuildings` directly, so a second free-with-account provider added to
    // the catalogue later is picked up here without this file needing to change to
    // notice it. A provider whose account is not configured is refused by `enabling`
    // and simply stays out of the plan — `Err` is not a failure to react to, it is the
    // ordinary "not yet" case this loop exists to leave alone.
    for capabilities in &PROVIDERS {
        if matches!(capabilities.cost, Cost::FreeWithAccount { .. }) {
            if let Ok(widened) = plan.clone().enabling(capabilities.id) {
                plan = widened;
            }
        }
    }

    let enabled: Vec<ProviderSummary> = plan.enabled().map(|id| summarize(*id, None)).collect();

    let disabled: Vec<ProviderSummary> = PROVIDERS
        .iter()
        .filter(|capabilities| !plan.is_enabled(capabilities.id))
        .map(|capabilities| summarize(capabilities.id, Some(exclusion_reason(capabilities.cost))))
        .collect();

    let selection = SCALE_BANDS
        .iter()
        .map(|band| BandSelection {
            band: band.name,
            lod: band.lod,
            terrain: plan.source_for(GeoDataClass::Terrain, band.lod),
            buildings: plan.source_for(GeoDataClass::SemanticBuildings, band.lod),
            photoreal: plan.source_for(GeoDataClass::PhotorealMesh, band.lod),
        })
        .collect();

    // Only an unconfigured account leaves something disabled that isn't meant to be:
    // a metered provider is *supposed* to stay in `disabled` forever, so its presence
    // there is not a configuration gap. `fully_configured` is about the accounts, not
    // about billed providers nobody asked to enable.
    let fully_configured = !disabled
        .iter()
        .any(|entry| matches!(provider(entry.id).cost, Cost::FreeWithAccount { .. }));

    GeoFabricSnapshot {
        schema_version: 1,
        free_only: !plan.bills_anything(),
        fully_configured,
        enabled,
        disabled,
        selection,
    }
}
