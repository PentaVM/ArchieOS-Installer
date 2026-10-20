import QtQuick

// The hologram's grade over the finished frame of a 3D view, as the CSS filter Xavi's
// shell laid over his sphere's canvas: hue-rotate, saturate, contrast, brightness, then
// faint scan lines and a small red/cyan split. Used as a layer effect of the View3D.
ShaderEffect {
    required property var look
    /// Whether the render budget allows the full post-processing chain: the glow then
    /// takes 16 taps instead of 8.
    property bool postProcessing: false
    property real glow: Number(look.gradeGlow || 0)
    property real glowTaps: postProcessing ? 16 : 8
    property real hue: Number(look.gradeHue || 0)
    property real saturation: Number(look.gradeSaturate === undefined ? 1 : look.gradeSaturate)
    property real contrast: Number(look.gradeContrast === undefined ? 1 : look.gradeContrast)
    property real brightness: Number(look.gradeBrightness === undefined ? 1 : look.gradeBrightness)
    property real scanlines: Number(look.gradeScanlines || 0)
    property real split: Number(look.gradeSplit || 0)
    property size size: Qt.size(width, height)
    fragmentShader: "qrc:/archie/shaders/hologram_grade.frag.qsb"
}
