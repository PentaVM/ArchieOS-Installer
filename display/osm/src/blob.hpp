#pragma once

// Reading the outer, format-defined shell of a .osm.pbf file: a sequence of fileblocks,
// each a length-prefixed BlobHeader followed by a Blob it describes. This is the one
// place decompression happens, and the one place a corrupt or unsupported file is told
// apart from a truncated read -- both of which building.cpp needs to report honestly
// rather than silently parsing whatever garbage a bad inflate produced.
//
// Internal to archie-osm: not installed, not part of the public API.

#include <cstdint>
#include <string>
#include <vector>

namespace archie::osm::detail {

/// One decompressed fileblock: its declared type ("OSMHeader" or "OSMData") and its
/// decompressed protobuf payload, ready to be parsed as a HeaderBlock or PrimitiveBlock.
struct RawBlock {
    std::string type;
    std::vector<std::uint8_t> data;
};

/// What went wrong reading a blob, when something did. Each value names a specific,
/// checkable fact about the bytes rather than a generic "corrupt file", matching how the
/// rest of this codebase reports what it actually knows instead of what it suspects.
enum class BlobError {
    None,
    TruncatedLengthPrefix,
    TruncatedHeader,
    HeaderTooLarge,
    MalformedHeader,
    MissingDataSize,
    TruncatedBlob,
    MalformedBlob,
    UnsupportedCompression,
    DecompressionFailed,
    DecompressedSizeMismatch,
};

[[nodiscard]] const char* to_string(BlobError error) noexcept;

/// Reads one fileblock from `bytes` starting at `offset`, advancing `offset` past it.
/// The caller stops when `offset` reaches `bytes.size()` or this returns anything other
/// than `BlobError::None`.
[[nodiscard]] BlobError read_next_block(const std::vector<std::uint8_t>& bytes,
                                         std::size_t& offset, RawBlock& out);

}  // namespace archie::osm::detail
