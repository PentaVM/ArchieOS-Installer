// A test harness with no dependencies.
//
// Deliberately not Catch2 or GoogleTest. Sentry's sources are compiled inside the
// archiso build, and a test framework pulled in by FetchContent would put a network
// fetch on the path of building a bootable image. Tests are gated off in that build,
// but the safest way to guarantee a dependency is never fetched is not to have one.
//
// Sixty lines buys: named cases, an assertion that reports the expression and the
// values, and a non-zero exit code. That is the whole contract CTest needs.

#pragma once

#include <cstdio>
#include <string>
#include <string_view>
#include <vector>

namespace sentry::test {

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
        std::printf("  %s\n    line %d: %s\n", f.test.c_str(), f.line,
                    f.expression.c_str());
        if (!f.detail.empty()) {
            std::printf("    %s\n", f.detail.c_str());
        }
    }
    return 1;
}

}  // namespace sentry::test

// Names the case every following assertion belongs to. The name is what an operator
// reads when CI goes red, so it should state the rule, not the function under test.
#define CASE(name)                                    \
    ::sentry::test::current_test() = (name);          \
    do {                                              \
    } while (false)

#define CHECK(expr)                                                    \
    do {                                                              \
        ++::sentry::test::checks_run;                                 \
        if (!(expr)) {                                                \
            ::sentry::test::record(#expr, {}, __LINE__);              \
        }                                                             \
    } while (false)

// Like CHECK, but carries a sentence explaining why the rule exists — which is the part
// worth having when the assertion fires a year from now.
#define CHECK_BECAUSE(expr, why)                                      \
    do {                                                              \
        ++::sentry::test::checks_run;                                 \
        if (!(expr)) {                                                \
            ::sentry::test::record(#expr, (why), __LINE__);           \
        }                                                             \
    } while (false)

#define CHECK_EQ(a, b)                                                 \
    do {                                                              \
        ++::sentry::test::checks_run;                                 \
        if (!((a) == (b))) {                                          \
            ::sentry::test::record(#a " == " #b, {}, __LINE__);        \
        }                                                             \
    } while (false)
