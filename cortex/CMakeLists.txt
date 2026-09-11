cmake_minimum_required(VERSION 3.24)

project(archie-cortex LANGUAGES CXX)

option(ARCHIE_CORTEX_BUILD_TESTS "Build Archie Cortex contract tests" ON)
option(ARCHIE_CORTEX_WITH_ONNX "Enable the optional ONNX Runtime inference backend" OFF)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_library(archie-cortex-core STATIC
    src/contracts.cpp
    src/ipc.cpp
    src/intelligence.cpp
)
target_include_directories(archie-cortex-core PUBLIC include)
target_compile_options(archie-cortex-core PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall -Wextra -Wpedantic -Wswitch-enum -Werror>
)

# Model execution remains optional until a measured exported model exists. The core
# contract has no runtime dependency on a model library, and deterministic retrieval is
# always present as a safe fallback.
if(ARCHIE_CORTEX_WITH_ONNX)
    find_package(onnxruntime CONFIG REQUIRED)
    target_link_libraries(archie-cortex-core PUBLIC onnxruntime::onnxruntime)
    target_compile_definitions(archie-cortex-core PUBLIC ARCHIE_CORTEX_WITH_ONNX=1)
endif()

add_executable(archie-cortexd src/main.cpp)
target_link_libraries(archie-cortexd PRIVATE archie-cortex-core)
if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    target_compile_definitions(archie-cortexd PRIVATE _GNU_SOURCE)
endif()
target_compile_options(archie-cortexd PRIVATE
    $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall -Wextra -Wpedantic -Wswitch-enum -Werror>
)

if(ARCHIE_CORTEX_BUILD_TESTS)
    enable_testing()
    add_executable(archie-cortex-contracts tests/test_contracts.cpp)
    target_link_libraries(archie-cortex-contracts PRIVATE archie-cortex-core)
    target_compile_options(archie-cortex-contracts PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall -Wextra -Wpedantic -Wswitch-enum -Werror>
    )
    add_test(NAME archie-cortex-contracts COMMAND archie-cortex-contracts)
    add_executable(archie-cortex-intelligence tests/test_intelligence.cpp)
    target_link_libraries(archie-cortex-intelligence PRIVATE archie-cortex-core)
    target_compile_definitions(archie-cortex-intelligence PRIVATE ARCHIE_CORTEX_FIXTURES="${CMAKE_CURRENT_SOURCE_DIR}/../protocols/fixtures")
    add_test(NAME archie-cortex-intelligence COMMAND archie-cortex-intelligence)
    add_executable(archie-cortex-benchmark tests/benchmark_retrieval.cpp)
    target_link_libraries(archie-cortex-benchmark PRIVATE archie-cortex-core)

    add_executable(archie-cortex-daemon-e2e tests/test_daemon.cpp)
    find_package(Threads REQUIRED)
    target_link_libraries(archie-cortex-daemon-e2e PRIVATE archie-cortex-core Threads::Threads)
    if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
        target_compile_definitions(archie-cortex-daemon-e2e PRIVATE _GNU_SOURCE)
    endif()
    target_compile_options(archie-cortex-daemon-e2e PRIVATE
        $<$<CXX_COMPILER_ID:GNU,Clang>:-Wall -Wextra -Wpedantic -Wswitch-enum -Werror>
    )
    target_compile_definitions(archie-cortex-daemon-e2e PRIVATE
        ARCHIE_CORTEXD_PATH="$<TARGET_FILE:archie-cortexd>"
    )
    add_dependencies(archie-cortex-daemon-e2e archie-cortexd)
    add_test(NAME archie-cortex-daemon-e2e COMMAND archie-cortex-daemon-e2e)
    set_tests_properties(archie-cortex-daemon-e2e PROPERTIES SKIP_RETURN_CODE 77 TIMEOUT 30)
endif()

install(TARGETS archie-cortexd RUNTIME DESTINATION bin)
