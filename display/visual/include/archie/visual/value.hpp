#pragma once

// A neutral tree for a decoded projection.
//
// The bridge decodes JSON with Qt and hands the result over as one of these. That
// indirection is worth one small type: it means every rule about what a payload is
// allowed to contain — missing fields, wrong types, unknown enums, fields from a
// version of the daemon this build has never seen — can be written and checked without
// a Qt event loop, a network stack or a running daemon anywhere in sight.
//
// It is deliberately not a JSON parser. Deciding whether a byte sequence is valid JSON
// is the transport's job and Qt already does it; deciding whether a *valid* payload
// says something usable is this library's, and that is the half where a display quietly
// starts showing the wrong thing.
//
// Every accessor is total. Nothing here throws, and nothing returns a value that was
// not actually present — asking a string for its number gives you nothing, not zero.
// A display that reads a missing field as 0 draws a healthy node for a machine it knows
// nothing about, which is the failure this whole file exists to make impossible.

#include <cstddef>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace archie::visual {

class Value;

/// One key/value pair. Objects keep insertion order so that iteration is deterministic
/// and a duplicated key is visible rather than silently collapsed by a map.
struct Member;

class Value final {
public:
    enum class Kind { Null, Boolean, Number, String, Array, Object };

    Value() = default;
    explicit Value(bool value);
    explicit Value(double value);
    explicit Value(int value);
    explicit Value(std::string value);
    explicit Value(const char* value);

    static Value array(std::vector<Value> items);
    static Value object(std::vector<Member> members);

    Value(const Value&);
    Value(Value&&) noexcept;
    Value& operator=(const Value&);
    Value& operator=(Value&&) noexcept;
    ~Value();

    [[nodiscard]] Kind kind() const noexcept { return m_kind; }
    [[nodiscard]] bool is_null() const noexcept { return m_kind == Kind::Null; }

    /// Typed access. Absent when this value is of another kind — never a substitute.
    [[nodiscard]] std::optional<bool> as_boolean() const noexcept;
    [[nodiscard]] std::optional<double> as_number() const noexcept;
    [[nodiscard]] const std::string* as_string() const noexcept;
    [[nodiscard]] const std::vector<Value>* as_array() const noexcept;
    [[nodiscard]] const std::vector<Member>* as_object() const noexcept;

    /// Object lookup. Null when this is not an object or the key is absent.
    ///
    /// Returns the *first* match: a payload with a repeated key is malformed, and
    /// taking the first is the only choice that does not depend on iteration order.
    [[nodiscard]] const Value* find(std::string_view key) const noexcept;

    /// How many times `key` appears. Anything above one is a defect worth reporting.
    [[nodiscard]] int count(std::string_view key) const noexcept;

    /// Element count for arrays and objects; zero for everything else.
    [[nodiscard]] std::size_t size() const noexcept;

private:
    Kind m_kind = Kind::Null;
    bool m_boolean = false;
    double m_number = 0.0;
    std::string m_string{};
    std::vector<Value> m_array{};
    std::vector<Member> m_members{};
};

struct Member {
    std::string key;
    Value value;
};

/// Convenience for building objects in tests and in the bridge.
inline Member member(std::string key, Value value) {
    return Member{std::move(key), std::move(value)};
}

}  // namespace archie::visual
