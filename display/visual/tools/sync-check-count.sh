#!/usr/bin/env bash
# Run every suite and write the measured total into the documents that quote it.
#
# It exists because I twice put a hand-added figure into files whose whole job is to say
# what is actually true. The number is small and nobody would have noticed, which is
# exactly why it is worth taking the opportunity to be wrong away rather than resolving
# to concentrate harder.
#
# Usage: display/visual/tools/sync-check-count.sh [--check]
#   --check verifies the documents match without editing them, for CI.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
visual="$(dirname "$here")"
repo="$(cd "$visual/../.." && pwd)"
# A directory beside the repository rather than `mktemp -d`, with a scratch directory the
# compiler will actually use pointed at it.
#
# The second part is the one that matters, and the obvious guess is wrong: MinGW's gcc
# ignores `TMPDIR` and reads `TMP`/`TEMP`, and it wants a Windows path, so exporting a
# POSIX `TMPDIR` is no better than leaving it unset. Either way gcc falls back to
# `C:\WINDOWS` and dies on permission before compiling anything — and the tool that
# exists to stop a wrong number reaching the documents could not run on the machine
# somebody was editing them on. Measured, not assumed: with everything unset, only `TMP`
# and `TEMP` holding a `D:\...`-style path let a compile through.
#
# So all three are set. Linux gcc reads `TMPDIR` and the other two are harmless there;
# Windows gcc reads `TMP`/`TEMP` and ignores the first.
#
# The pid keeps two concurrent runs apart; `rm -rf` on exit keeps neither around.
build="$repo/.sync-check-$$"
rm -rf "$build"
mkdir -p "$build"
export TMPDIR="$build"
if command -v cygpath >/dev/null 2>&1; then
    TMP="$(cygpath -w "$build")"
else
    TMP="$build"
fi
export TMP
export TEMP="$TMP"
trap 'rm -rf "$build"' EXIT

flags=(-std=c++20 -Wall -Wextra -Wpedantic -Wswitch-enum -Werror -I"$visual/include")

for source in "$visual"/src/*.cpp; do
    g++ "${flags[@]}" -c "$source" -o "$build/$(basename "${source%.cpp}").o"
done
ar rcs "$build/libvisual.a" "$build"/*.o

total=0
suites=0
for test in "$visual"/tests/test_*.cpp; do
    name="$(basename "${test%.cpp}")"
    g++ "${flags[@]}" -I"$visual/tests" "$test" "$build/libvisual.a" -o "$build/$name"
    output="$("$build/$name")"
    count="$(printf '%s' "$output" | grep -o '[0-9]* checks' | cut -d' ' -f1)"
    total=$((total + count))
    suites=$((suites + 1))
done

phrase="$suites suites, $total checks"
echo "measured: $phrase"

documents=(
    "$repo/docs/FUNCTIONS.md"
    "$repo/docs/ARCHITECTURE.md"
    "$repo/docs/ROADMAP.md"
    "$visual/README.md"
)

status=0
for document in "${documents[@]}"; do
    if grep -q "$phrase" "$document"; then
        continue
    fi
    if [ "${1:-}" = "--check" ]; then
        echo "stale: $document does not say '$phrase'" >&2
        grep -o '[0-9]* suites, [0-9]* checks' "$document" | head -1 >&2
        status=1
    else
        # Rewrite whatever figure is there, whatever it says.
        perl -pi -e "s/\\d+ suites, \\d+ checks/$phrase/" "$document"
        echo "updated: $document"
    fi
done
exit $status
