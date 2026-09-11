#include "blob.hpp"

#include <zlib.h>

#include <optional>

#include "protobuf_reader.hpp"

namespace archie::osm::detail {
namespace {

std::uint32_t read_be32(const std::vector<std::uint8_t>& bytes, std::size_t pos) noexcept {
    return (static_cast<std::uint32_t>(bytes[pos]) << 24) |
           (static_cast<std::uint32_t>(bytes[pos + 1]) << 16) |
           (static_cast<std::uint32_t>(bytes[pos + 2]) << 8) | static_cast<std::uint32_t>(bytes[pos + 3]);
}

std::string_view as_view(const std::vector<std::uint8_t>& bytes, std::size_t offset,
                          std::size_t length) noexcept {
    return {reinterpret_cast<const char*>(bytes.data()) + offset, length};
}

// A file this format can actually produce never has a header or a blob anywhere near this
// large; refusing to trust a declared length past it turns a corrupt length field into an
// honest error instead of an attempted multi-gigabyte allocation.
constexpr std::uint64_t kMaxReasonableLength = 64ULL * 1024ULL * 1024ULL;

}  // namespace

const char* to_string(BlobError error) noexcept {
    switch (error) {
        case BlobError::None:
            return "none";
        case BlobError::TruncatedLengthPrefix:
            return "file ends mid length-prefix";
        case BlobError::TruncatedHeader:
            return "file ends mid blob header";
        case BlobError::HeaderTooLarge:
            return "a declared length is larger than any real fileblock would be";
        case BlobError::MalformedHeader:
            return "blob header is not a valid BlobHeader message";
        case BlobError::MissingDataSize:
            return "blob header did not declare a data size";
        case BlobError::TruncatedBlob:
            return "file ends before the blob its header declared";
        case BlobError::MalformedBlob:
            return "blob is not a valid Blob message";
        case BlobError::UnsupportedCompression:
            return "blob uses a compression this parser does not implement (only raw and "
                   "zlib are, which is every compression a real Geofabrik extract uses)";
        case BlobError::DecompressionFailed:
            return "zlib could not inflate the blob's data";
        case BlobError::DecompressedSizeMismatch:
            return "decompressed size did not match the size the blob declared";
    }
    return "unknown blob error";
}

BlobError read_next_block(const std::vector<std::uint8_t>& bytes, std::size_t& offset,
                           RawBlock& out) {
    if (offset + 4 > bytes.size()) {
        return BlobError::TruncatedLengthPrefix;
    }
    const std::uint32_t header_length = read_be32(bytes, offset);
    offset += 4;

    if (header_length > kMaxReasonableLength) {
        return BlobError::HeaderTooLarge;
    }
    if (offset + header_length > bytes.size()) {
        return BlobError::TruncatedHeader;
    }
    const std::string_view header_bytes = as_view(bytes, offset, header_length);
    offset += header_length;

    std::string type;
    bool have_type = false;
    std::optional<std::int64_t> datasize;
    {
        Reader reader(header_bytes);
        std::uint32_t field = 0;
        WireType wire{};
        while (reader.next_tag(field, wire)) {
            if (field == 1 && wire == WireType::LengthDelimited) {
                std::string_view value;
                if (!reader.read_bytes(value)) {
                    return BlobError::MalformedHeader;
                }
                type.assign(value);
                have_type = true;
            } else if (field == 3 && wire == WireType::Varint) {
                std::uint64_t value = 0;
                if (!reader.read_varint(value)) {
                    return BlobError::MalformedHeader;
                }
                datasize = static_cast<std::int64_t>(value);
            } else if (!reader.skip(wire)) {
                return BlobError::MalformedHeader;
            }
        }
    }
    if (!have_type) {
        return BlobError::MalformedHeader;  // `type` is a required field
    }
    if (!datasize.has_value() || *datasize < 0) {
        return BlobError::MissingDataSize;
    }

    const auto blob_length = static_cast<std::uint64_t>(*datasize);
    if (blob_length > kMaxReasonableLength) {
        return BlobError::HeaderTooLarge;
    }
    if (offset + blob_length > bytes.size()) {
        return BlobError::TruncatedBlob;
    }
    const std::string_view blob_bytes = as_view(bytes, offset, static_cast<std::size_t>(blob_length));
    offset += blob_length;

    std::optional<std::int64_t> raw_size;
    std::string_view raw_data;
    std::string_view zlib_data;
    bool has_raw = false;
    bool has_zlib = false;
    bool has_unsupported_compression = false;
    {
        Reader reader(blob_bytes);
        std::uint32_t field = 0;
        WireType wire{};
        while (reader.next_tag(field, wire)) {
            if (field == 2 && wire == WireType::Varint) {
                std::uint64_t value = 0;
                if (!reader.read_varint(value)) {
                    return BlobError::MalformedBlob;
                }
                raw_size = static_cast<std::int64_t>(value);
            } else if (field == 1 && wire == WireType::LengthDelimited) {
                if (!reader.read_bytes(raw_data)) {
                    return BlobError::MalformedBlob;
                }
                has_raw = true;
            } else if (field == 3 && wire == WireType::LengthDelimited) {
                if (!reader.read_bytes(zlib_data)) {
                    return BlobError::MalformedBlob;
                }
                has_zlib = true;
            } else if ((field == 4 || field == 5 || field == 6 || field == 7) &&
                       wire == WireType::LengthDelimited) {
                std::string_view discard;
                if (!reader.read_bytes(discard)) {
                    return BlobError::MalformedBlob;
                }
                has_unsupported_compression = true;
            } else if (!reader.skip(wire)) {
                return BlobError::MalformedBlob;
            }
        }
    }

    out.type = type;
    if (has_raw) {
        out.data.assign(raw_data.begin(), raw_data.end());
        return BlobError::None;
    }
    if (has_zlib) {
        if (!raw_size.has_value() || *raw_size < 0 ||
            static_cast<std::uint64_t>(*raw_size) > kMaxReasonableLength) {
            return BlobError::MalformedBlob;  // zlib data with no sane declared output size
        }
        out.data.assign(static_cast<std::size_t>(*raw_size), std::uint8_t{0});
        auto dest_len = static_cast<uLongf>(*raw_size);
        const int result = uncompress(out.data.data(), &dest_len,
                                       reinterpret_cast<const Bytef*>(zlib_data.data()),
                                       static_cast<uLong>(zlib_data.size()));
        if (result != Z_OK) {
            return BlobError::DecompressionFailed;
        }
        if (dest_len != static_cast<uLongf>(*raw_size)) {
            return BlobError::DecompressedSizeMismatch;
        }
        return BlobError::None;
    }
    if (has_unsupported_compression) {
        return BlobError::UnsupportedCompression;
    }
    return BlobError::MalformedBlob;  // oneof `data` had no branch set, which proto2 forbids
}

}  // namespace archie::osm::detail
