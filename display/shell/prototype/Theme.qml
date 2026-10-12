pragma Singleton
import QtQuick

// Archie Mac · cristal grafito. Tokens from the Claude Design handoff
// (archie-mac-handoff/tokens.css). Two surface sets: glass (translucent; real blur comes from
// the compositor, not from the shell) and opaque (no blur: "reduce transparency" and the Quiet
// profile). Only a dark theme exists.
QtObject {
    id: t
    property bool opaque: false
    // True only when the compositor really blurs what is behind a surface. The shell cannot blur:
    // without it, windows, sidebar and panels use their solid fill (translucency over unblurred
    // content breaks legibility: text shows through). Bar and dock stay glass over the desktop.
    property bool blur: false
    property bool reducedMotion: false

    // Fonts shipped with the shell (display/shell/fonts, SIL OFL 1.1).
    property FontLoader interFont: FontLoader { source: Qt.resolvedUrl("../fonts/inter/Inter-Variable.ttf") }
    property FontLoader groteskFont: FontLoader { source: Qt.resolvedUrl("../fonts/space-grotesk/SpaceGrotesk-Variable.ttf") }
    readonly property string font: interFont.status === FontLoader.Ready ? interFont.font.family : "sans-serif"
    readonly property string disp: groteskFont.status === FontLoader.Ready ? groteskFont.font.family : font

    readonly property color desktop: "#07080c"

    // Surfaces: glass / opaque.
    readonly property color bar: opaque ? "#101117" : Qt.rgba(16 / 255, 17 / 255, 23 / 255, 0.62)
    readonly property color dock: opaque ? "#14151c" : Qt.rgba(20 / 255, 21 / 255, 28 / 255, 0.55)
    readonly property color win: (opaque || !blur) ? "#12131a" : Qt.rgba(18 / 255, 19 / 255, 26 / 255, 0.78)
    readonly property color side: (opaque || !blur) ? "#1a1b24" : Qt.rgba(26 / 255, 27 / 255, 36 / 255, 0.66)
    readonly property color panel: (opaque || !blur) ? "#16171f" : Qt.rgba(22 / 255, 23 / 255, 31 / 255, 0.86)
    readonly property color raised: opaque ? "#22232a" : Qt.rgba(1, 1, 1, 0.05)
    readonly property color hover: opaque ? "#2a2b32" : Qt.rgba(1, 1, 1, 0.08)
    readonly property color pressed: opaque ? "#31323a" : Qt.rgba(1, 1, 1, 0.11)
    readonly property color hair: opaque ? "#2c2d36" : Qt.rgba(1, 1, 1, 0.09)
    readonly property color inner: opaque ? "#25262e" : Qt.rgba(1, 1, 1, 0.06)
    readonly property color sel: opaque ? "#1f3a4a" : Qt.rgba(111 / 255, 200 / 255, 245 / 255, 0.16)

    // Text.
    readonly property color t1: "#f2f1ee"
    readonly property color t2: "#b9b8c4"
    readonly property color t3: "#8d8c9c"
    readonly property color tdis: "#5e5d6b"

    // Accents and states. A state is never colour alone: icon + word always.
    readonly property color acc: "#6fc8f5"
    readonly property color arch: "#f0bb78"
    readonly property color ok: "#47d18b"
    readonly property color warn: "#f0b429"
    readonly property color bad: "#f2626b"
    readonly property color info: "#6fe3f5"
    readonly property color unk: "#8d8c9c"

    // File categories (colour-blind safe, same as the sphere's nodes).
    readonly property var cat: ({ documents: "#e69f00", media: "#009e73", data: "#f0e442",
                                  archive: "#cc79a7", executable: "#d55e00", other: "#bbbbbb" })
    // Dock icons: solid colour under a 165° white→black wash.
    readonly property var dk: ({ orb: "#1b8fa6", vault: "#3b7fd6", forge: "#b87a0b",
                                 sentry: "#1f9a62", stage: "#8a5fd6", set: "#6b7686" })
    // Window controls.
    readonly property color tlClose: "#d9605a"
    readonly property color tlMin: "#d9a441"
    readonly property color tlZoom: "#4fb67b"
    readonly property color tlOff: Qt.rgba(1, 1, 1, 0.17)

    // Radii.
    readonly property int rWindow: 14
    readonly property int rPanel: 16
    readonly property int rCard: 10
    readonly property int rRow: 8
    readonly property int rDock: 22
    readonly property int rDockIcon: 14

    readonly property int barH: 32
    readonly property int dockIcon: 56

    // Motion: the handoff defines no motion table yet, so only simple state transitions animate.
    readonly property int fast: reducedMotion ? 0 : 120
    readonly property int base: reducedMotion ? 0 : 180

    // Tinted chip fills: only warn, bad and arch carry a background tint.
    function tint(c, a) { return Qt.rgba(c.r, c.g, c.b, a) }
}
