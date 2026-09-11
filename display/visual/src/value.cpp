#include "archie/visual/value.hpp"

#include <algorithm>

namespace archie::visual {

// The special members are defined here rather than in the header on purpose: `Member`
// is an incomplete type at the point the class is declared, and defining them out of
// line is what keeps that legal without reaching for an indirection the tree does not
// otherwise need.
Value::Value(const Value&) = default;
Value::Value(Value&&) noexcept = default;
Value& Value::operator=(const Value&) = default;
Value& Value::operator=(Value&&) noexcept = default;
Value::~Value() = default;

Value::Value(bool value) : m_kind(Kind::Boolean), m_boolean(value) {}
Value::Value(double value) : m_kind(Kind::Number), m_number(value) {}
Value::Value(int value) : m_kind(Kind::Number), m_number(static_cast<double>(value)) {}
Value::Value(std::string value) : m_kind(Kind::String), m_string(std::move(value)) {}
Value::Value(const char* value) : m_kind(Kind::String), m_string(value == nullptr ? "" : value) {}

Value Value::array(std::vector<Value> items) {
    Value value{};
    value.m_kind = Kind::Array;
    value.m_array = std::move(items);
    return value;
}

Value Value::object(std::vector<Member> members) {
    Value value{};
    value.m_kind = Kind::Object;
    value.m_members = std::move(members);
    return value;
}

std::optional<bool> Value::as_boolean() const noexcept {
    if (m_kind != Kind::Boolean) {
        return std::nullopt;
    }
    return m_boolean;
}

std::optional<double> Value::as_number() const noexcept {
    if (m_kind != Kind::Number) {
        // Deliberately not parsing a numeric string. A projection that sends "3" where
        // it promised 3 has a bug, and quietly accepting it means the bug ships and the
        // next value it sends in the wrong shape is read as something else entirely.
        return std::nullopt;
    }
    return m_number;
}

const std::string* Value::as_string() const noexcept {
    return m_kind == Kind::String ? &m_string : nullptr;
}

const std::vector<Value>* Value::as_array() const noexcept {
    return m_kind == Kind::Array ? &m_array : nullptr;
}

const std::vector<Member>* Value::as_object() const noexcept {
    return m_kind == Kind::Object ? &m_members : nullptr;
}

const Value* Value::find(std::string_view key) const noexcept {
    if (m_kind != Kind::Object) {
        return nullptr;
    }
    const auto it = std::find_if(m_members.begin(), m_members.end(),
                                 [&](const Member& m) { return m.key == key; });
    return it == m_members.end() ? nullptr : &it->value;
}

int Value::count(std::string_view key) const noexcept {
    if (m_kind != Kind::Object) {
        return 0;
    }
    return static_cast<int>(
        std::count_if(m_members.begin(), m_members.end(),
                      [&](const Member& m) { return m.key == key; }));
}

std::size_t Value::size() const noexcept {
    switch (m_kind) {
        case Kind::Array:
            return m_array.size();
        case Kind::Object:
            return m_members.size();
        case Kind::Null:
        case Kind::Boolean:
        case Kind::Number:
        case Kind::String:
            return 0;
    }
    return 0;
}

}  // namespace archie::visual
