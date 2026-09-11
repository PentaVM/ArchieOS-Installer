//! Which map the machine gets, and where it goes.
//!
//! The globe ships with NASA imagery, which is a picture. A vector map is a different
//! thing: buildings with heights, roads with names, coastlines that stay sharp at any
//! zoom. OpenStreetMap has it, freely, and it is far too large to put in an ISO — the
//! planet is tens of gigabytes and even one country is hundreds of megabytes. So it is
//! fetched during installation, when there is a network and a person who can say where
//! it should live.
//!
//! # Why nothing here has a size constant
//!
//! An extract is rebuilt daily and grows. A number compiled into this file would be
//! wrong within a week and would be wrong in the direction that matters: it would promise
//! a download fits when it does not. So the size is asked of the server at install time
//! and the answer is used. A machine with no network cannot be told a size, and is told
//! that instead of a guess.
//!
//! # Why this is skippable and must stay skippable
//!
//! An install with no network has to work. The map is the one part of ArchieOS that
//! cannot be made offline, so it is the one part that must be optional — and the globe
//! without it is exactly what it is today, which is not broken.

use std::path::{Path, PathBuf};

/// One downloadable region.
///
/// The URL is the whole identity. Geofabrik's layout is stable and its regions nest
/// (`south-america/chile`), so a region is named by its path and nothing here has to
/// model a hierarchy it does not use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRegion {
    /// What the person picks from a list.
    pub name: String,
    /// Where the extract comes from.
    pub url: String,
}

impl MapRegion {
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            url: url.to_string(),
        }
    }

    /// The companion checksum Geofabrik publishes beside every extract.
    ///
    /// It detects a truncated or corrupted download, which is the failure that actually
    /// happens on a domestic connection part way through four gigabytes. It is **not** a
    /// supply-chain guarantee: it comes from the same server as the file, so the trust
    /// anchor is the TLS connection to that server and nothing more. Saying so here is
    /// cheaper than someone later mistaking it for a signature.
    pub fn checksum_url(&self) -> String {
        format!("{}.md5", self.url)
    }

    /// The file name to write, taken from the URL rather than invented.
    pub fn file_name(&self) -> &str {
        self.url.rsplit('/').next().unwrap_or("map.osm.pbf")
    }
}

/// The regions offered by the wizard.
///
/// Deliberately short. A complete list of Geofabrik's several hundred extracts is a
/// menu nobody can read, and the wizard accepts a pasted URL for anything not here — so
/// this is a shortcut for the common cases, not a boundary on what is possible.
pub fn offered_regions() -> Vec<MapRegion> {
    const BASE: &str = "https://download.geofabrik.de";
    [
        ("South America", "south-america-latest.osm.pbf"),
        ("Chile", "south-america/chile-latest.osm.pbf"),
        ("Argentina", "south-america/argentina-latest.osm.pbf"),
        ("Brazil", "south-america/brazil-latest.osm.pbf"),
        ("North America", "north-america-latest.osm.pbf"),
        ("Europe", "europe-latest.osm.pbf"),
        ("Africa", "africa-latest.osm.pbf"),
        ("Asia", "asia-latest.osm.pbf"),
    ]
    .into_iter()
    .map(|(name, path)| MapRegion::new(name, &format!("{BASE}/{path}")))
    .collect()
}

/// What the person chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapChoice {
    /// No vector map. The default, and what an install with no network gets.
    Skip,
    /// Fetch this region into this directory.
    Download {
        region: MapRegion,
        /// Where the extract is written. Chosen rather than fixed, because the point of
        /// asking is that the system disk is often not where the space is.
        destination: PathBuf,
    },
}

impl Default for MapChoice {
    fn default() -> Self {
        Self::Skip
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapProblem {
    /// The destination is not an absolute path on the installed system.
    ///
    /// The person types where the map should live *on the machine they are installing*,
    /// like `/var/lib/archie/maps`. The installer prefixes the mount point itself. A
    /// relative path has no meaning here, and one under the installer's own mount point
    /// is somebody describing the seam rather than the destination.
    NotAnInstalledPath { path: PathBuf },
    /// The destination will not survive the restart.
    ///
    /// `/tmp` and `/run` are cleared at boot. Downloading four gigabytes into one is the
    /// most expensive way to end up with nothing, and it is an easy path to type.
    Volatile { path: PathBuf },
    /// There is not enough room where it was asked to go.
    NotEnoughRoom { need_bytes: u64, have_bytes: u64 },
    /// The server did not say how large the file is.
    SizeUnknown,
}

impl std::fmt::Display for MapProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnInstalledPath { path } => write!(
                f,
                "{} is not an absolute path on the installed system; give one like \
                 /var/lib/archie/maps and the installer will put it in the right place",
                path.display()
            ),
            Self::Volatile { path } => write!(
                f,
                "{} is emptied at every boot, so the map would be gone the first time \
                 the machine restarts",
                path.display()
            ),
            Self::NotEnoughRoom {
                need_bytes,
                have_bytes,
            } => write!(
                f,
                "the map needs about {} GB of room and there is {} GB",
                need_bytes / 1_000_000_000,
                have_bytes / 1_000_000_000
            ),
            Self::SizeUnknown => f.write_str(
                "the download server did not report a size, so there is no way to check \
                 it fits before starting",
            ),
        }
    }
}

/// How much room to require, given what the server said the download is.
///
/// Twice the archive. The renderer that will read this file has not been written, so how
/// large its parsed index will be is genuinely unknown — and a reservation chosen from an
/// unmeasured number should be generous in the direction that fails safely. It is a
/// deliberate over-reservation, not a measurement, and when the index exists this should
/// be replaced by what it actually costs.
pub fn room_needed(download_bytes: u64) -> u64 {
    download_bytes.saturating_mul(2)
}

/// Directories the installed system empties at boot.
const VOLATILE: [&str; 4] = ["/tmp", "/run", "/var/tmp", "/dev/shm"];

/// Whether this choice can be carried out.
///
/// `available_bytes` is the free space on the target filesystem and `download_bytes` is
/// what the server reported. Both are facts the wizard gathers; the judgement is here.
///
/// `installer_mount` is where the installer has the target mounted — `/mnt`. A
/// destination *under* it means somebody has typed the seam instead of the destination,
/// and taking it literally would produce `/mnt/mnt/...`.
pub fn assess(
    choice: &MapChoice,
    installer_mount: &Path,
    download_bytes: Option<u64>,
    available_bytes: u64,
) -> Result<(), MapProblem> {
    let MapChoice::Download { destination, .. } = choice else {
        // Skipping is always possible, and is the answer whenever anything is uncertain.
        return Ok(());
    };

    // A leading `/`, checked as text, and deliberately not `Path::is_absolute`. That
    // asks the *host* what absolute means, and on Windows the answer is "has a drive
    // letter" — so the installer would judge a path on the Linux system it is building
    // by the rules of the machine the test happens to run on, and reject
    // `/var/lib/archie/maps`. The target is Linux whatever this is compiled on.
    let text = destination.to_string_lossy();
    if !text.starts_with('/') || destination.starts_with(installer_mount) {
        return Err(MapProblem::NotAnInstalledPath {
            path: destination.clone(),
        });
    }
    if VOLATILE
        .iter()
        .any(|volatile| destination.starts_with(volatile))
    {
        return Err(MapProblem::Volatile {
            path: destination.clone(),
        });
    }

    if !safe_destination(&text) {
        return Err(MapProblem::NotAnInstalledPath {
            path: destination.clone(),
        });
    }
    let Some(bytes) = download_bytes else {
        return Err(MapProblem::SizeUnknown);
    };

    let need = room_needed(bytes);
    if need > available_bytes {
        return Err(MapProblem::NotEnoughRoom {
            need_bytes: need,
            have_bytes: available_bytes,
        });
    }
    Ok(())
}

/// A size in the units the download server and the disk both use.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "MB", "GB", "TB"];
    if bytes < 1_000_000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1_000_000.0;
    let mut unit = 1;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Extracts use the public catalog host; command metacharacters are never URLs.
pub fn safe_source(url: &str) -> bool {
    url.strip_prefix("https://download.geofabrik.de/")
        .is_some_and(|p| {
            p.ends_with(".osm.pbf")
                && !p.contains("..")
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/-._".contains(&b))
        })
}
/// Interpret installed Linux paths independently of the build host.
pub fn safe_destination(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 4096
        && path.split('/').skip(1).all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
        && !["/mnt", "/run", "/proc", "/sys", "/dev", "/tmp", "/var/tmp"]
            .iter()
            .any(|p| path == *p || path.starts_with(&format!("{p}/")))
}

/// Plain Linux block devices only; mapper/RAID stacks need a separate topology planner.
pub fn safe_device(value: &str) -> bool {
    value.strip_prefix("/dev/").is_some_and(|v| {
        !v.is_empty()
            && v.len() < 64
            && v.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

pub fn safe_mirror(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    !host.is_empty()
        && url.len() <= 2048
        && !rest.contains("..")
        && path.ends_with(".osm.pbf")
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".:-".contains(&b))
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-._".contains(&b))
}
