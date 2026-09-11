// The whole parser, end to end, against a hand-built .osm.pbf: real blob framing, real
// zlib compression, real protobuf field encoding -- assembled here byte by byte rather
// than downloaded, so the fixture's content is exactly known and the file is still a
// genuine instance of the format, not a stand-in for one.
//
// A single .osm.pbf this size is impractical to fetch for a test; hand-encoding a small
// one exercises the identical decoder a multi-gigabyte Geofabrik extract would.

#include "archie/osm/building.hpp"

#include <cmath>
#include <cstdint>
#include <string>
#include <zlib.h>

#include "harness.hpp"

using namespace archie::osm;

namespace {

using Bytes = std::string;

// ---- A minimal protobuf encoder, test-only: the exact inverse of protobuf_reader.hpp,
// built independently rather than by calling anything the library under test provides,
// so a shared bug in encode/decode cannot cancel itself out. ----

void put_varint(Bytes& out, std::uint64_t value) {
    while (value >= 0x80) {
        out.push_back(static_cast<char>((value & 0x7FU) | 0x80U));
        value >>= 7;
    }
    out.push_back(static_cast<char>(value));
}

void put_tag(Bytes& out, std::uint32_t field, std::uint32_t wire) {
    put_varint(out, (static_cast<std::uint64_t>(field) << 3) | wire);
}

std::uint64_t zigzag_encode(std::int64_t value) {
    const auto raw = static_cast<std::uint64_t>(value);
    return (raw << 1) ^ static_cast<std::uint64_t>(value >> 63);
}

void put_bytes_field(Bytes& out, std::uint32_t field, std::string_view data) {
    put_tag(out, field, 2);
    put_varint(out, data.size());
    out.append(data.data(), data.size());
}

void put_varint_field(Bytes& out, std::uint32_t field, std::uint64_t value) {
    put_tag(out, field, 0);
    put_varint(out, value);
}

void put_plain_int64_field(Bytes& out, std::uint32_t field, std::int64_t value) {
    // PrimitiveBlock.lat_offset/lon_offset: a plain int64 varint, i.e. the two's
    // complement bit pattern read back as an unsigned varint -- not zigzag.
    put_varint_field(out, field, static_cast<std::uint64_t>(value));
}

void put_packed_svarints(Bytes& out, std::uint32_t field,
                          const std::vector<std::int64_t>& deltas) {
    Bytes payload;
    for (const std::int64_t v : deltas) {
        put_varint(payload, zigzag_encode(v));
    }
    put_bytes_field(out, field, payload);
}

void put_packed_varints(Bytes& out, std::uint32_t field, const std::vector<std::uint64_t>& values) {
    Bytes payload;
    for (const std::uint64_t v : values) {
        put_varint(payload, v);
    }
    put_bytes_field(out, field, payload);
}

/// Turns a list of absolute values into first-difference deltas, the encoding every
/// packed-delta field in this schema (DenseNodes id/lat/lon, Way.refs) actually uses.
std::vector<std::int64_t> to_deltas(const std::vector<std::int64_t>& absolute) {
    std::vector<std::int64_t> deltas;
    deltas.reserve(absolute.size());
    std::int64_t previous = 0;
    for (const std::int64_t v : absolute) {
        deltas.push_back(v - previous);
        previous = v;
    }
    return deltas;
}

struct RawNode {
    std::int64_t id;
    std::int64_t lat_raw;
    std::int64_t lon_raw;
};

struct RawWay {
    std::int64_t id;
    std::vector<std::int64_t> refs;
    std::vector<std::pair<std::string, std::string>> tags;
};

/// Encodes one OSMData fileblock's PrimitiveBlock: one DenseNodes group holding all the
/// nodes, then one Way per entry in `ways`, all sharing one string table built from
/// whatever keys/values the ways actually use.
Bytes encode_primitive_block(const std::vector<RawNode>& nodes, const std::vector<RawWay>& ways,
                              std::int64_t granularity, std::int64_t lat_offset,
                              std::int64_t lon_offset) {
    // String table: index 0 is the reserved blank entry every real writer includes.
    std::vector<std::string> strings{""};
    auto intern = [&strings](const std::string& s) -> std::uint64_t {
        for (std::size_t i = 0; i < strings.size(); ++i) {
            if (strings[i] == s) return i;
        }
        strings.push_back(s);
        return strings.size() - 1;
    };

    Bytes dense;
    {
        std::vector<std::int64_t> ids, lats, lons;
        for (const RawNode& n : nodes) {
            ids.push_back(n.id);
            lats.push_back(n.lat_raw);
            lons.push_back(n.lon_raw);
        }
        put_packed_svarints(dense, 1, to_deltas(ids));
        put_packed_svarints(dense, 8, to_deltas(lats));
        put_packed_svarints(dense, 9, to_deltas(lons));
    }

    Bytes group;
    put_bytes_field(group, 2, dense);  // PrimitiveGroup.dense

    for (const RawWay& way : ways) {
        Bytes way_bytes;
        put_varint_field(way_bytes, 1, static_cast<std::uint64_t>(way.id));
        std::vector<std::uint64_t> keys, vals;
        for (const auto& [key, value] : way.tags) {
            keys.push_back(intern(key));
            vals.push_back(intern(value));
        }
        if (!keys.empty()) {
            put_packed_varints(way_bytes, 2, keys);
            put_packed_varints(way_bytes, 3, vals);
        }
        put_packed_svarints(way_bytes, 8, to_deltas(way.refs));
        put_bytes_field(group, 3, way_bytes);  // PrimitiveGroup.ways
    }

    Bytes stringtable;
    for (const std::string& s : strings) {
        put_bytes_field(stringtable, 1, s);
    }

    Bytes block;
    put_bytes_field(block, 1, stringtable);
    put_bytes_field(block, 2, group);
    if (granularity != 100) {
        put_varint_field(block, 17, static_cast<std::uint64_t>(granularity));
    }
    if (lat_offset != 0) {
        put_plain_int64_field(block, 19, lat_offset);
    }
    if (lon_offset != 0) {
        put_plain_int64_field(block, 20, lon_offset);
    }
    return block;
}

Bytes encode_header_block() {
    Bytes header;
    put_bytes_field(header, 4, "OsmSchema-V0.6");
    put_bytes_field(header, 4, "DenseNodes");
    return header;
}

/// Wraps a message's bytes as an uncompressed Blob (data = raw).
Bytes wrap_blob_raw(const Bytes& payload) {
    Bytes blob;
    put_bytes_field(blob, 1, payload);
    return blob;
}

/// Wraps a message's bytes as a zlib-compressed Blob, exercising the exact decompression
/// path a real Geofabrik extract uses (every real extract is zlib, never raw).
Bytes wrap_blob_zlib(const Bytes& payload) {
    uLongf bound = compressBound(static_cast<uLong>(payload.size()));
    std::string compressed(bound, '\0');
    const int result = compress(reinterpret_cast<Bytef*>(compressed.data()), &bound,
                                 reinterpret_cast<const Bytef*>(payload.data()), payload.size());
    if (result != Z_OK) {
        std::abort();  // the test fixture itself is broken; nothing downstream can help
    }
    compressed.resize(bound);

    Bytes blob;
    put_varint_field(blob, 2, payload.size());  // raw_size
    put_bytes_field(blob, 3, compressed);        // zlib_data
    return blob;
}

void append_fileblock(Bytes& file, const std::string& type, const Bytes& blob_bytes) {
    Bytes header;
    put_bytes_field(header, 1, type);
    put_varint_field(header, 3, blob_bytes.size());

    const auto header_length = static_cast<std::uint32_t>(header.size());
    file.push_back(static_cast<char>((header_length >> 24) & 0xFFU));
    file.push_back(static_cast<char>((header_length >> 16) & 0xFFU));
    file.push_back(static_cast<char>((header_length >> 8) & 0xFFU));
    file.push_back(static_cast<char>(header_length & 0xFFU));
    file.append(header);
    file.append(blob_bytes);
}

std::vector<std::uint8_t> to_bytes(const Bytes& s) {
    return {s.begin(), s.end()};
}

bool near(double a, double b, double tolerance = 1e-9) { return std::fabs(a - b) <= tolerance; }

const Building* find_building(const ParseOutcome& outcome, std::int64_t way_id) {
    for (const Building& b : outcome.buildings) {
        if (b.way_id == way_id) return &b;
    }
    return nullptr;
}

/// Builds the full synthetic extract used by most of the cases below: an OSMHeader block
/// plus one OSMData block containing nine nodes (with a deliberately non-monotonic id 9,
/// to exercise a negative delta) and six ways covering every path resolve_building takes.
std::vector<std::uint8_t> build_primary_fixture() {
    const std::vector<RawNode> nodes = {
        {1, 100000000, 200000000}, {2, 100000000, 200001000}, {3, 100001000, 200001000},
        {4, 100001000, 200000000}, {5, 100002000, 200000000}, {6, 100002000, 200003000},
        {7, 100005000, 200003000}, {8, 100005000, 200000000}, {9, 99999000, 199999000},
    };
    const std::vector<RawWay> ways = {
        {10, {1, 2, 3, 4, 1}, {{"building", "yes"}, {"height", "12.5"}}},
        {20, {5, 6, 7, 8, 5}, {{"building", "house"}, {"building:levels", "4"}}},
        {100, {1, 2, 3, 4}, {{"building", "yes"}}},               // 4 distinct nodes, not closed
        {101, {1, 2, 999, 1}, {{"building", "yes"}}},             // node 999 does not exist
        {102, {1, 2, 3, 4, 1}, {{"building", "no"}}},             // explicit negative
        {103, {1, 2}, {{"highway", "residential"}}},              // not a building at all
    };

    Bytes file;
    append_fileblock(file, "OSMHeader", wrap_blob_raw(encode_header_block()));
    append_fileblock(file, "OSMData",
                      wrap_blob_zlib(encode_primitive_block(nodes, ways, 100, 0, 0)));
    return to_bytes(file);
}

void a_footprint_with_an_explicit_height_is_read_exactly() {
    CASE("a closed square tagged building=yes, height=12.5");
    const ParseOutcome outcome = parse_buildings(build_primary_fixture());
    CHECK_BECAUSE(outcome.ok(), outcome.error);

    const Building* building = find_building(outcome, 10);
    CHECK(building != nullptr);
    if (building == nullptr) return;

    CHECK(building->footprint.size() == 5);  // the ring, including its closing repeat
    CHECK(near(building->footprint[0].latitude, 10.0));
    CHECK(near(building->footprint[0].longitude, 20.0));
    CHECK(near(building->footprint[2].latitude, 10.0001));
    CHECK(near(building->footprint[2].longitude, 20.0001));
    CHECK_BECAUSE(
        building->footprint[0].latitude == building->footprint[4].latitude &&
            building->footprint[0].longitude == building->footprint[4].longitude,
        "the ring's declared closing node must decode to exactly the same point as its first");

    CHECK(building->height_meters.has_value());
    CHECK_BECAUSE(building->height_meters.has_value() && near(*building->height_meters, 12.5),
                  "an explicit height tag is a measurement, read verbatim");
    CHECK_BECAUSE(!building->height_is_estimated, "an explicit height tag is not an estimate");
}

void a_footprint_with_only_levels_gets_an_estimated_height() {
    CASE("building:levels=4 with no height tag estimates 4 * 3m, flagged as an estimate");
    const ParseOutcome outcome = parse_buildings(build_primary_fixture());
    const Building* building = find_building(outcome, 20);
    CHECK(building != nullptr);
    if (building == nullptr) return;

    CHECK(building->height_meters.has_value());
    CHECK_BECAUSE(building->height_meters.has_value() && near(*building->height_meters, 12.0),
                  "OSM's own documented convention: about 3 meters per level");
    CHECK_BECAUSE(building->height_is_estimated,
                  "a renderer must be able to tell a survey from a guess");
}

void an_open_way_is_not_a_footprint_however_it_is_tagged() {
    CASE("building=yes on a way whose first and last node differ");
    const ParseOutcome outcome = parse_buildings(build_primary_fixture());
    CHECK(find_building(outcome, 100) == nullptr);
    bool warned = false;
    for (const std::string& w : outcome.warnings) {
        if (w.find("100") != std::string::npos && w.find("closed") != std::string::npos) {
            warned = true;
        }
    }
    CHECK_BECAUSE(warned, "silently dropping it would look identical to the extract simply not "
                          "having this building");
}

void a_way_referencing_a_missing_node_is_reported_incomplete() {
    CASE("a footprint that crosses the edge of this extract's cutout");
    const ParseOutcome outcome = parse_buildings(build_primary_fixture());
    CHECK(find_building(outcome, 101) == nullptr);
    bool warned = false;
    for (const std::string& w : outcome.warnings) {
        if (w.find("101") != std::string::npos && w.find("not found") != std::string::npos) {
            warned = true;
        }
    }
    CHECK(warned);
}

void building_no_is_excluded_without_a_warning() {
    CASE("building=no is a deliberate, valid statement that nothing stands here");
    const ParseOutcome outcome = parse_buildings(build_primary_fixture());
    CHECK(find_building(outcome, 102) == nullptr);
    for (const std::string& w : outcome.warnings) {
        CHECK_BECAUSE(w.find("102") == std::string::npos,
                      "building=no is not an error to report, unlike an unclosed or "
                      "incomplete building=yes");
    }
}

void an_unrelated_way_is_ignored_entirely() {
    CASE("a highway is not a building and produces no footprint or complaint");
    const ParseOutcome outcome = parse_buildings(build_primary_fixture());
    CHECK(find_building(outcome, 103) == nullptr);
}

void exactly_the_expected_buildings_were_found() {
    CASE("two real footprints out of six ways: the rest are excluded for a specific reason");
    const ParseOutcome outcome = parse_buildings(build_primary_fixture());
    CHECK(outcome.buildings.size() == 2);
}

void a_nonzero_offset_and_custom_granularity_decode_correctly() {
    CASE("PrimitiveBlock.lat_offset/lon_offset are plain int64, not zigzag -- the case a "
         "backwards decode would silently get wrong");
    const std::vector<RawNode> nodes = {
        {50, 1000, 2000}, {51, 1000, 2010}, {52, 1010, 2010}, {53, 1010, 2000},
    };
    const std::vector<RawWay> ways = {
        {30, {50, 51, 52, 53, 50}, {{"building", "yes"}}},
    };
    Bytes file;
    append_fileblock(file, "OSMHeader", wrap_blob_raw(encode_header_block()));
    append_fileblock(
        file, "OSMData",
        wrap_blob_zlib(encode_primitive_block(nodes, ways, 1000, -200000000, 300000000)));

    const ParseOutcome outcome = parse_buildings(to_bytes(file));
    CHECK_BECAUSE(outcome.ok(), outcome.error);
    const Building* building = find_building(outcome, 30);
    CHECK(building != nullptr);
    if (building == nullptr) return;

    // Hand-computed from 1e-9 * (offset + granularity * raw), independently of the parser.
    CHECK(near(building->footprint[0].latitude, -0.199));
    CHECK(near(building->footprint[0].longitude, 0.302));
    CHECK(near(building->footprint[1].longitude, 0.30201));
    CHECK(near(building->footprint[2].latitude, -0.19899));

    CHECK_BECAUSE(!building->height_meters.has_value(),
                  "no height and no building:levels tag means an unknown height, never a "
                  "guessed one");
}

void an_unsupported_required_feature_is_a_fatal_error_not_a_best_effort_parse() {
    CASE("a header requiring a feature this parser never implemented");
    Bytes header;
    put_bytes_field(header, 4, "OsmSchema-V0.6");
    put_bytes_field(header, 4, "SomeFutureFeatureThisParserPredatesEntirely");

    Bytes file;
    append_fileblock(file, "OSMHeader", wrap_blob_raw(header));

    const ParseOutcome outcome = parse_buildings(to_bytes(file));
    CHECK_BECAUSE(!outcome.ok(),
                  "silently proceeding risks reading a file whose shape does not match what "
                  "this decoder assumes, which is worse than refusing it outright");
}

void a_truncated_file_is_reported_not_partially_guessed() {
    CASE("cutting the file off mid-blob must fail loudly, not return whatever decoded first");
    const std::vector<std::uint8_t> full = build_primary_fixture();
    const std::vector<std::uint8_t> truncated(full.begin(), full.begin() + full.size() / 2);
    const ParseOutcome outcome = parse_buildings(truncated);
    CHECK(!outcome.ok());
}

void an_empty_input_is_not_the_same_claim_as_a_corrupt_one() {
    CASE("zero bytes is 'nothing to read', not 'these bytes are broken'");
    const ParseOutcome outcome = parse_buildings({});
    CHECK(outcome.ok());
    CHECK(outcome.buildings.empty());
}

}  // namespace

int main() {
    a_footprint_with_an_explicit_height_is_read_exactly();
    a_footprint_with_only_levels_gets_an_estimated_height();
    an_open_way_is_not_a_footprint_however_it_is_tagged();
    a_way_referencing_a_missing_node_is_reported_incomplete();
    building_no_is_excluded_without_a_warning();
    an_unrelated_way_is_ignored_entirely();
    exactly_the_expected_buildings_were_found();
    a_nonzero_offset_and_custom_granularity_decode_correctly();
    an_unsupported_required_feature_is_a_fatal_error_not_a_best_effort_parse();
    a_truncated_file_is_reported_not_partially_guessed();
    an_empty_input_is_not_the_same_claim_as_a_corrupt_one();
    return archie::osm::test::report();
}
