pragma Singleton
import QtQuick

// Design tokens for the ArchieOS professional shell (DESIGN PROTOTYPE).
// Three explored directions plus the final synthesis; see docs/gui/DESIGN_SYSTEM.md.
// Base colours come from the existing Observatory (display/native/qml/Main.qml)
// so the shell and the Orb read as one system.
QtObject {
    id: t
    // frost | workspace | signature | synthesis
    property string variant: "synthesis"
    // Real blur depends on the compositor; false = opaque accessible fallback.
    property bool glass: true
    property bool reducedMotion: false

    readonly property var table: ({
        frost:     { bg: "#0d1117", surface: "#161b22", raised: "#1e252e", line: "#2a323d", ink: "#eef2f7", soft: "#aab3c0", dim: "#8691a0",
                     accent: "#5aa9ff", radius: 14, gap: 12, bar: 34, dock: 64, density: 1.0, wall0: "#1b2a3c", wall1: "#0b0f15" },
        workspace: { bg: "#0f1214", surface: "#171b1e", raised: "#1f2427", line: "#2c3338", ink: "#e8ecef", soft: "#a3acb2", dim: "#85909a",
                     accent: "#3ec9a7", radius: 8,  gap: 8,  bar: 30, dock: 52, density: 0.86, wall0: "#12302b", wall1: "#0a0d0d" },
        signature: { bg: "#04060a", surface: "#0a0d13", raised: "#10141b", line: "#1d232d", ink: "#e9edf3", soft: "#a4acb9", dim: "#7b8596",
                     accent: "#6fe3f5", radius: 12, gap: 12, bar: 32, dock: 60, density: 1.0, wall0: "#0b2030", wall1: "#04060a" },
        synthesis: { bg: "#080b10", surface: "#11161d", raised: "#192029", line: "#252d38", ink: "#eaeff5", soft: "#a8b1be", dim: "#8792a2",
                     accent: "#6fc8f5", radius: 12, gap: 10, bar: 32, dock: 60, density: 0.94, wall0: "#14283c", wall1: "#05070b" }
    })
    readonly property var v: table[variant]

    readonly property color bg: v.bg
    readonly property color solid: v.surface
    readonly property color surface: glass ? Qt.rgba(Qt.color(v.surface).r, Qt.color(v.surface).g, Qt.color(v.surface).b, 0.88) : v.surface
    readonly property color raised: v.raised
    readonly property color line: v.line
    readonly property color ink: v.ink
    readonly property color soft: v.soft
    readonly property color dim: v.dim
    readonly property color accent: v.accent
    readonly property color wall0: v.wall0
    readonly property color wall1: v.wall1
    readonly property int radius: v.radius
    readonly property int gap: Math.round(v.gap * v.density)
    readonly property int barH: v.bar
    readonly property int dockH: v.dock
    readonly property real density: v.density

    // State colours. Severity is never carried by hue alone: every state also has a glyph and a word.
    readonly property color ok: "#47d18b"
    readonly property color warn: "#f0b429"
    readonly property color bad: "#f2626b"
    readonly property color info: "#6fe3f5"
    readonly property color unknown: "#8792a2"

    // Motion tokens (ms). Reduced motion collapses spatial movement to instant.
    readonly property int fast: reducedMotion ? 0 : 120
    readonly property int base: reducedMotion ? 0 : 180
    readonly property int slow: reducedMotion ? 0 : 280

    readonly property int fsSmall: Math.round(11 * density + 1)
    readonly property int fsBody: Math.round(13 * density + 1)
    readonly property int fsTitle: Math.round(16 * density + 1)
}
