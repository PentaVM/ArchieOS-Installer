//! Which map, and where it lands.
//!
//! The map is the one part of ArchieOS that cannot ship offline, so it is the one part
//! that can fail at install time for reasons outside the machine. Everything that decides
//! whether it *can* be fetched is here, and none of it needs a network.

use archie_installer::mapdata::{
    assess, human_bytes, offered_regions, room_needed, MapChoice, MapProblem, MapRegion,
};
use std::path::{Path, PathBuf};

fn region() -> MapRegion {
    MapRegion::new(
        "Chile",
        "https://download.geofabrik.de/south-america/chile-latest.osm.pbf",
    )
}

fn download_to(path: &str) -> MapChoice {
    MapChoice::Download {
        region: region(),
        destination: PathBuf::from(path),
    }
}

/// An install with no network must still produce a working machine.
///
/// The map is the only thing here that needs one, so skipping has to be a real answer
/// rather than a degraded install — and it must never be blocked by a check about space
/// or size for a download that is not happening.
#[test]
fn skipping_the_map_is_always_possible() {
    assert_eq!(
        assess(&MapChoice::Skip, Path::new("/mnt"), None, 0),
        Ok(()),
        "no network, no room, no size — and skipping is still fine"
    );
    assert_eq!(MapChoice::default(), MapChoice::Skip);
}

/// The person names a path on the machine they are installing, and the installer puts
/// it in the right place. Typing the installer's own mount point means describing the
/// seam, and taking it literally would produce `/mnt/mnt/...`.
#[test]
fn a_path_that_is_not_on_the_installed_system_is_refused() {
    for bad in ["maps", "../maps", "/mnt/var/lib/archie/maps"] {
        let problem = assess(
            &download_to(bad),
            Path::new("/mnt"),
            Some(346_000_000),
            900_000_000_000,
        )
        .unwrap_err();
        assert!(
            matches!(problem, MapProblem::NotAnInstalledPath { .. }),
            "{bad} -> {problem:?}"
        );
        assert!(
            problem.to_string().contains("/var/lib/archie/maps"),
            "the refusal has to show what a good answer looks like: {problem}"
        );
    }

    // An ordinary installed path is exactly what is wanted.
    assert_eq!(
        assess(
            &download_to("/var/lib/archie/maps"),
            Path::new("/mnt"),
            Some(346_000_000),
            900_000_000_000
        ),
        Ok(())
    );
}

/// The most expensive way to end up with nothing.
#[test]
fn a_destination_that_is_emptied_at_boot_is_refused() {
    for volatile in ["/tmp/maps", "/run/maps", "/var/tmp/maps", "/dev/shm/maps"] {
        let problem = assess(
            &download_to(volatile),
            Path::new("/mnt"),
            Some(4_116_000_000),
            900_000_000_000,
        )
        .unwrap_err();
        assert!(
            matches!(problem, MapProblem::Volatile { .. }),
            "{volatile} survives a reboot? {problem:?}"
        );
        assert!(problem.to_string().contains("restarts"), "{problem}");
    }

    // A path is volatile because of where it *starts*, not because the word appears in
    // it somewhere. Matching anywhere in the string would refuse every one of these, and
    // they are all perfectly good places to put a map.
    for fine in [
        "/var/lib/archie/maps",
        "/var/lib/archie/tmp-maps",
        "/home/tmp-archive/maps",
        "/srv/run-data/maps",
        "/opt/shm-cache/maps",
    ] {
        assert_eq!(
            assess(
                &download_to(fine),
                Path::new("/mnt"),
                Some(346_000_000),
                900_000_000_000
            ),
            Ok(()),
            "{fine} does not start with a directory the system empties"
        );
    }
}

/// Checked before the download starts, not four gigabytes into it.
#[test]
fn a_download_that_will_not_fit_is_refused_before_it_begins() {
    let problem = assess(
        &download_to("/var/lib/archie/maps"),
        Path::new("/mnt"),
        Some(4_116_000_000),
        5_000_000_000,
    )
    .unwrap_err();

    match problem {
        MapProblem::NotEnoughRoom {
            need_bytes,
            have_bytes,
        } => {
            assert_eq!(have_bytes, 5_000_000_000);
            assert!(
                need_bytes > 4_116_000_000,
                "the archive is not the only thing that has to fit"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// The reservation is deliberately larger than the file.
///
/// The renderer that will read the extract has not been written, so the size of its
/// parsed index is unknown. An unmeasured number should be generous in the direction that
/// fails safely: refusing an install that would have fit is recoverable, and filling the
/// disk of a machine somebody just installed is not.
#[test]
fn the_reservation_is_bigger_than_the_archive_and_says_so() {
    assert!(room_needed(1_000_000_000) > 1_000_000_000);
    assert_eq!(room_needed(346_000_000), 692_000_000);
    // Never wraps, whatever a server claims.
    assert!(room_needed(u64::MAX) >= u64::MAX / 2);
}

/// A server that will not say how big the file is cannot be checked against the disk.
#[test]
fn an_unknown_size_is_refused_rather_than_assumed() {
    assert_eq!(
        assess(
            &download_to("/var/lib/archie/maps"),
            Path::new("/mnt"),
            None,
            900_000_000_000
        ),
        Err(MapProblem::SizeUnknown),
        "a size nobody reported must not be treated as a size that fits"
    );
}

/// The file name comes from the URL, so a pasted region is named by what it is.
#[test]
fn the_written_file_is_named_after_what_was_downloaded() {
    assert_eq!(region().file_name(), "chile-latest.osm.pbf");
    assert_eq!(
        MapRegion::new(
            "Europe",
            "https://download.geofabrik.de/europe-latest.osm.pbf"
        )
        .file_name(),
        "europe-latest.osm.pbf"
    );
    // A URL with no path still produces something writable rather than an empty name.
    assert!(!MapRegion::new("odd", "https://example.invalid")
        .file_name()
        .is_empty());
}

/// The checksum sits beside the file, which is the convention the source publishes.
#[test]
fn the_checksum_is_fetched_from_beside_the_archive() {
    assert_eq!(
        region().checksum_url(),
        "https://download.geofabrik.de/south-america/chile-latest.osm.pbf.md5"
    );
}

/// A menu nobody can read is a menu people pick wrongly from.
#[test]
fn the_offered_regions_are_few_and_real() {
    let regions = offered_regions();
    assert!(
        (4..=12).contains(&regions.len()),
        "a shortcut for the common cases, not a copy of the whole catalogue"
    );
    for region in &regions {
        assert!(region.url.starts_with("https://"), "{}", region.url);
        assert!(region.url.ends_with(".osm.pbf"), "{}", region.url);
        assert!(!region.name.is_empty());
    }
}

/// Sizes are shown the way a download and a disk are both measured.
#[test]
fn sizes_read_the_way_a_person_would_say_them() {
    assert_eq!(human_bytes(346_000_000), "346.0 MB");
    assert_eq!(human_bytes(4_116_000_000), "4.1 GB");
    assert_eq!(human_bytes(512), "512 B");
}
