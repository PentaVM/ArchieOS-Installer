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
    // Vivid colours (default): near-black surfaces, more saturated accents. Settings > Apariencia.
    property bool vivid: true

    // Fonts shipped with the shell (display/shell/fonts, SIL OFL 1.1).
    property FontLoader interFont: FontLoader { source: Qt.resolvedUrl("../fonts/inter/Inter-Variable.ttf") }
    property FontLoader groteskFont: FontLoader { source: Qt.resolvedUrl("../fonts/space-grotesk/SpaceGrotesk-Variable.ttf") }
    readonly property string font: interFont.status === FontLoader.Ready ? interFont.font.family : "sans-serif"
    readonly property string disp: groteskFont.status === FontLoader.Ready ? groteskFont.font.family : font

    readonly property color desktop: vivid ? "#000000" : "#07080c"

    // Surfaces: glass / opaque.
    readonly property color bar: opaque ? (vivid ? "#030304" : "#101117") : (vivid ? Qt.rgba(2 / 255, 2 / 255, 4 / 255, 0.7) : Qt.rgba(16 / 255, 17 / 255, 23 / 255, 0.62))
    readonly property color dock: opaque ? (vivid ? "#050506" : "#14151c") : (vivid ? Qt.rgba(4 / 255, 4 / 255, 6 / 255, 0.62) : Qt.rgba(20 / 255, 21 / 255, 28 / 255, 0.55))
    readonly property color win: (opaque || !blur) ? (vivid ? "#050507" : "#12131a") : (vivid ? Qt.rgba(3 / 255, 3 / 255, 5 / 255, 0.82) : Qt.rgba(18 / 255, 19 / 255, 26 / 255, 0.78))
    readonly property color side: (opaque || !blur) ? (vivid ? "#09090c" : "#1a1b24") : (vivid ? Qt.rgba(8 / 255, 8 / 255, 11 / 255, 0.7) : Qt.rgba(26 / 255, 27 / 255, 36 / 255, 0.66))
    readonly property color panel: (opaque || !blur) ? (vivid ? "#070709" : "#16171f") : (vivid ? Qt.rgba(5 / 255, 5 / 255, 8 / 255, 0.9) : Qt.rgba(22 / 255, 23 / 255, 31 / 255, 0.86))
    readonly property color raised: opaque ? (vivid ? "#101013" : "#22232a") : Qt.rgba(1, 1, 1, 0.05)
    readonly property color hover: opaque ? (vivid ? "#17171c" : "#2a2b32") : Qt.rgba(1, 1, 1, 0.08)
    readonly property color pressed: opaque ? (vivid ? "#1e1e24" : "#31323a") : Qt.rgba(1, 1, 1, 0.11)
    readonly property color hair: opaque ? (vivid ? "#202026" : "#2c2d36") : Qt.rgba(1, 1, 1, 0.09)
    readonly property color inner: opaque ? (vivid ? "#141418" : "#25262e") : Qt.rgba(1, 1, 1, 0.06)
    readonly property color sel: opaque ? (vivid ? "#0a3a52" : "#1f3a4a") : (vivid ? Qt.rgba(0, 190 / 255, 1, 0.22) : Qt.rgba(111 / 255, 200 / 255, 245 / 255, 0.16))

    // Text.
    readonly property color t1: vivid ? "#ffffff" : "#f2f1ee"
    readonly property color t2: vivid ? "#cfcfdc" : "#b9b8c4"
    readonly property color t3: vivid ? "#a2a1b4" : "#8d8c9c"
    readonly property color tdis: "#5e5d6b"

    // Accents and states. A state is never colour alone: icon + word always.
    readonly property color acc: vivid ? "#1ec0ff" : "#6fc8f5"
    readonly property color arch: vivid ? "#ffb02e" : "#f0bb78"
    readonly property color ok: vivid ? "#17e387" : "#47d18b"
    readonly property color warn: vivid ? "#ffc21a" : "#f0b429"
    readonly property color bad: vivid ? "#ff4757" : "#f2626b"
    readonly property color info: vivid ? "#22e6ff" : "#6fe3f5"
    readonly property color unk: "#8d8c9c"

    // File categories (colour-blind safe, same as the sphere's nodes).
    readonly property var cat: (vivid ? { documents: "#ffa000", media: "#00c98d", data: "#fff000",
                                  archive: "#e85cb4", executable: "#ff5a00", other: "#d8d8d8" }
                                : { documents: "#e69f00", media: "#009e73", data: "#f0e442",
                                  archive: "#cc79a7", executable: "#d55e00", other: "#bbbbbb" })
    // Dock icons: solid colour under a 165° white→black wash.
    readonly property var dk: (vivid ? { orb: "#00b4d8", vault: "#2f7bff", forge: "#ff9500",
                                 sentry: "#00c46a", stage: "#9b5cff", set: "#7b8798" }
                               : { orb: "#1b8fa6", vault: "#3b7fd6", forge: "#b87a0b",
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

    // Motion (handoff MOVIMIENTO.md): four profiles. Only transform and opacity ever animate;
    // never blur or shadows. The person's setting decides the profile; `reducedMotion` (also what
    // the OS "reduce motion" preference forces) kills every spatial movement. The default is the
    // handoff's recommendation (Balanced); it is an open owner decision (DECISIONES.md, 10).
    property string motion: "balanced"   // full | balanced | quiet
    readonly property string profile: reducedMotion ? "reduced" : motion
    function pick(full, balanced, quiet) { return profile === "full" ? full : profile === "balanced" ? balanced : profile === "quiet" ? quiet : 0 }
    // Quiet keeps only opacity: no translation or scale.
    readonly property bool spatial: profile === "full" || profile === "balanced"
    readonly property int durOpen: pick(220, 160, 100)
    readonly property int durClose: pick(160, 120, 100)
    readonly property int durPanel: pick(200, 150, 100)       // control center, activity, workspace menu
    readonly property int durPalette: pick(180, 140, 100)     // palette and search, opening
    readonly property int durPaletteClose: pick(120, 100, 100)
    readonly property int durNav: pick(200, 150, 100)         // folders, list/grid switch
    readonly property int durToast: pick(240, 180, 100)
    readonly property int durToastOut: pick(200, 150, 100)
    readonly property int durDock: pick(160, 120, 100)
    readonly property int durSel: profile === "reduced" ? 0 : 100
    readonly property int durPulse: pick(600, 400, 120)
    // Curves: Salida (out), Entrada (in), Estándar; as cubic-bezier control points.
    readonly property var easeOut: [0.16, 1, 0.3, 1, 1, 1]
    readonly property var easeIn: [0.4, 0, 1, 1, 1, 1]
    readonly property var easeStd: [0.4, 0, 0.2, 1, 1, 1]

    // Tinted chip fills: only warn, bad and arch carry a background tint.
    function tint(c, a) { return Qt.rgba(c.r, c.g, c.b, a) }
}
