cmake_minimum_required(VERSION 3.24)

project(archie-visual
        VERSION 0.1.0
        DESCRIPTION "The observatory's semantic visual state model"
        LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# ---------------------------------------------------------------------------
# archie-visual — what the observatory shows, decided without Qt.
#
# The split from `display/native` is the point of this directory, not an accident of
# layout. Everything here answers a question that has a right answer: how old is this
# reading, what may the machine afford to draw, which node is this, has the camera
# finished moving. None of those questions need a scene graph to ask or a screen to
# answer, and a decision that can only be checked by looking at a running window is a
# decision that will not be checked.
#
# So this library links no Qt, opens no socket and reads no clock. `display/native`
# depends on it; it depends on nothing. That is what makes the interesting half of the
# observatory testable on a machine with no GPU, which is where CI runs.
# ---------------------------------------------------------------------------
add_library(archie-visual STATIC
    src/budget.cpp
    src/camera.cpp
    src/cognition.cpp
    src/events.cpp
    src/geo.cpp
    src/ingest.cpp
    src/layout.cpp
    src/mesh.cpp
    src/observatory.cpp
    src/palette.cpp
    src/graph.cpp
    src/history.cpp
    src/lod.cpp
    src/scene.cpp
    src/value.cpp
)

target_include_directories(archie-visual
    PUBLIC $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
)

target_compile_options(archie-visual PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
)

add_library(archie::visual ALIAS archie-visual)

# ---------------------------------------------------------------------------
# Tests.
#
# Off by default, for the same reason Sentry's are: this library is compiled inside the
# archiso build, and that build must never spend time on a test or acquire a dependency
# to run one. CI turns them on explicitly.
# ---------------------------------------------------------------------------
# ---------------------------------------------------------------------------
# archie-visual-probe — the model's decisions, printed rather than drawn.
#
# A passing test says a rule held; it does not let anyone look at what the
# observatory would actually do. This walks a scripted scenario and prints every
# frame, which makes it both a diagnostic for a node whose display will not start
# and a way to review the model end to end without hardware that can render it.
#
# Off by default so the ISO build never pays for it. CI builds and runs it, since
# the model is deterministic and a crash or a changed decision shows up there.
# ---------------------------------------------------------------------------
option(ARCHIE_VISUAL_BUILD_TOOLS "Build the visual model's headless probe" OFF)

if(ARCHIE_VISUAL_BUILD_TOOLS)
    add_executable(archie-visual-probe tools/probe.cpp)
    target_link_libraries(archie-visual-probe PRIVATE archie-visual)
    target_compile_options(archie-visual-probe PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
    )

    # What deciding a frame costs, before anything is drawn. Prints numbers and never
    # fails: a timing threshold in CI is a flake generator on a shared runner, and the
    # figure is for a person to read against the hardware they are actually on.
    # Projects a real frame through its own camera and emits SVG. Not a screenshot of
    # the observatory -- there is no renderer to screenshot -- but a way to look at what
    # the model decided rather than reading it in a table.
    add_executable(archie-visual-render tools/render_svg.cpp)
    target_link_libraries(archie-visual-render PRIVATE archie-visual)
    target_compile_options(archie-visual-render PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
    )

    add_executable(archie-visual-bench tools/bench.cpp)
    target_link_libraries(archie-visual-bench PRIVATE archie-visual)
    target_compile_options(archie-visual-bench PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
    )

    # Runs a scripted Cerebra pass against the cognition motion engine exactly as a
    # renderer must drive it -- on input and at the frame it asks for, never on a timer --
    # and writes an animated SVG of the frames it drew, with the frame count beside what
    # a fixed 60 fps loop would have cost. Motion is the one thing `render_svg` cannot
    # show; this shows it without a GPU.
    add_executable(archie-visual-cognition tools/cognition_svg.cpp)
    target_link_libraries(archie-visual-cognition PRIVATE archie-visual)
    target_compile_options(archie-visual-cognition PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
    )
endif()

option(ARCHIE_VISUAL_BUILD_TESTS "Build the visual model's tests" OFF)

if(ARCHIE_VISUAL_BUILD_TESTS)
    enable_testing()

    set(ARCHIE_VISUAL_TESTS
        test_freshness
        test_budget
        test_cognition
        test_graph
        test_camera
        test_scene
        test_lod
        test_events
        test_ingest
        test_layout
        test_observatory
        test_palette
        test_history
        test_geo
        test_mesh
        test_world
    )

    foreach(test_name IN LISTS ARCHIE_VISUAL_TESTS)
        add_executable(${test_name} tests/${test_name}.cpp)
        target_link_libraries(${test_name} PRIVATE archie-visual)
        target_include_directories(${test_name} PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/tests)
        target_compile_options(${test_name} PRIVATE
            $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall;-Wextra;-Wpedantic;-Wswitch-enum;-Werror>
        )
        add_test(NAME ${test_name} COMMAND ${test_name})
    endforeach()
endif()
