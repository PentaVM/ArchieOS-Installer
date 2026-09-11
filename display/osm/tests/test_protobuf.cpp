// The wire-format reader, checked against byte sequences hand-encoded from the protobuf
// spec rather than round-tripped through the reader's own inverse -- a bug that flips the
// same sign in both an encoder and a decoder built together would otherwise cancel out
// and pass silently.
//
// Every fixture is bound to a named `std::string` before a `Reader` is built over it:
// `Reader` stores a non-owning `string_view`, so a `Reader` built directly over a
// temporary would be left pointing at freed memory the moment the constructor returns.

#include "../src/protobuf_reader.hpp"

#include <string>

#include "harness.hpp"

using namespace archie::osm::detail;

namespace {

std::string bytes(std::initializer_list<unsigned char> values) {
    return std::string(values.begin(), values.end());
}

void a_single_byte_varint_is_itself() {
    CASE("values under 128 fit in one byte with no continuation bit");
    const std::string zero = bytes({0x00});
    Reader reader(zero);
    std::uint64_t value = 0;
    CHECK(reader.read_varint(value));
    CHECK(value == 0);

    const std::string max_single_byte = bytes({0x7F});
    Reader reader2(max_single_byte);
    CHECK(reader2.read_varint(value));
    CHECK(value == 127);
}

void a_multi_byte_varint_matches_a_hand_encoding() {
    CASE("300, hand-encoded from the spec's own worked example");
    // 300 = 0b1_0010_1100. Low 7 bits (0101100 = 0x2C) go in byte 0 with the
    // continuation bit set (0xAC); the remaining bits (10 = 0x02) go in byte 1.
    const std::string data = bytes({0xAC, 0x02});
    Reader reader(data);
    std::uint64_t value = 0;
    CHECK(reader.read_varint(value));
    CHECK(value == 300);
}

void zigzag_decoding_matches_the_known_table() {
    CASE("sint64's zigzag mapping: 0,1,2,3,4 decode to 0,-1,1,-2,2");
    const std::pair<unsigned char, std::int64_t> table[] = {
        {0, 0}, {1, -1}, {2, 1}, {3, -2}, {4, 2},
    };
    for (const auto& [raw, expected] : table) {
        const std::string data = bytes({raw});
        Reader reader(data);
        std::int64_t value = 0;
        CHECK(reader.read_svarint(value));
        CHECK_BECAUSE(value == expected,
                      "this is the exact table from the protobuf spec; a decoder that gets "
                      "any row wrong will silently mis-place every negative coordinate");
    }
}

void a_plain_varint_and_a_zigzag_varint_of_the_same_byte_differ() {
    CASE("byte 0x01 means 1 as a plain varint and -1 as a zigzag one");
    const std::string data = bytes({0x01});

    Reader plain(data);
    std::uint64_t plain_value = 0;
    CHECK(plain.read_varint(plain_value));
    CHECK(plain_value == 1);

    Reader zigzag(data);
    std::int64_t zigzag_value = 0;
    CHECK(zigzag.read_svarint(zigzag_value));
    CHECK_BECAUSE(zigzag_value == -1,
                  "PrimitiveBlock.lat_offset is plain int64 while Node.lat is sint64; reading "
                  "one with the other's decoder silently produces a wrong but plausible number");
}

void a_tag_splits_into_field_number_and_wire_type() {
    CASE("a tag byte for field 1, length-delimited");
    // (1 << 3) | 2 = 0x0A
    const std::string data = bytes({0x0A});
    Reader reader(data);
    std::uint32_t field = 0;
    WireType wire{};
    CHECK(reader.next_tag(field, wire));
    CHECK(field == 1);
    CHECK(wire == WireType::LengthDelimited);
}

void length_delimited_reads_exactly_its_declared_span() {
    CASE("a length-prefixed string reads only its own bytes, nothing past them");
    const std::string data = bytes({0x05, 'h', 'e', 'l', 'l', 'o', 0x21});  // trailing byte
    Reader reader(data);
    std::string_view value;
    CHECK(reader.read_bytes(value));
    CHECK(value == "hello");
    CHECK_BECAUSE(!reader.at_end(), "the trailing byte after the string must still be there to read");
}

void skip_moves_past_a_field_of_any_wire_type() {
    CASE("an unrecognised field must still be skippable, per protobuf's own compatibility rule");
    // field 5 varint (300) followed by field 1 length-delimited ("ok").
    const std::string data = bytes({(5 << 3) | 0, 0xAC, 0x02, (1 << 3) | 2, 0x02, 'o', 'k'});
    Reader reader(data);
    std::uint32_t field = 0;
    WireType wire{};
    CHECK(reader.next_tag(field, wire));
    CHECK(field == 5);
    CHECK(reader.skip(wire));  // never decoded field 5's value directly
    CHECK(reader.next_tag(field, wire));
    CHECK(field == 1);
    std::string_view value;
    CHECK(reader.read_bytes(value));
    CHECK(value == "ok");
}

void a_truncated_varint_is_reported_not_guessed() {
    CASE("a continuation bit with nothing after it is a truncated file, not a value");
    const std::string data = bytes({0x80});
    Reader reader(data);
    std::uint64_t value = 0;
    CHECK_BECAUSE(!reader.read_varint(value),
                  "a byte that says 'more follows' with no more bytes must fail, not return "
                  "whatever partial bits it had accumulated");
}

void reading_past_the_end_fails_cleanly() {
    CASE("a length-delimited field claiming more bytes than remain is rejected");
    const std::string data = bytes({0x0A, 'h', 'i'});  // claims length 10, only 2 bytes follow
    Reader reader(data);
    std::string_view value;
    CHECK(!reader.read_bytes(value));
}

}  // namespace

int main() {
    a_single_byte_varint_is_itself();
    a_multi_byte_varint_matches_a_hand_encoding();
    zigzag_decoding_matches_the_known_table();
    a_plain_varint_and_a_zigzag_varint_of_the_same_byte_differ();
    a_tag_splits_into_field_number_and_wire_type();
    length_delimited_reads_exactly_its_declared_span();
    skip_moves_past_a_field_of_any_wire_type();
    a_truncated_varint_is_reported_not_guessed();
    reading_past_the_end_fails_cleanly();
    return archie::osm::test::report();
}
