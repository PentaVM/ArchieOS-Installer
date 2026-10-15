cmake_minimum_required(VERSION 3.24)

project(archie-display LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# The semantic visual state model: what to show, decided without Qt. Kept in its own
# subproject so that the decisions worth testing can be tested on a machine with no GPU.
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../visual archie-visual)
# Reading a real .osm.pbf extract into building footprints. Depends on zlib, which is
# exactly why it is its own library rather than living in archie-visual.
add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../osm archie-osm)

find_package(Qt6 6.6 REQUIRED COMPONENTS Quick QuickControls2 Quick3D Network Concurrent)
qt_standard_project_setup(REQUIRES 6.6)

qt_add_executable(archie-display
    src/main.cpp
    src/ArchieBridge.cpp
    src/ArchieBridge.h
    src/MeshGeometry.cpp
    src/MeshGeometry.h
    src/Payload.cpp
    src/Payload.h
    src/LumenField.cpp
    src/LumenField.h
    src/LumenGeometry.cpp
    src/LumenGeometry.h
    src/AppCatalog.cpp
    src/AppCatalog.h
    src/DisplayControl.cpp
    src/DisplayControl.h
    src/ShellRuntime.cpp
    src/ShellRuntime.h
)

qt_add_qml_module(archie-display
    URI Archie.Observatory
    VERSION 1.0
    QML_FILES
        qml/Main.qml
        qml/LumenScene.qml
        qml/OrbHud.qml
        qml/ForgeView.qml
        qml/OrbViewport.qml
)

# Keep relative prototype/font URLs intact; every operational shell asset is local.
file(GLOB_RECURSE ARCHIE_SHELL_QML CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/../shell/prototype/*.qml")
file(GLOB_RECURSE ARCHIE_SHELL_JS CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/../shell/prototype/*.js")
list(FILTER ARCHIE_SHELL_QML EXCLUDE REGEX "Test\\.qml$")
list(FILTER ARCHIE_SHELL_QML EXCLUDE REGEX "/tst_[^/]+\\.qml$")
file(GLOB_RECURSE ARCHIE_SHELL_FONTS CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/../shell/fonts/*.ttf")
qt_add_resources(archie-display "archie-shell-assets"
    PREFIX "/archie/shell"
    BASE "${CMAKE_CURRENT_SOURCE_DIR}/../shell"
    FILES ${ARCHIE_SHELL_QML} ${ARCHIE_SHELL_JS} ${ARCHIE_SHELL_FONTS}
          "${CMAKE_CURRENT_SOURCE_DIR}/../shell/prototype/qmldir")

qt_add_resources(archie-display "archie-native-assets"
    PREFIX "/archie"
    BASE "${CMAKE_CURRENT_SOURCE_DIR}"
    FILES
        assets/earth-blue-marble.png
        assets/earth-bmng-5400.jpg
        shaders/atmosphere.vert
        shaders/atmosphere.frag
        shaders/star.vert
        shaders/star.frag
        shaders/lumen_orb.vert
        shaders/lumen_orb.frag
        shaders/lumen_sprite.vert
        shaders/lumen_sprite.frag
        shaders/lumen_shell.vert
        shaders/lumen_shell.frag
        shaders/lumen_line.vert
        shaders/lumen_line.frag
        shaders/lumen_ring.vert
        shaders/lumen_ring.frag
        presets/field.json
        presets/sphere.json
        presets/orb.json
        presets/neural.json
        apps/archie-stage.desktop
        apps/archie-voice.desktop
        apps/archie-editor.desktop
        apps/archie-lienzo.desktop
        apps/archie-cli.desktop
)

target_link_libraries(archie-display PRIVATE
    archie::visual
    archie::osm
    Qt6::Quick
    Qt6::QuickControls2
    Qt6::Quick3D
    Qt6::Network
    Qt6::Concurrent
)

# QML_ELEMENT's generated registration code looks up a QML type's header with an
# angle-bracket include (`#if __has_include(<MeshGeometry.h>)`), which only searches -I
# directories, not "the same directory as the including file" the way a quoted include
# does. Without src/ on the include path that check silently evaluates false and the type
# never gets a definition to register against.
target_include_directories(archie-display PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/src)

target_compile_definitions(archie-display PRIVATE
    QT_NO_CAST_FROM_ASCII
    QT_NO_CAST_TO_ASCII
    QT_NO_URL_CAST_FROM_STRING
)

install(TARGETS archie-display RUNTIME DESTINATION bin)

option(ARCHIE_NATIVE_BUILD_TESTS "Build native lifecycle and transport tests" OFF)
if(ARCHIE_NATIVE_BUILD_TESTS)
    find_package(Qt6 6.6 REQUIRED COMPONENTS Test)
    enable_testing()
    qt_add_executable(test_native tests/test_native.cpp
        src/LumenField.cpp src/LumenField.h src/LumenGeometry.cpp src/LumenGeometry.h)
    target_include_directories(test_native PRIVATE src)
    target_link_libraries(test_native PRIVATE archie::visual Qt6::Quick3D Qt6::Network Qt6::Test)
    qt_add_resources(test_native "archie-test-presets" PREFIX "/archie" BASE "${CMAKE_CURRENT_SOURCE_DIR}"
        FILES presets/field.json presets/sphere.json presets/orb.json presets/neural.json)
    add_test(NAME native_contracts COMMAND test_native)
    qt_add_executable(test_shell_runtime tests/test_shell_runtime.cpp src/ShellRuntime.cpp src/ShellRuntime.h)
    target_include_directories(test_shell_runtime PRIVATE src)
    target_link_libraries(test_shell_runtime PRIVATE Qt6::Core Qt6::Test)
    add_test(NAME shell_runtime_contracts COMMAND test_shell_runtime)
    qt_add_executable(test_display_control tests/test_display_control.cpp src/DisplayControl.cpp src/DisplayControl.h)
    target_include_directories(test_display_control PRIVATE src)
    target_link_libraries(test_display_control PRIVATE Qt6::Network Qt6::Test)
    add_test(NAME display_control_contracts COMMAND test_display_control)
    set_target_properties(test_native test_shell_runtime test_display_control PROPERTIES WIN32_EXECUTABLE FALSE)
endif()
