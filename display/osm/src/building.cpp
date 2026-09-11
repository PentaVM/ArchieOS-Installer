#include "archie/osm/building.hpp"

#include <cmath>
#include <cstdlib>
#include <functional>
#include <unordered_map>
#include <utility>

#include "blob.hpp"
#include "protobuf_reader.hpp"

namespace archie::osm {
namespace {

using archie::visual::GeoPoint;
using detail::Reader;
using detail::WireType;

std::string_view as_view(const std::vector<std::uint8_t>& bytes) noexcept {
    return {reinterpret_cast<const char*>(bytes.data()), bytes.size()};
}

/// A way collected while scanning a PrimitiveBlock, before it is known whether it is a
/// building. Tags are already resolved to strings against *that block's own* string
/// table, since the table is not global -- resolving later against the wrong block's
/// table would silently pair a key with an unrelated value.
struct PendingWay {
    std::int64_t way_id = 0;
    std::vector<std::int64_t> node_refs;
    std::vector<std::pair<std::string, std::string>> tags;
};

/// Parses the leading decimal number in an OSM tag value, e.g. "12", "12.5", or "12 m"
/// (the unit-suffixed form the wiki deprecates but real extracts still contain). Returns
/// nothing for a value with no numeric prefix at all, rather than guessing zero.
std::optional<double> leading_number(const std::string& text) {
    if (text.empty()) {
        return std::nullopt;
    }
    const char* begin = text.c_str();
    char* end = nullptr;
    const double value = std::strtod(begin, &end);
    if (end == begin || !std::isfinite(value)) {
        return std::nullopt;
    }
    return value;
}

[[nodiscard]] bool read_packed_varints(std::string_view bytes, std::vector<std::uint64_t>& out) {
    Reader reader(bytes);
    while (!reader.at_end()) {
        std::uint64_t value = 0;
        if (!reader.read_varint(value)) {
            return false;
        }
        out.push_back(value);
    }
    return true;
}

[[nodiscard]] bool read_packed_svarints(std::string_view bytes, std::vector<std::int64_t>& out) {
    Reader reader(bytes);
    while (!reader.at_end()) {
        std::int64_t value = 0;
        if (!reader.read_svarint(value)) {
            return false;
        }
        out.push_back(value);
    }
    return true;
}

[[nodiscard]] bool header_features_are_supported(const std::vector<std::uint8_t>& data,
                                                  std::string& error) {
    Reader reader(as_view(data));
    std::uint32_t field = 0;
    WireType wire{};
    while (reader.next_tag(field, wire)) {
        if (field == 4 && wire == WireType::LengthDelimited) {  // required_features
            std::string_view feature;
            if (!reader.read_bytes(feature)) {
                error = "malformed HeaderBlock";
                return false;
            }
            // The only two features any real-world writer (osmium, osmconvert, Osmosis)
            // sets as required. Anything else names a schema variant this hand-rolled
            // decoder was never checked against, and silently proceeding risks reading a
            // file whose shape does not match what is assumed below.
            if (feature != "OsmSchema-V0.6" && feature != "DenseNodes") {
                error = "extract requires an unsupported feature: " + std::string(feature);
                return false;
            }
        } else if (!reader.skip(wire)) {
            error = "malformed HeaderBlock";
            return false;
        }
    }
    return true;
}

[[nodiscard]] bool parse_dense_nodes(std::string_view dense_bytes, std::int64_t granularity,
                                      std::int64_t lat_offset, std::int64_t lon_offset,
                                      std::unordered_map<std::int64_t, GeoPoint>& node_positions,
                                      std::string& error) {
    Reader reader(dense_bytes);
    std::uint32_t field = 0;
    WireType wire{};
    std::vector<std::int64_t> id_deltas;
    std::vector<std::int64_t> lat_deltas;
    std::vector<std::int64_t> lon_deltas;
    while (reader.next_tag(field, wire)) {
        if (field == 1 && wire == WireType::LengthDelimited) {
            std::string_view payload;
            if (!reader.read_bytes(payload) || !read_packed_svarints(payload, id_deltas)) {
                error = "malformed DenseNodes.id";
                return false;
            }
        } else if (field == 8 && wire == WireType::LengthDelimited) {
            std::string_view payload;
            if (!reader.read_bytes(payload) || !read_packed_svarints(payload, lat_deltas)) {
                error = "malformed DenseNodes.lat";
                return false;
            }
        } else if (field == 9 && wire == WireType::LengthDelimited) {
            std::string_view payload;
            if (!reader.read_bytes(payload) || !read_packed_svarints(payload, lon_deltas)) {
                error = "malformed DenseNodes.lon";
                return false;
            }
        } else if (!reader.skip(wire)) {
            error = "malformed DenseNodes";
            return false;
        }
    }
    if (id_deltas.size() != lat_deltas.size() || id_deltas.size() != lon_deltas.size()) {
        error = "DenseNodes id/lat/lon columns have different lengths";
        return false;
    }

    std::int64_t id = 0;
    std::int64_t lat = 0;
    std::int64_t lon = 0;
    for (std::size_t i = 0; i < id_deltas.size(); ++i) {
        id += id_deltas[i];
        lat += lat_deltas[i];
        lon += lon_deltas[i];
        GeoPoint point;
        point.latitude = 1e-9 * static_cast<double>(lat_offset + granularity * lat);
        point.longitude = 1e-9 * static_cast<double>(lon_offset + granularity * lon);
        node_positions[id] = point;
    }
    return true;
}

[[nodiscard]] bool parse_plain_node(std::string_view node_bytes, std::int64_t granularity,
                                     std::int64_t lat_offset, std::int64_t lon_offset,
                                     std::unordered_map<std::int64_t, GeoPoint>& node_positions,
                                     std::string& error) {
    Reader reader(node_bytes);
    std::uint32_t field = 0;
    WireType wire{};
    std::optional<std::int64_t> id;
    std::optional<std::int64_t> lat;
    std::optional<std::int64_t> lon;
    while (reader.next_tag(field, wire)) {
        if (field == 1 && wire == WireType::Varint) {
            std::int64_t value = 0;
            if (!reader.read_svarint(value)) {
                error = "malformed Node";
                return false;
            }
            id = value;
        } else if (field == 8 && wire == WireType::Varint) {
            std::int64_t value = 0;
            if (!reader.read_svarint(value)) {
                error = "malformed Node";
                return false;
            }
            lat = value;
        } else if (field == 9 && wire == WireType::Varint) {
            std::int64_t value = 0;
            if (!reader.read_svarint(value)) {
                error = "malformed Node";
                return false;
            }
            lon = value;
        } else if (!reader.skip(wire)) {
            error = "malformed Node";
            return false;
        }
    }
    if (!id.has_value() || !lat.has_value() || !lon.has_value()) {
        error = "Node message is missing a required field";
        return false;
    }
    GeoPoint point;
    point.latitude = 1e-9 * static_cast<double>(lat_offset + granularity * (*lat));
    point.longitude = 1e-9 * static_cast<double>(lon_offset + granularity * (*lon));
    node_positions[*id] = point;
    return true;
}

[[nodiscard]] bool parse_way(std::string_view way_bytes,
                              const std::function<const std::string&(std::uint64_t)>& string_at,
                              PendingWay& out, std::string& error) {
    Reader reader(way_bytes);
    std::uint32_t field = 0;
    WireType wire{};
    std::optional<std::int64_t> id;
    std::vector<std::uint64_t> keys;
    std::vector<std::uint64_t> vals;
    while (reader.next_tag(field, wire)) {
        if (field == 1 && wire == WireType::Varint) {
            std::uint64_t value = 0;
            if (!reader.read_varint(value)) {
                error = "malformed Way";
                return false;
            }
            id = static_cast<std::int64_t>(value);
        } else if (field == 2 && wire == WireType::LengthDelimited) {
            std::string_view payload;
            if (!reader.read_bytes(payload) || !read_packed_varints(payload, keys)) {
                error = "malformed Way.keys";
                return false;
            }
        } else if (field == 3 && wire == WireType::LengthDelimited) {
            std::string_view payload;
            if (!reader.read_bytes(payload) || !read_packed_varints(payload, vals)) {
                error = "malformed Way.vals";
                return false;
            }
        } else if (field == 8 && wire == WireType::LengthDelimited) {
            std::string_view payload;
            std::vector<std::int64_t> deltas;
            if (!reader.read_bytes(payload) || !read_packed_svarints(payload, deltas)) {
                error = "malformed Way.refs";
                return false;
            }
            std::int64_t ref = 0;
            out.node_refs.reserve(deltas.size());
            for (const std::int64_t delta : deltas) {
                ref += delta;
                out.node_refs.push_back(ref);
            }
        } else if (!reader.skip(wire)) {
            error = "malformed Way";
            return false;
        }
    }
    if (!id.has_value()) {
        error = "Way message is missing its required id";
        return false;
    }
    if (keys.size() != vals.size()) {
        error = "Way keys/vals columns have different lengths";
        return false;
    }
    out.way_id = *id;
    out.tags.reserve(keys.size());
    for (std::size_t i = 0; i < keys.size(); ++i) {
        out.tags.emplace_back(string_at(keys[i]), string_at(vals[i]));
    }
    return true;
}

[[nodiscard]] bool parse_primitive_group(
    std::string_view group_bytes, std::int64_t granularity, std::int64_t lat_offset,
    std::int64_t lon_offset, const std::function<const std::string&(std::uint64_t)>& string_at,
    std::unordered_map<std::int64_t, GeoPoint>& node_positions,
    std::vector<PendingWay>& pending_ways, std::string& error) {
    Reader reader(group_bytes);
    std::uint32_t field = 0;
    WireType wire{};
    while (reader.next_tag(field, wire)) {
        if (field == 1 && wire == WireType::LengthDelimited) {  // plain Node (rare in practice)
            std::string_view node_bytes;
            if (!reader.read_bytes(node_bytes) ||
                !parse_plain_node(node_bytes, granularity, lat_offset, lon_offset, node_positions,
                                   error)) {
                return false;
            }
        } else if (field == 2 && wire == WireType::LengthDelimited) {  // dense
            std::string_view dense_bytes;
            if (!reader.read_bytes(dense_bytes) ||
                !parse_dense_nodes(dense_bytes, granularity, lat_offset, lon_offset,
                                    node_positions, error)) {
                return false;
            }
        } else if (field == 3 && wire == WireType::LengthDelimited) {  // ways
            std::string_view way_bytes;
            PendingWay way;
            if (!reader.read_bytes(way_bytes) || !parse_way(way_bytes, string_at, way, error)) {
                return false;
            }
            pending_ways.push_back(std::move(way));
        } else if (!reader.skip(wire)) {
            error = "malformed PrimitiveGroup";
            return false;
        }
    }
    return true;
}

[[nodiscard]] bool parse_primitive_block(const std::vector<std::uint8_t>& data,
                                          std::unordered_map<std::int64_t, GeoPoint>& node_positions,
                                          std::vector<PendingWay>& pending_ways,
                                          std::string& error) {
    Reader reader(as_view(data));
    std::uint32_t field = 0;
    WireType wire{};

    std::string_view stringtable_bytes;
    bool have_stringtable = false;
    std::vector<std::string_view> groups;
    std::int64_t granularity = 100;
    std::int64_t lat_offset = 0;
    std::int64_t lon_offset = 0;

    while (reader.next_tag(field, wire)) {
        if (field == 1 && wire == WireType::LengthDelimited) {
            if (!reader.read_bytes(stringtable_bytes)) {
                error = "malformed PrimitiveBlock";
                return false;
            }
            have_stringtable = true;
        } else if (field == 2 && wire == WireType::LengthDelimited) {
            std::string_view group;
            if (!reader.read_bytes(group)) {
                error = "malformed PrimitiveBlock";
                return false;
            }
            groups.push_back(group);
        } else if (field == 17 && wire == WireType::Varint) {
            std::uint64_t value = 0;
            if (!reader.read_varint(value)) {
                error = "malformed PrimitiveBlock";
                return false;
            }
            granularity = static_cast<std::int64_t>(value);
        } else if (field == 19 && wire == WireType::Varint) {
            std::uint64_t value = 0;
            if (!reader.read_varint(value)) {
                error = "malformed PrimitiveBlock";
                return false;
            }
            // PrimitiveBlock.lat_offset is a plain `int64`, not an `sint64`: the raw
            // varint is the two's-complement value directly, with no zigzag decode. Every
            // real extract ships this as zero, which is exactly why getting it backwards
            // would go unnoticed without a test built around a non-zero offset.
            lat_offset = static_cast<std::int64_t>(value);
        } else if (field == 20 && wire == WireType::Varint) {
            std::uint64_t value = 0;
            if (!reader.read_varint(value)) {
                error = "malformed PrimitiveBlock";
                return false;
            }
            lon_offset = static_cast<std::int64_t>(value);
        } else if (!reader.skip(wire)) {
            error = "malformed PrimitiveBlock";
            return false;
        }
    }
    if (!have_stringtable) {
        error = "PrimitiveBlock has no string table";
        return false;
    }

    std::vector<std::string> strings;
    {
        Reader st_reader(stringtable_bytes);
        std::uint32_t st_field = 0;
        WireType st_wire{};
        while (st_reader.next_tag(st_field, st_wire)) {
            if (st_field == 1 && st_wire == WireType::LengthDelimited) {
                std::string_view value;
                if (!st_reader.read_bytes(value)) {
                    error = "malformed StringTable";
                    return false;
                }
                strings.emplace_back(value);
            } else if (!st_reader.skip(st_wire)) {
                error = "malformed StringTable";
                return false;
            }
        }
    }

    const auto string_at = [&strings](std::uint64_t index) -> const std::string& {
        static const std::string kEmpty;
        return index < strings.size() ? strings[index] : kEmpty;
    };

    for (const std::string_view group : groups) {
        if (!parse_primitive_group(group, granularity, lat_offset, lon_offset, string_at,
                                    node_positions, pending_ways, error)) {
            return false;
        }
    }
    return true;
}

void resolve_building(const PendingWay& way,
                       const std::unordered_map<std::int64_t, GeoPoint>& node_positions,
                       ParseOutcome& outcome) {
    std::optional<std::string> building_value;
    std::optional<double> explicit_height;
    std::optional<double> levels;
    for (const auto& [key, value] : way.tags) {
        if (key == "building") {
            building_value = value;
        } else if (key == "height") {
            explicit_height = leading_number(value);
        } else if (key == "building:levels") {
            levels = leading_number(value);
        }
    }

    if (!building_value.has_value() || *building_value == "no") {
        return;  // not a building footprint
    }
    if (way.node_refs.size() < 4) {  // 3 distinct corners plus the repeated closing node
        outcome.warnings.push_back("way " + std::to_string(way.way_id) +
                                    " is tagged building but has too few nodes to be a footprint");
        return;
    }
    if (way.node_refs.front() != way.node_refs.back()) {
        outcome.warnings.push_back("way " + std::to_string(way.way_id) +
                                    " is tagged building but is not a closed ring");
        return;
    }

    Building building;
    building.way_id = way.way_id;
    building.footprint.reserve(way.node_refs.size());
    std::size_t missing = 0;
    for (const std::int64_t ref : way.node_refs) {
        const auto it = node_positions.find(ref);
        if (it == node_positions.end()) {
            ++missing;
            continue;
        }
        building.footprint.push_back(it->second);
    }
    if (missing > 0) {
        outcome.warnings.push_back(
            "way " + std::to_string(way.way_id) + ": " + std::to_string(missing) + " of " +
            std::to_string(way.node_refs.size()) +
            " node references were not found in this extract (likely a footprint that "
            "crosses the edge of a regional cutout); the footprint is incomplete and was "
            "not included");
        return;
    }

    if (explicit_height.has_value() && *explicit_height > 0.0) {
        building.height_meters = explicit_height;
        building.height_is_estimated = false;
    } else if (levels.has_value() && *levels > 0.0) {
        // OSM's own documented convention for an approximate height where none was
        // surveyed: roughly 3 meters per level. Flagged as an estimate, never presented
        // to a caller as a measurement.
        building.height_meters = *levels * 3.0;
        building.height_is_estimated = true;
    }

    outcome.buildings.push_back(std::move(building));
}

}  // namespace

ParseOutcome parse_buildings(const std::vector<std::uint8_t>& pbf_bytes) {
    ParseOutcome outcome;
    std::unordered_map<std::int64_t, GeoPoint> node_positions;
    std::vector<PendingWay> pending_ways;

    std::size_t offset = 0;
    bool seen_header = false;
    while (offset < pbf_bytes.size()) {
        detail::RawBlock block;
        const detail::BlobError block_error = detail::read_next_block(pbf_bytes, offset, block);
        if (block_error != detail::BlobError::None) {
            outcome.error = std::string("reading fileblock: ") + detail::to_string(block_error);
            return outcome;
        }

        if (block.type == "OSMHeader") {
            if (seen_header) {
                outcome.error = "a second OSMHeader block appeared after the first";
                return outcome;
            }
            seen_header = true;
            if (!header_features_are_supported(block.data, outcome.error)) {
                return outcome;
            }
        } else if (block.type == "OSMData") {
            if (!seen_header) {
                outcome.error = "an OSMData block appeared before any OSMHeader block";
                return outcome;
            }
            if (!parse_primitive_block(block.data, node_positions, pending_ways, outcome.error)) {
                return outcome;
            }
        } else {
            outcome.warnings.push_back("skipped a fileblock of unrecognised type '" + block.type +
                                        "'");
        }
    }

    for (const PendingWay& way : pending_ways) {
        resolve_building(way, node_positions, outcome);
    }
    return outcome;
}

}  // namespace archie::osm
