import QtQuick

// What sits over the orb: almost nothing. The orb is configured by Cortex, not by
// panels, so this layer only keeps the hands (drag to turn, scroll to approach, click to
// select, double click to open a module), a name beside the pointer, and one quiet line
// saying what Archie is doing and what it is using.
Item {
    id: hud

    required property var view
    required property var orbScene

    function pickAt(x, y) {
        const near = hud.view.mapTo3DScene(Qt.vector3d(x, y, 0))
        const far = hud.view.mapTo3DScene(Qt.vector3d(x, y, 4000))
        const field = hud.orbScene.field
        return lumen.pick(field.mapPositionFromScene(near), field.mapPositionFromScene(far))
    }

    readonly property var s: lumen.scene
    readonly property string mode: Number(s.speak || 0) > 0.05 ? "Hablando"
                                   : Number(s.think || 0) > 0.05 ? "Pensando"
                                   : Number(s.listen || 0) > 0.05 ? "Escuchando" : "En reposo"
    readonly property color modeColour: mode === "Hablando" ? "#a9c7ff" : mode === "Pensando" ? "#f2c98f"
                                        : mode === "Escuchando" ? "#8fe3d3" : "#6f7785"

    HoverHandler {
        id: hover
        onPointChanged: hoverDelay.restart()
        onHoveredChanged: if (!hovered) lumen.hover("")
    }
    Timer {
        id: hoverDelay
        interval: 40
        onTriggered: lumen.hover(hover.hovered ? hud.pickAt(hover.point.position.x, hover.point.position.y) : "")
    }

    MouseArea {
        anchors.fill: parent
        property point last
        property point origin
        property bool moved: false
        onPressed: function (mouse) { last = Qt.point(mouse.x, mouse.y); origin = last; moved = false }
        onPositionChanged: function (mouse) {
            if (Math.abs(mouse.x - origin.x) + Math.abs(mouse.y - origin.y) > 5)
                moved = true
            if (moved) {
                hud.orbScene.yaw -= (mouse.x - last.x) * 0.3
                hud.orbScene.pitch = Math.max(-80, Math.min(80, hud.orbScene.pitch - (mouse.y - last.y) * 0.3))
            }
            last = Qt.point(mouse.x, mouse.y)
        }
        onClicked: function (mouse) {
            if (moved) return
            const id = hud.pickAt(mouse.x, mouse.y)
            if (id.length > 0) lumen.select(id)
            else lumen.clearSelection()
        }
        onDoubleClicked: function (mouse) {
            const id = hud.pickAt(mouse.x, mouse.y)
            if (id.length > 0) lumen.toggleModule(id)
        }
        onWheel: function (wheel) {
            hud.orbScene.approach(Math.pow(0.88, wheel.angleDelta.y / 120))
        }
    }

    // A name beside the pointer, nothing more.
    Text {
        visible: !!lumen.hovered.id && hover.hovered
        x: Math.min(parent.width - width - 12, hover.point.position.x + 18)
        y: Math.max(12, hover.point.position.y - 26)
        text: lumen.hovered.name || ""
        color: "#e9edf3"
        font.pixelSize: 13
        font.weight: Font.Medium
        style: Text.Raised
        styleColor: "#aa000000"
    }

    // The one line: state, then what is lit and why.
    Row {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 34
        spacing: 12

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 7; height: 7; radius: 3.5
            color: hud.modeColour
            SequentialAnimation on opacity {
                running: hud.mode !== "En reposo"
                loops: 3
                NumberAnimation { to: 0.35; duration: 700; easing.type: Easing.InOutSine }
                NumberAnimation { to: 1.0; duration: 700; easing.type: Easing.InOutSine }
            }
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: hud.mode
            color: "#c9ced8"
            font.pixelSize: 13
            font.weight: Font.Medium
            font.letterSpacing: 0.3
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            visible: lumen.attention.length > 0
            text: "·  " + lumen.attention.slice(0, 3).map(function (a) { return a.name }).join("  ·  ")
            color: "#8b93a1"
            font.pixelSize: 13
        }
    }

    Text {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 14
        visible: lumen.error.length > 0
        text: "Sin conexión con el observador del sistema"
        color: "#c98a8a"
        font.pixelSize: 11
    }
}
