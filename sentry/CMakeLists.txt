cmake_minimum_required(VERSION 3.25)

project(sentry
        VERSION 0.1.0
        DESCRIPTION "Archie's analytical world-intelligence subsystem"
        LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
# No compiler extensions: the ISO is built on Arch with GCC, but the invariants in this
# library are pure logic and must stay portable enough to be checked anywhere.
set(CMAKE_CXX_EXTENSIONS OFF)

# Tests are OFF by default because this library is compiled inside the archiso build,
# and that build must never acquire a test dependency or spend time on one. CI turns
# them on explicitly.
option(SENTRY_BUILD_TESTS "Build Sentry's invariant tests" OFF)

# ---------------------------------------------------------------------------
# sentry-world — the contracts.
#
# Deliberately has no dependencies: no libcurl, no simdjson, no GDAL. Everything here is
# a rule about what Sentry is allowed to believe, and rules that need a network stack to
# be checked are rules that will eventually go unchecked.
# ---------------------------------------------------------------------------
add_library(sentry-world STATIC
    src/world/time.cpp
    src/world/geo.cpp
    src/world/epistemic.cpp
    src/world/provenance.cpp
    src/providers/manifest.cpp
    src/camera/authority.cpp
    src/camera/connection.cpp
    src/policy/consent.cpp
    src/identity/subject.cpp
    src/identity/relationship.cpp
    src/identity/retention.cpp
    src/policy/override.cpp
    src/identity/override.cpp
    src/identity/profile.cpp
    src/identity/wanda.cpp
    src/ingest/chat_export.cpp
    src/capture/activity.cpp
    src/capture/ingest.cpp
    src/vision/track.cpp
)

target_include_directories(sentry-world
    PUBLIC $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
)

target_compile_options(sentry-world PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
)

add_library(sentry::world ALIAS sentry-world)

# ---------------------------------------------------------------------------
# sentry-store — the only part of Sentry that touches a disk.
#
# Separate from sentry-world so the contracts stay provably I/O-free: a rule that needs a
# filesystem to be checked is a rule that eventually goes unchecked. Anything linking
# sentry::world alone still cannot open a file.
# ---------------------------------------------------------------------------
add_library(sentry-store STATIC
    src/store/graph_store.cpp
    src/store/graph_transaction.cpp
    src/store/wanda_projection.cpp
)

target_link_libraries(sentry-store PUBLIC sentry::world)

target_include_directories(sentry-store
    PUBLIC $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
)

target_compile_options(sentry-store PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
)

add_library(sentry::store ALIAS sentry-store)

# ---------------------------------------------------------------------------
# sentry-runtime — the camera worker orchestration.
#
# Pure logic: it drives a FrameSource and a PersonDetector, which are interfaces. The real
# GStreamer and ONNX implementations live behind build guards in the optional sentry-camd
# target, so this library and its tests need neither library present.
# ---------------------------------------------------------------------------
add_library(sentry-runtime STATIC
    src/runtime/camera_worker.cpp
)
target_link_libraries(sentry-runtime PUBLIC sentry::world)
target_include_directories(sentry-runtime
    PUBLIC $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
)
target_compile_options(sentry-runtime PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
)
add_library(sentry::runtime ALIAS sentry-runtime)

# ---------------------------------------------------------------------------
# sentry-camd — the camera daemon. Optional: it needs GStreamer and ONNX Runtime, so it is
# off unless both are found. The ISO build and CI, which have no camera, never build it.
# ---------------------------------------------------------------------------
option(SENTRY_WITH_GSTREAMER "Build the RTSP camera source (needs GStreamer)" OFF)
option(SENTRY_WITH_ONNX "Build the ONNX person detector (needs ONNX Runtime)" OFF)

if(SENTRY_WITH_GSTREAMER AND SENTRY_WITH_ONNX)
    find_package(PkgConfig REQUIRED)
    pkg_check_modules(GST REQUIRED IMPORTED_TARGET gstreamer-1.0 gstreamer-app-1.0)
    find_library(ONNXRUNTIME_LIB onnxruntime REQUIRED)
    find_path(ONNXRUNTIME_INCLUDE onnxruntime_cxx_api.h PATH_SUFFIXES onnxruntime)

    add_executable(sentry-camd
        src/runtime/camd_main.cpp
        src/runtime/gstreamer_source.cpp
        src/runtime/onnx_detector.cpp
    )
    target_compile_definitions(sentry-camd PRIVATE SENTRY_WITH_GSTREAMER SENTRY_WITH_ONNX)
    target_include_directories(sentry-camd PRIVATE ${ONNXRUNTIME_INCLUDE})
    target_link_libraries(sentry-camd PRIVATE sentry::runtime PkgConfig::GST ${ONNXRUNTIME_LIB})
    target_compile_options(sentry-camd PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )
    install(TARGETS sentry-camd RUNTIME DESTINATION bin)
endif()

# ---------------------------------------------------------------------------
# archie-sentry — the person graph from a terminal.
#
# The first program that links these rules. Until something did, they were a compiler's
# opinion rather than something that applies to a real file on a real machine.
# ---------------------------------------------------------------------------
add_executable(archie-sentry src/cli/main.cpp)
target_link_libraries(archie-sentry PRIVATE sentry::store)
target_compile_options(archie-sentry PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Werror>
)

install(TARGETS archie-sentry RUNTIME DESTINATION bin)

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------
if(SENTRY_BUILD_TESTS)
    enable_testing()

    add_executable(sentry-invariants tests/test_invariants.cpp)
    target_link_libraries(sentry-invariants PRIVATE sentry::world)
    target_include_directories(sentry-invariants PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-invariants PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.invariants COMMAND sentry-invariants)

    add_executable(sentry-provenance tests/test_provenance.cpp)
    target_link_libraries(sentry-provenance PRIVATE sentry::world)
    target_include_directories(sentry-provenance PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-provenance PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.provenance COMMAND sentry-provenance)

    add_executable(sentry-providers tests/test_providers.cpp)
    target_link_libraries(sentry-providers PRIVATE sentry::world)
    target_include_directories(sentry-providers PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-providers PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.providers COMMAND sentry-providers)

    add_executable(sentry-identity tests/test_identity.cpp)
    target_link_libraries(sentry-identity PRIVATE sentry::world)
    target_include_directories(sentry-identity PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-identity PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.identity COMMAND sentry-identity)

    add_executable(sentry-wanda tests/test_wanda.cpp)
    target_link_libraries(sentry-wanda PRIVATE sentry::world)
    target_include_directories(sentry-wanda PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-wanda PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.wanda COMMAND sentry-wanda)

    # Draws what Wanda decided, by calling Wanda. Built with the tests because it is a
    # diagnostic, and must never enter the ISO.
    add_executable(sentry-wanda-snapshot tools/wanda_snapshot.cpp)
    target_link_libraries(sentry-wanda-snapshot PRIVATE sentry::world)
    target_compile_options(sentry-wanda-snapshot PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_executable(sentry-retention tests/test_retention.cpp)
    target_link_libraries(sentry-retention PRIVATE sentry::world)
    target_include_directories(sentry-retention PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-retention PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.retention COMMAND sentry-retention)

    add_executable(sentry-override tests/test_override.cpp)
    target_link_libraries(sentry-override PRIVATE sentry::world)
    target_include_directories(sentry-override PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-override PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.override COMMAND sentry-override)

    add_executable(sentry-profile tests/test_profile.cpp)
    target_link_libraries(sentry-profile PRIVATE sentry::world)
    target_include_directories(sentry-profile PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-profile PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.profile COMMAND sentry-profile)

    add_executable(sentry-store-tests tests/test_store.cpp)
    target_link_libraries(sentry-store-tests PRIVATE sentry::store)
    target_include_directories(sentry-store-tests PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-store-tests PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.store COMMAND sentry-store-tests)

    add_executable(sentry-wanda-projection tests/test_wanda_projection.cpp)
    target_link_libraries(sentry-wanda-projection PRIVATE sentry::store)
    target_include_directories(sentry-wanda-projection PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-wanda-projection PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )
    add_test(NAME sentry.wanda_projection COMMAND sentry-wanda-projection)

    add_executable(sentry-lock-holder tests/lock_holder.cpp)
    target_link_libraries(sentry-lock-holder PRIVATE sentry::store)
    # Python's standard library validates real subprocess/JSON behaviour, without a
    # runtime dependency in the library or ISO. Only register when Python is available.
    find_package(Python3 COMPONENTS Interpreter QUIET)
    if(Python3_Interpreter_FOUND)
        add_test(NAME sentry.graph_cli COMMAND ${Python3_EXECUTABLE}
                 ${CMAKE_CURRENT_SOURCE_DIR}/tests/test_graph_cli.py $<TARGET_FILE:archie-sentry>)
        set_tests_properties(sentry.graph_cli PROPERTIES TIMEOUT 90)
    endif()

    add_executable(sentry-ingest tests/test_ingest.cpp)
    target_link_libraries(sentry-ingest PRIVATE sentry::world)
    target_include_directories(sentry-ingest PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-ingest PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.ingest COMMAND sentry-ingest)

    add_executable(sentry-capture tests/test_capture.cpp)
    target_link_libraries(sentry-capture PRIVATE sentry::world)
    target_include_directories(sentry-capture PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-capture PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.capture COMMAND sentry-capture)

    add_executable(sentry-capture-ingest tests/test_capture_ingest.cpp)
    target_link_libraries(sentry-capture-ingest PRIVATE sentry::world)
    target_include_directories(sentry-capture-ingest PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-capture-ingest PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.capture_ingest COMMAND sentry-capture-ingest)

    add_executable(sentry-vision tests/test_vision.cpp)
    target_link_libraries(sentry-vision PRIVATE sentry::world)
    target_include_directories(sentry-vision PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-vision PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.vision COMMAND sentry-vision)

    add_executable(sentry-connection tests/test_connection.cpp)
    target_link_libraries(sentry-connection PRIVATE sentry::world)
    target_include_directories(sentry-connection PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-connection PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.connection COMMAND sentry-connection)

    add_executable(sentry-geo tests/test_geo.cpp)
    target_link_libraries(sentry-geo PRIVATE sentry::world)
    target_include_directories(sentry-geo PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-geo PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.geo COMMAND sentry-geo)

    add_executable(sentry-camera-worker tests/test_camera_worker.cpp)
    target_link_libraries(sentry-camera-worker PRIVATE sentry::runtime)
    target_include_directories(sentry-camera-worker PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
    target_compile_options(sentry-camera-worker PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Werror>
    )

    add_test(NAME sentry.camera_worker COMMAND sentry-camera-worker)
endif()
