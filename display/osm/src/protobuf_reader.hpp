#pragma once

// The smallest protobuf wire-format reader that can decode OSM's PBF schema, and no more.
//
// OSM's binary format is protobuf-encoded, but the message shapes it uses have been fixed
// since 2010 for the sake of compatibility and never change. Pulling in a full protobuf
// runtime to decode five known message types is exactly the kind of dependency this
// codebase hand-rolls around instead of accepting -- the same judgment call as decoding a
// fixed camera-frame header by hand elsewhere in the project. This reader understands
// varints, zigzag signed integers, length-delimited fields, and how to skip a field it
// does not recognise (a requirement of protobuf's own forward-compatibility rule); it does
// not know how to encode anything, resolve extensions, or read a .proto file, because
// nothing here ever needs to.
//
// Internal to archie-osm: not installed, not part of the public API.

#include <cstdint>
#include <string_view>

namespace archie::osm::detail {

enum class WireType : std::uint32_t {
    Varint = 0,
    Fixed64 = 1,
    LengthDelimited = 2,
    Fixed32 = 5,
};

/// Reads fields out of one protobuf message's bytes, in order. Does not recurse into
/// submessages itself -- a caller wraps the bytes a length-delimited field returns in a
/// new Reader to descend, which is what keeps this type ignorant of any message's shape.
class Reader {
public:
    explicit Reader(std::string_view data) noexcept : m_data(data) {}

    [[nodiscard]] bool at_end() const noexcept { return m_pos >= m_data.size(); }

    /// Reads the next field's tag. Returns false at the end of the message, or on a wire
    /// type this schema never produces -- either way, nothing left here is decodable.
    [[nodiscard]] bool next_tag(std::uint32_t& field_number, WireType& wire_type) noexcept {
        std::uint64_t raw = 0;
        if (!read_varint(raw)) {
            return false;
        }
        field_number = static_cast<std::uint32_t>(raw >> 3);
        const auto wire = static_cast<std::uint32_t>(raw & 0x7U);
        if (wire != 0U && wire != 1U && wire != 2U && wire != 5U) {
            return false;
        }
        wire_type = static_cast<WireType>(wire);
        return true;
    }

    /// Reads a raw varint, unsigned. Correct for int32/int64/uint32/uint64/bool/enum,
    /// none of which zigzag-encode -- only sint32/sint64 do, which is `read_svarint`.
    [[nodiscard]] bool read_varint(std::uint64_t& value) noexcept {
        value = 0;
        for (int shift = 0; shift < 64; shift += 7) {
            if (m_pos >= m_data.size()) {
                return false;
            }
            const auto byte = static_cast<std::uint8_t>(m_data[m_pos]);
            ++m_pos;
            value |= static_cast<std::uint64_t>(byte & 0x7FU) << shift;
            if ((byte & 0x80U) == 0U) {
                return true;
            }
        }
        return false;  // ten bytes without a terminator: not a valid varint
    }

    /// Reads a zigzag-encoded signed varint (sint32/sint64): the wire form that keeps
    /// small negative numbers short, distinct from a plain int32/int64 varint reinterpreted
    /// as two's complement (which `lat_offset`/`lon_offset` in PrimitiveBlock actually are
    /// -- easy to get backwards, and only a hand-checked test catches it).
    [[nodiscard]] bool read_svarint(std::int64_t& value) noexcept {
        std::uint64_t raw = 0;
        if (!read_varint(raw)) {
            return false;
        }
        value = static_cast<std::int64_t>(raw >> 1);
        if ((raw & 1U) != 0U) {
            value = -value - 1;
        }
        return true;
    }

    /// Reads a length-delimited field's payload: a string, bytes, or embedded message.
    [[nodiscard]] bool read_bytes(std::string_view& out) noexcept {
        std::uint64_t length = 0;
        if (!read_varint(length) || length > m_data.size() - m_pos) {
            return false;
        }
        out = m_data.substr(m_pos, length);
        m_pos += length;
        return true;
    }

    /// Skips a field's value once its wire type is known, per protobuf's own
    /// forward-compatibility rule: an unrecognised field number must still be skippable.
    [[nodiscard]] bool skip(WireType wire_type) noexcept {
        switch (wire_type) {
            case WireType::Varint: {
                std::uint64_t discard = 0;
                return read_varint(discard);
            }
            case WireType::Fixed64:
                if (m_pos + 8 > m_data.size()) {
                    return false;
                }
                m_pos += 8;
                return true;
            case WireType::LengthDelimited: {
                std::string_view discard;
                return read_bytes(discard);
            }
            case WireType::Fixed32:
                if (m_pos + 4 > m_data.size()) {
                    return false;
                }
                m_pos += 4;
                return true;
        }
        return false;
    }

private:
    std::string_view m_data;
    std::size_t m_pos = 0;
};

}  // namespace archie::osm::detail
