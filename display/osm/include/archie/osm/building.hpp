#pragma once

// Building footprints, out of a real .osm.pbf extract.
//
// This is the other half of OSM building extrusion, kept apart from `archie::visual` on
// purpose: reading an actual map extract needs zlib to decompress it and a
// protobuf-shaped decoder to read it, and `archie::visual` promises to depend on nothing
// so it stays testable on a machine with no GPU. This library carries that dependency
// instead, and hands back plain geographic footprints -- no scene geometry, no meters, no
// mesh -- which is `archie::visual::mesh`'s job once a later stage decides where to place
// them on the globe.
//
// The parser only ever hand-rolls exactly as much of protobuf and OSM's schema as
// buildings need: dense nodes, plain nodes, ways, and the tags on both. It does not read
// relations, changesets, or anything from an OSM history file, because nothing here needs
// them yet -- a multipolygon building relation is a real gap this leaves, not a silent one.

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

#include "archie/visual/geo.hpp"

namespace archie::osm {

/// A building way, exactly as tagged in the source data.
struct Building {
    std::int64_t way_id = 0;

    /// The footprint's ground ring, in the order the way listed its nodes. A well-formed
    /// building way repeats its first node as its last to close the ring; this keeps that
    /// repeat rather than stripping it, since it is what the source data actually says.
    std::vector<archie::visual::GeoPoint> footprint;

    /// Height in meters, when the source data actually states or estimates one.
    /// `std::nullopt` when neither a `height` nor a `building:levels` tag was present --
    /// a caller decides what an unknown-height building looks like; this parser never
    /// invents a number to fill the field.
    std::optional<double> height_meters;

    /// True when `height_meters` came from `building:levels * 3m`, OSM's own documented
    /// convention for an approximate height, rather than from an explicit `height` tag. A
    /// renderer that wants to say so on screen -- rather than presenting an estimate as a
    /// survey -- needs this bit.
    bool height_is_estimated = false;
};

/// Everything a parse run produced, and everything about the source data it could not
/// honestly resolve. A run that hits a genuine format error still keeps whatever
/// buildings it had already decoded, because a corrupt tail block should not discard
/// footprints that were already fully read.
struct ParseOutcome {
    std::vector<Building> buildings;

    /// Notes about specific, non-fatal problems: a way whose node references were not
    /// found in the extract (common at the edge of a regional cutout, where a way can
    /// reference a node the cutout did not include), a footprint that was not actually
    /// closed, and so on. Not a substitute for `error` -- a clean parse of an extract with
    /// no buildings in it produces zero of both.
    std::vector<std::string> warnings;

    /// Set when the parse could not continue: a truncated file, a block that is not valid
    /// protobuf, a compression this parser does not implement, or a required feature the
    /// header declares that this parser does not know. Empty when the parse reached the
    /// end of the file.
    std::string error;

    [[nodiscard]] bool ok() const noexcept { return error.empty(); }
};

/// Parses every building footprint out of a `.osm.pbf` extract's raw bytes.
///
/// A "building" is any closed way tagged `building=*` with a value other than the
/// explicit negative OSM itself defines (`building=no`). A way that is not closed (its
/// first and last node differ) is reported as a warning and excluded: an open way is not
/// a footprint, whatever it is tagged.
[[nodiscard]] ParseOutcome parse_buildings(const std::vector<std::uint8_t>& pbf_bytes);

}  // namespace archie::osm
