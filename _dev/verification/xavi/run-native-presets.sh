#!/usr/bin/env bash
set -euo pipefail
# $1 is the immutable PR40 snapshot, $2 is this audit-harness directory.
snapshot=$(realpath "${1:?PR40 snapshot required}")
harness=$(realpath "${2:?audit harness directory required}")
build="$snapshot/build"
mapfile -t objects < <(find "$build/CMakeFiles/archie-display.dir/src" -name '*.o' ! -name main.cpp.o)
test "${#objects[@]}" -gt 0
read -r -a qt_flags <<< "$(pkg-config --cflags Qt6Quick3D Qt6QuickControls2 Qt6Network Qt6Concurrent)"
read -r -a qt_libs <<< "$(pkg-config --libs Qt6Quick3D Qt6QuickControls2 Qt6Network Qt6Concurrent zlib)"
g++ -O0 -std=c++20 "${qt_flags[@]}" \
  -I"$snapshot/display/native/src" -I"$snapshot/display/visual/include" \
  "$harness/NativePresetReview.cpp" "${objects[@]}" \
  "$build/CMakeFiles/archie-display.dir/archie-display_autogen/mocs_compilation.cpp.o" \
  "$build/CMakeFiles/archie-display.dir/.qt/rcc/qrc_archie-native-assets.cpp.o" \
  "$build/archie-visual/libarchie-visual.a" "$build/archie-osm/libarchie-osm.a" \
  "${qt_libs[@]}" -o "$snapshot/NativePresetReview"
LC_ALL=C.UTF-8 QT_QPA_PLATFORM=offscreen "$snapshot/NativePresetReview"
