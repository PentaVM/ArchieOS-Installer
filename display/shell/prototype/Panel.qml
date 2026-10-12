import QtQuick

// Glass surface: window, panel or dock. Fill + hairline border + 1px inner highlight on top +
// a soft drop shadow. The handoff's shadows (window 0 18 50 .45, panel 0 24 70 .55, dock
// 0 8 24 .35) are approximated with stacked translucent layers: no shader, so it costs
// nothing on the GPU IRIS uses. Background blur is the compositor's job, not the shell's.
Item {
    id: p
    property string kind: "panel"       // window | panel | dock
    default property alias data: holder.data
    property alias contentItem: holder
    // Reveal (MOVIMIENTO.md): opacity plus a short slide (panels) or scale (windows); only
    // transform and opacity. Quiet keeps opacity alone; reduced motion shows or hides at once.
    property bool shown: true
    property real enterDy: 0
    property real enterScale: 1
    property int durIn: Theme.durPanel
    property int durOut: Theme.durPanel
    property var easeIn: Theme.easeOut
    property var easeOut: Theme.easeIn
    opacity: shown ? 1 : 0
    visible: opacity > 0
    scale: (shown || !Theme.spatial) ? 1 : enterScale
    transform: Translate { y: (p.shown || !Theme.spatial) ? 0 : p.enterDy
        Behavior on y { NumberAnimation { duration: p.shown ? p.durIn : p.durOut; easing.type: Easing.BezierSpline; easing.bezierCurve: p.shown ? p.easeIn : p.easeOut } } }
    Behavior on opacity { NumberAnimation { duration: p.shown ? p.durIn : p.durOut; easing.type: Easing.BezierSpline; easing.bezierCurve: p.shown ? p.easeIn : p.easeOut } }
    Behavior on scale { NumberAnimation { duration: p.shown ? p.durIn : p.durOut; easing.type: Easing.BezierSpline; easing.bezierCurve: p.shown ? p.easeIn : p.easeOut } }
    readonly property int radius: kind === "window" ? Theme.rWindow : kind === "dock" ? Theme.rDock : Theme.rPanel
    readonly property color fill: kind === "window" ? Theme.win : kind === "dock" ? Theme.dock : Theme.panel
    readonly property int shY: kind === "window" ? 18 : kind === "dock" ? 8 : 24
    readonly property int shBlur: kind === "window" ? 50 : kind === "dock" ? 24 : 70
    readonly property real shA: kind === "window" ? 0.45 : kind === "dock" ? 0.35 : 0.55

    // Soft shadow: 8 rings, each a little wider and fainter.
    Repeater {
        model: 8
        Rectangle {
            readonly property real f: (index + 1) / 8
            x: -p.shBlur * 0.5 * f; y: p.shY - p.shBlur * 0.5 * f + p.shBlur * 0.15
            width: p.width + p.shBlur * f; height: p.height + p.shBlur * f
            radius: p.radius + p.shBlur * 0.5 * f
            color: Qt.rgba(0, 0, 0, p.shA * 0.22 * (1 - f) + 0.004)
        }
    }
    Rectangle { // tight contact shadow
        x: 0; y: 2; width: p.width; height: p.height; radius: p.radius
        color: Qt.rgba(0, 0, 0, p.kind === "dock" ? 0.2 : 0.28)
    }
    Rectangle {
        id: body
        anchors.fill: parent; radius: p.radius
        color: p.fill; border.width: 1; border.color: Theme.hair
    }
    Rectangle { // inner highlight: inset 0 1px 0 var(--inner)
        x: p.radius * 0.6; y: 1; width: p.width - p.radius * 1.2; height: 1; color: Theme.inner
    }
    Item { id: holder; anchors.fill: parent }
}
