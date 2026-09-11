cmake_minimum_required(VERSION 3.24)

project(archie-osm
        VERSION 0.1.0
        DESCRIPTION "Reading real OpenStreetMap building footprints out of a .osm.pbf extract"
        LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# ---------------------------------------------------------------------------
# archie-osm — the half of building extrusion that touches a real map file.
#
# `archie-visual` promises to depend on nothing, so it can never be the place that reads a
# compressed, protobuf-encoded extract. This library carries exactly the two dependencies
# that promise excludes -- zlib, to inflate a blob, and a hand-rolled protobuf-wire-format
# decoder scoped to OSM's fixed schema, because pulling in a full protobuf runtime to read
# five message shapes that have not changed since 2010 would be a much larger dependency
# for no more correctness. It hands back plain geographic footprints; turning those into
# scene geometry is `archie::visual::mesh`'s job, one layer away from zlib entirely.
# ---------------------------------------------------------------------------

if(NOT TARGET archie::visual)
    add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../visual archie-visual)
endif()

find_package(ZLIB REQUIRED)

add_library(archie-osm STATIC
    src/blob.cpp
    src/building.cpp
)

target_include_directories(archie-osm
    PUBLIC $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
)

target_link_libraries(archie-osm
    PUBLIC archie::visual
    PRIVATE ZLIB::ZLIB
)

target_compile_options(archie-osm PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
)

add_library(archie::osm ALIAS archie-osm)

# ---------------------------------------------------------------------------
# Tests.
#
# Off by default, for the same reason as archie-visual's: nothing in the archiso build
# should acquire a test dependency or spend time running one. CI turns them on explicitly.
# ---------------------------------------------------------------------------
option(ARCHIE_OSM_BUILD_TESTS "Build archie-osm's tests" OFF)

if(ARCHIE_OSM_BUILD_TESTS)
    enable_testing()

    set(ARCHIE_OSM_TESTS
        test_protobuf
        test_building
    )

    foreach(test_name IN LISTS ARCHIE_OSM_TESTS)
        add_executable(${test_name} tests/${test_name}.cpp)
        target_link_libraries(${test_name} PRIVATE archie-osm)
        target_include_directories(${test_name} PRIVATE
            ${CMAKE_CURRENT_SOURCE_DIR}/tests
            ${CMAKE_CURRENT_SOURCE_DIR}/src
        )
        target_compile_options(${test_name} PRIVATE
            $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
        )
        add_test(NAME ${test_name} COMMAND ${test_name})
    endforeach()
endif()
