#pragma once

// A test harness with no dependencies.
//
// The same reasoning as `display/visual/tests/harness.hpp`, for the same reason: pulling
// in a framework by FetchContent puts a network fetch on the path of building this
// library, and the surest way to guarantee that never happens is not to have the
// dependency at all. Duplicated rather than shared, because archie-osm and archie-visual
// have no other reason to know about each other's test code.

#include <cstdio>
#include <string>
#include <string_view>
#include <vector>

namespace archie::osm::test {

struct Failure {
    std::string test;
    std::string expression;
    std::string detail;
    int line = 0;
};

inline std::vector<Failure>& failures() {
    static std::vector<Failure> f;
    return f;
}

inline std::string& current_test() {
    static std::string name;
    return name;
}

inline int checks_run = 0;

inline void record(std::string_view expression, std::string detail, int line) {
    failures().push_back(Failure{current_test(), std::string{expression}, std::move(detail), line});
}

// Runs every registered case and reports. Returns a process exit code.
inline int report() {
    if (failures().empty()) {
        std::printf("ok — %d checks passed\n", checks_run);
        return 0;
    }
    std::printf("FAILED — %zu of %d checks\n\n", failures().size(), checks_run);
    for (const auto& f : failures()) {
        std::printf("  %s\n    line %d: %s\n", f.test.c_str(), f.line, f.expression.c_str());
        if (!f.detail.empty()) {
            std::printf("    %s\n", f.detail.c_str());
        }
    }
    return 1;
}

}  // namespace archie::osm::test

#define CASE(name)                               \
    ::archie::osm::test::current_test() = (name); \
    do {                                         \
    } while (false)

#define CHECK(expr)                                       \
    do {                                                  \
        ++::archie::osm::test::checks_run;                \
        if (!(expr)) {                                    \
            ::archie::osm::test::record(#expr, {}, __LINE__); \
        }                                                 \
    } while (false)

#define CHECK_BECAUSE(expr, why)                              \
    do {                                                      \
        ++::archie::osm::test::checks_run;                    \
        if (!(expr)) {                                        \
            ::archie::osm::test::record(#expr, (why), __LINE__); \
        }                                                      \
    } while (false)

#define CHECK_EQ(a, b)                                             \
    do {                                                           \
        ++::archie::osm::test::checks_run;                         \
        if (!((a) == (b))) {                                       \
            ::archie::osm::test::record(#a " == " #b, {}, __LINE__); \
        }                                                          \
    } while (false)
