//! The layers the globe is allowed to draw, and where each one comes from.
//!
//! Every layer here was fetched and checked on the date recorded against it. None of these
//! endpoints is a guess: an invented URL produces a globe that renders a plausible nothing,
//! which is worse than a globe with a missing layer, because the second one says so.
//!
//! # Free, and it stays free
//!
//! The constraint on this catalogue is not a preference. Nothing in it may cost money —
//! not now, and not by a later edit that pastes in a nicer-looking tile server. So the
//! constraint is data: [`LayerAccess`] has no billed variant to select, and [`EXCLUDED`]
//! names the sources that were considered and rejected, with the host that identifies each
//! one, so a test can refuse an endpoint that reaches for one.
//!
//! A free account key is a different thing from a bill, and one layer needs one. It is off
//! until the key exists, because a globe that silently drops a layer teaches you to
//! distrust the whole display.
//!
//! # Refresh floors
//!
//! Each provider states how often it is willing to be asked, and asking faster is how a
//! keyless service stops being keyless. CelesTrak recomputes every two hours and firewalls
//! addresses that poll harder; OpenSky gives an anonymous client four hundred credits a
//! day and charges four for a global query, which is a fetch every fifteen minutes and no
//! more. Those numbers are here rather than in a scheduler, because the scheduler is not
//! where somebody will look when the requests start failing.

use serde::{Deserialize, Serialize};

/// A layer of the world view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerId {
    UsgsEarthquakes,
    OpenSkyAircraft,
    AdsbLolAircraft,
    CelestrakSatellites,
    NwsAlerts,
    GdacsAlerts,
    NasaBlackMarble,
    NasaFirmsHotspots,
}

/// What it takes to read a layer.
///
/// There is deliberately no variant for a paid tier. A source that requires payment cannot
/// be expressed here, so adding one is a change to this enum and not a change to a table
/// somebody skims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerAccess {
    /// Anonymous. Nothing to register, nothing to store, nothing to expire.
    Keyless,
    /// A free account key, read from this environment variable. Free as in no invoice —
    /// the layer is off until the variable is set.
    FreeCredential { env_var: &'static str },
}

impl LayerAccess {
    pub const fn needs_a_key(self) -> bool {
        matches!(self, Self::FreeCredential { .. })
    }
}

/// What the layer draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    /// Discrete events with a position: quakes, fires, alerts.
    PointObservations,
    /// Moving objects with a heading: aircraft.
    Tracks,
    /// Orbital elements, propagated locally rather than fetched per position.
    Ephemeris,
    /// Raster tiles for the surface.
    TileImagery,
}

/// Where on Earth a layer has anything to say.
///
/// Stated because a globe that draws a national feed without saying so implies the rest of
/// the world is quiet, and a quiet Chile is not the same claim as an unobserved Chile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Global,
    /// Limited to a named area, in plain words.
    Regional(&'static str),
}

/// One source, as it was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WorldLayer {
    pub id: LayerId,
    /// Who publishes it.
    pub authority: &'static str,
    /// What it is, in the words a person would use.
    pub dataset: &'static str,
    /// The URL, with `{braced}` placeholders for anything supplied per request.
    pub endpoint: &'static str,
    pub access: LayerAccess,
    pub kind: LayerKind,
    pub coverage: Coverage,
    /// The fastest this provider is willing to be asked, from its own documentation.
    pub min_refresh_seconds: u32,
    /// Required on the display wherever the layer is drawn.
    pub attribution: &'static str,
    /// Whether the provider requires a client to identify itself in `User-Agent`.
    pub identifies_the_client: bool,
    /// The date this endpoint was last actually fetched, ISO 8601.
    pub verified_on: &'static str,
}

impl WorldLayer {
    /// Whether the layer can run as the system ships.
    ///
    /// A keyless layer works out of the box. A layer needing a key does not, and pretending
    /// otherwise produces a globe that is quietly missing something.
    pub const fn available_without_setup(&self) -> bool {
        matches!(self.access, LayerAccess::Keyless)
    }

    /// What to tell somebody who expected this layer and does not have it.
    pub fn setup_required(&self) -> Option<String> {
        match self.access {
            LayerAccess::Keyless => None,
            LayerAccess::FreeCredential { env_var } => Some(format!(
                "{} needs a free {} key in {env_var}; no account, no layer",
                self.dataset, self.authority
            )),
        }
    }
}

/// Sources that were considered and are not here, and the host that gives each away.
///
/// Kept as data so it can be enforced. The rule Nelson set is that nothing may be billed,
/// and the way that rule breaks is not somebody deciding to spend money — it is a tile URL
/// pasted in during a rendering problem because it looked better.
pub const EXCLUDED: [(&str, &str); 5] = [
    (
        "tile.googleapis.com",
        "Google Photorealistic 3D Tiles: billed per request after a free tier",
    ),
    (
        "services.sentinel-hub.com",
        "Sentinel Hub: free tier is trial-limited, then billed by processing unit",
    ),
    (
        "api.mapbox.com",
        "Mapbox: billed per map load beyond the free allowance",
    ),
    (
        "maps.googleapis.com",
        "Google Maps Platform: billed per request, and requires a billing account to enable at all",
    ),
    (
        "api.cesium.com",
        "Cesium ion: hosted asset tiles are billed by streaming volume; Cesium Native itself is used locally instead",
    ),
];

/// Every layer, as verified.
pub const LAYERS: [WorldLayer; 8] = [
    WorldLayer {
        id: LayerId::UsgsEarthquakes,
        authority: "U.S. Geological Survey",
        dataset: "Earthquakes, past hour",
        endpoint: "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_hour.geojson",
        access: LayerAccess::Keyless,
        kind: LayerKind::PointObservations,
        coverage: Coverage::Global,
        // The feed itself is rebuilt each minute; asking faster only re-reads a file.
        min_refresh_seconds: 60,
        attribution: "U.S. Geological Survey, Earthquake Hazards Program",
        identifies_the_client: true,
        verified_on: "2026-09-09",
    },
    WorldLayer {
        id: LayerId::OpenSkyAircraft,
        authority: "The OpenSky Network",
        dataset: "Live aircraft state vectors",
        endpoint: "https://opensky-network.org/api/states/all",
        access: LayerAccess::Keyless,
        kind: LayerKind::Tracks,
        coverage: Coverage::Global,
        // Anonymous access is 400 credits a day per endpoint and a global query costs
        // four of them. That is 100 fetches a day, so no faster than every 15 minutes.
        min_refresh_seconds: 900,
        attribution: "The OpenSky Network, https://opensky-network.org",
        identifies_the_client: true,
        verified_on: "2026-09-09",
    },
    WorldLayer {
        id: LayerId::AdsbLolAircraft,
        authority: "adsb.lol",
        dataset: "Community ADS-B aircraft near a point",
        endpoint: "https://api.adsb.lol/v2/lat/{latitude}/lon/{longitude}/dist/{radius_nm}",
        access: LayerAccess::Keyless,
        kind: LayerKind::Tracks,
        coverage: Coverage::Global,
        // No published figure; the service states its limits move with load. Ten seconds
        // is the rate its own map refreshes at, and slower is always safe.
        min_refresh_seconds: 10,
        attribution: "adsb.lol community feeders",
        identifies_the_client: true,
        verified_on: "2026-09-09",
    },
    WorldLayer {
        id: LayerId::CelestrakSatellites,
        authority: "CelesTrak",
        dataset: "Orbital elements by satellite group",
        endpoint: "https://celestrak.org/NORAD/elements/gp.php?GROUP={group}&FORMAT=JSON",
        access: LayerAccess::Keyless,
        kind: LayerKind::Ephemeris,
        coverage: Coverage::Global,
        // CelesTrak recomputes every two hours and firewalls addresses that poll harder.
        // Positions come from propagating these locally, not from asking again.
        min_refresh_seconds: 7_200,
        attribution: "CelesTrak, Dr. T.S. Kelso",
        identifies_the_client: true,
        verified_on: "2026-09-09",
    },
    WorldLayer {
        id: LayerId::NwsAlerts,
        authority: "NOAA National Weather Service",
        dataset: "Active weather alerts",
        endpoint: "https://api.weather.gov/alerts/active",
        access: LayerAccess::Keyless,
        kind: LayerKind::PointObservations,
        // Explicitly regional: drawing it without saying so implies the rest of the world
        // has no weather worth warning about.
        coverage: Coverage::Regional("the United States and its territories"),
        min_refresh_seconds: 60,
        attribution: "NOAA/National Weather Service",
        // The NWS asks every client to identify itself and rate-limits those that do not.
        identifies_the_client: true,
        verified_on: "2026-09-09",
    },
    WorldLayer {
        id: LayerId::GdacsAlerts,
        authority: "Global Disaster Alert and Coordination System",
        dataset: "Current disaster alerts",
        endpoint: "https://www.gdacs.org/xml/rss.xml",
        access: LayerAccess::Keyless,
        kind: LayerKind::PointObservations,
        coverage: Coverage::Global,
        min_refresh_seconds: 300,
        attribution: "GDACS, European Commission and United Nations",
        identifies_the_client: true,
        verified_on: "2026-09-09",
    },
    WorldLayer {
        id: LayerId::NasaBlackMarble,
        authority: "NASA Global Imagery Browse Services",
        dataset: "VIIRS Black Marble night lights",
        endpoint: "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_Black_Marble/default/{date}/GoogleMapsCompatible_Level8/{z}/{y}/{x}.png",
        access: LayerAccess::Keyless,
        kind: LayerKind::TileImagery,
        coverage: Coverage::Global,
        // A night-lights composite; the imagery changes on the order of a day at most.
        min_refresh_seconds: 86_400,
        attribution: "NASA Earth Observing System Data and Information System (EOSDIS)",
        identifies_the_client: true,
        verified_on: "2026-09-09",
    },
    WorldLayer {
        id: LayerId::NasaFirmsHotspots,
        authority: "NASA FIRMS",
        dataset: "Active fire and thermal hotspots",
        endpoint:
            "https://firms.modaps.eosdis.nasa.gov/api/area/csv/{map_key}/{source}/{area}/{days}",
        // Free, and still a key: the layer stays dark until somebody registers for one.
        access: LayerAccess::FreeCredential {
            env_var: "ARCHIE_FIRMS_MAP_KEY",
        },
        kind: LayerKind::PointObservations,
        coverage: Coverage::Global,
        // The key allows 5000 transactions per ten minutes; the detections themselves
        // arrive with each satellite overpass, so minutes is the meaningful unit.
        min_refresh_seconds: 600,
        attribution: "NASA FIRMS, MODIS and VIIRS active fire data",
        identifies_the_client: true,
        verified_on: "2026-09-09",
    },
];

/// Look a layer up by id.
pub fn layer(id: LayerId) -> &'static WorldLayer {
    LAYERS
        .iter()
        .find(|layer| layer.id == id)
        // Every variant is in the table, and `every_layer_id_has_an_entry` holds it there.
        .expect("every layer id has a catalogue entry")
}

/// The layers a fresh install can actually draw.
pub fn available_without_setup() -> impl Iterator<Item = &'static WorldLayer> {
    LAYERS
        .iter()
        .filter(|layer| layer.available_without_setup())
}

/// Whether a URL reaches a source that was excluded, and which one.
///
/// Host-based rather than substring-based, so a legitimate URL that merely mentions a
/// vendor's name in a query parameter is not refused.
pub fn excluded_source(url: &str) -> Option<&'static str> {
    let host = host_of(url)?;
    EXCLUDED
        .iter()
        .find(|(excluded_host, _)| host == *excluded_host)
        .map(|(_, reason)| *reason)
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    // Strip any userinfo and port; neither identifies the source.
    let host = authority.rsplit('@').next().unwrap_or(authority);
    Some(host.split(':').next().unwrap_or(host))
}
