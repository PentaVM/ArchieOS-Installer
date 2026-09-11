// A test harness with no dependencies.
//
// The same reasoning as `sentry/tests/harness.hpp`, for the same reason: these sources
// are compiled inside the archiso build, and a framework pulled in by FetchContent
// would put a network fetch on the path of building a bootable image. Tests are gated
// off in that build, but the surest way to guarantee a dependency is never fetched is
// not to have one.
//
// It is duplicated rather than shared because the display must not depend on Sentry.
// Sixty lines is a cheaper price than a link between two subsystems that have no other
// reason to know about each other.

#pragma once

#include <cstdio>
#include <string>
#include <string_view>
#include <vector>

namespace archie::visual::test {

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
    failures().push_back(
        Failure{current_test(), std::string{expression}, std::move(detail), line});
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

}  // namespace archie::visual::test

// Names the case every following assertion belongs to. The name is what an operator
// reads when CI goes red, so it should state the rule, not the function under test.
#define CASE(name)                                  \
    ::archie::visual::test::current_test() = (name); \
    do {                                            \
    } while (false)

#define CHECK(expr)                                             \
    do {                                                        \
        ++::archie::visual::test::checks_run;                   \
        if (!(expr)) {                                          \
            ::archie::visual::test::record(#expr, {}, __LINE__); \
        }                                                       \
    } while (false)

// Like CHECK, but carries a sentence explaining why the rule exists — which is the part
// worth having when the assertion fires a year from now.
#define CHECK_BECAUSE(expr, why)                                   \
    do {                                                           \
        ++::archie::visual::test::checks_run;                      \
        if (!(expr)) {                                             \
            ::archie::visual::test::record(#expr, (why), __LINE__); \
        }                                                          \
    } while (false)

#define CHECK_EQ(a, b)                                                  \
    do {                                                                \
        ++::archie::visual::test::checks_run;                           \
        if (!((a) == (b))) {                                            \
            ::archie::visual::test::record(#a " == " #b, {}, __LINE__); \
        }                                                               \
    } while (false)
