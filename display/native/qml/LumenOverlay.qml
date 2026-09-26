import QtQuick
import QtQuick.Controls
import QtCore

// Everything drawn over the orb's viewport: the heading, the toolbar, the cognitive
// lenses, the overview, the status line and the hover card; and the hands: dragging
// turns the field, the wheel moves closer, a click selects, a double click opens a module.
Item {
    id: overlay

    required property var view
    required property var lumenScene
    property bool inspectorOpen: true
    property string notice: ""

    readonly property color ink: "#dce5e8"
    readonly property color muted: "#81949e"
    readonly property color accent: "#f0be7e"
    readonly property color panel: "#e6101820"

    function rayAt(x, y) {
        const near = overlay.view.mapTo3DScene(Qt.vector3d(x, y, 0))
        const far = overlay.view.mapTo3DScene(Qt.vector3d(x, y, 4000))
        const field = overlay.lumenScene.field
        return [field.mapPositionFromScene(near), field.mapPositionFromScene(far)]
    }
    function pickAt(x, y) {
        const ray = rayAt(x, y)
        return lumen.pick(ray[0], ray[1])
    }
    function say(text) {
        overlay.notice = text
        noticeTimer.restart()
    }
    function capture() {
        const folder = StandardPaths.writableLocation(StandardPaths.PicturesLocation)
        const stamp = new Date().toISOString().replace(/[:.]/g, "-")
        const path = String(folder).replace("file://", "") + "/ARCHIE-Lumenfield-" + stamp + ".png"
        overlay.view.grabToImage(function (result) {
            overlay.say(result.saveToFile(path) ? "Captura guardada en " + path : "No se pudo guardar la captura")
        })
    }

    // --- The hands ---
    Item {
        anchors.fill: parent
        property point last
        property point origin
        property bool moved: false

        HoverHandler {
            id: hover
            onPointChanged: hoverDelay.restart()
        }
        Timer {
            id: hoverDelay
            interval: 40
            onTriggered: lumen.hover(hover.hovered ? overlay.pickAt(hover.point.position.x, hover.point.position.y) : "")
        }
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton
            property point last
            property point origin
            property bool moved: false
            onPressed: function (mouse) { last = Qt.point(mouse.x, mouse.y); origin = last; moved = false }
            onPositionChanged: function (mouse) {
                if (Math.abs(mouse.x - origin.x) + Math.abs(mouse.y - origin.y) > 5)
                    moved = true
                if (moved) {
                    overlay.lumenScene.yaw -= (mouse.x - last.x) * 0.3
                    overlay.lumenScene.pitch = Math.max(-85, Math.min(85, overlay.lumenScene.pitch - (mouse.y - last.y) * 0.3))
                }
                last = Qt.point(mouse.x, mouse.y)
            }
            onClicked: function (mouse) {
                if (moved) return
                const id = overlay.pickAt(mouse.x, mouse.y)
                if (id.length > 0) lumen.select(id)
                else lumen.clearSelection()
            }
            onDoubleClicked: function (mouse) {
                const id = overlay.pickAt(mouse.x, mouse.y)
                if (id.length > 0) lumen.toggleModule(id)
            }
            onWheel: function (wheel) {
                const factor = Math.pow(0.88, wheel.angleDelta.y / 120)
                overlay.lumenScene.distance = Math.max(26, Math.min(8000, overlay.lumenScene.distance * factor))
            }
        }
    }

    // --- Heading ---
    Column {
        anchors.left: parent.left; anchors.leftMargin: 24
        anchors.top: parent.top; anchors.topMargin: 18
        spacing: 3
        Text { text: "CAMPO ACTIVO"; color: overlay.muted; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.8 }
        Text { text: "ARCHIE LUMENFIELD"; color: overlay.ink; font.pixelSize: 16; font.bold: true; font.letterSpacing: 1.4 }
        Text {
            text: (lumen.scene.focusName ? lumen.scene.focusName + " · " : "ArchieOS · ") + "grafo TFOG en vivo"
            color: overlay.muted; font.pixelSize: 10
        }
    }

    // --- Toolbar ---
    Row {
        anchors.right: parent.right; anchors.rightMargin: 20
        anchors.top: parent.top; anchors.topMargin: 20
        spacing: 6
        Tool { glyph: "⟳"; tip: "Actualizar"; onClicked: lumen.refresh() }
        Tool { glyph: "⤢"; tip: "Encuadrar"; onClicked: overlay.lumenScene.fit() }
        Tool { glyph: lumen.paused ? "▶" : "❚❚"; tip: lumen.paused ? "Reanudar animación" : "Pausar animación"; onClicked: lumen.togglePause() }
        Tool { glyph: "◉"; tip: "Capturar"; onClicked: overlay.capture() }
        Tool { glyph: "☰"; tip: "Configuración"; checked: overlay.inspectorOpen; onClicked: overlay.inspectorOpen = !overlay.inspectorOpen }
    }

    // --- Cognitive lenses ---
    Rectangle {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top; anchors.topMargin: 66
        width: 360; height: 84; radius: 6
        color: overlay.panel; border.color: "#2c3b44"
        Column {
            anchors.fill: parent; anchors.margins: 10
            spacing: 7
            Item {
                width: parent.width; height: 12
                Row {
                    spacing: 7
                    Rectangle { width: 7; height: 7; radius: 4; color: "#78c2ca"; anchors.verticalCenter: parent.verticalCenter }
                    Text { text: "LECTURA COGNITIVA"; color: overlay.ink; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.6 }
                }
                Text { anchors.right: parent.right; text: "LOCAL / TFOG"; color: overlay.muted; font.pixelSize: 8; font.bold: true; font.letterSpacing: 1.2 }
            }
            Row {
                spacing: 4
                Repeater {
                    model: [["architecture", "Arquitectura"], ["folders", "Módulos"], ["entities", "Entidades"]]
                    delegate: Rectangle {
                        required property var modelData
                        width: 110; height: 28; radius: 4
                        color: lumen.lens === modelData[0] ? "#2a2217" : "#18232c"
                        border.color: lumen.lens === modelData[0] ? "#d6a873" : "#2c3b44"
                        Text { anchors.centerIn: parent; text: modelData[1]; color: lumen.lens === modelData[0] ? overlay.accent : overlay.ink; font.pixelSize: 11 }
                        MouseArea { anchors.fill: parent; onClicked: lumen.setLens(modelData[0]) }
                    }
                }
            }
            Text {
                width: parent.width; horizontalAlignment: Text.AlignHCenter
                text: lumen.lens === "entities" ? "Una esfera por entidad observada"
                      : lumen.lens === "folders" ? "Módulos desplegados · entidades agrupadas por tipo"
                      : "Vista funcional · módulos plegados por defecto"
                color: overlay.muted; font.pixelSize: 9
            }
        }
    }

    // --- Overview ---
    Rectangle {
        visible: lumen.lens !== "entities"
        anchors.left: parent.left; anchors.leftMargin: 20
        anchors.bottom: parent.bottom; anchors.bottomMargin: 46
        width: 300; height: overviewColumn.implicitHeight + 22; radius: 6
        color: overlay.panel; border.color: "#2c3b44"
        Column {
            id: overviewColumn
            anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top; anchors.margins: 11
            spacing: 5
            Row {
                spacing: 7
                Rectangle { width: 7; height: 7; radius: 4; color: "#78c2ca"; anchors.verticalCenter: parent.verticalCenter }
                Text { text: lumen.status; color: overlay.ink; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.4 }
            }
            Text {
                text: (lumen.scene.sourceCount || 0) + " entidades → " + (lumen.scene.nodeCount || 0) + " nodos"
                color: overlay.accent; font.pixelSize: 15; font.bold: true
            }
            Text {
                text: (lumen.scene.collapsedCount || 0) + " plegadas · perfil " + (lumen.scene.profile || "—")
                      + ((lumen.scene.staleSignals || 0) > 0 ? " · " + lumen.scene.staleSignals + " señales vencidas" : "")
                color: overlay.muted; font.pixelSize: 10
            }
            Row {
                spacing: 12
                Legend { swatch: "#8ba6a8"; text: "Jerarquía" }
                Legend { swatch: "#e8b47d"; text: (lumen.scene.observedCount || 0) + " observadas" }
                Legend { swatch: "#78c2ca"; text: (lumen.scene.semanticCount || 0) + " semánticas"; dashed: true }
            }
            Text {
                text: "Ámbar = relación observada · Cian = similitud de vectores"
                color: "#6c838e"; font.pixelSize: 8
            }
        }
    }

    // --- Status line ---
    Row {
        anchors.left: parent.left; anchors.leftMargin: 24
        anchors.bottom: parent.bottom; anchors.bottomMargin: 18
        spacing: 12
        Text { text: "◉"; color: overlay.accent; font.pixelSize: 11 }
        Text { text: "CAMPO DE CONTENIDO"; color: overlay.muted; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.6 }
        Text { text: (lumen.scene.nodeCount || 0); color: overlay.ink; font.pixelSize: 11; font.bold: true }
        Text { text: "NODOS"; color: overlay.muted; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.6 }
        Rectangle { width: 1; height: 12; color: "#35444c" }
        Text { text: lumen.fps; color: overlay.ink; font.pixelSize: 11; font.bold: true }
        Text { text: "FPS"; color: overlay.muted; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.6 }
        Rectangle { width: 1; height: 12; color: "#35444c" }
        Text {
            readonly property var s: lumen.scene
            text: Number(s.speak || 0) > 0.05 ? "HABLANDO" : Number(s.think || 0) > 0.05 ? "PENSANDO" : Number(s.listen || 0) > 0.05 ? "ESCUCHANDO" : "EN REPOSO"
            color: Number(s.speak || 0) > 0.05 || Number(s.listen || 0) > 0.05 ? "#78c2ca" : overlay.muted
            font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.6
        }
    }

    // --- Hover card ---
    Rectangle {
        visible: !!lumen.hovered.id && hover.hovered
        x: Math.min(parent.width - width - 8, hover.point.position.x + 16)
        y: Math.max(8, hover.point.position.y - height - 10)
        width: hoverText.implicitWidth + 18; height: 26; radius: 4
        color: overlay.panel; border.color: "#d6a873"
        Text { id: hoverText; anchors.centerIn: parent; text: (lumen.hovered.name || "") + " · " + (lumen.hovered.label || ""); color: overlay.ink; font.pixelSize: 10 }
    }

    // --- Notices ---
    Rectangle {
        visible: overlay.notice.length > 0 || lumen.error.length > 0
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom; anchors.bottomMargin: 18
        width: Math.min(parent.width - 60, noticeText.implicitWidth + 28); height: 32; radius: 5
        color: lumen.error.length > 0 ? "#33171319" : overlay.panel
        border.color: lumen.error.length > 0 ? "#f2626b" : "#d6a873"
        Text {
            id: noticeText
            anchors.centerIn: parent; width: parent.width - 20; elide: Text.ElideRight; horizontalAlignment: Text.AlignHCenter
            text: lumen.error.length > 0 ? lumen.error : overlay.notice
            color: lumen.error.length > 0 ? "#f2626b" : overlay.ink; font.pixelSize: 10
        }
    }
    Timer { id: noticeTimer; interval: 5000; onTriggered: overlay.notice = "" }

    component Tool: Rectangle {
        id: tool
        property string glyph: ""
        property string tip: ""
        property bool checked: false
        signal clicked()
        width: 34; height: 34; radius: 5
        color: checked ? "#2a2217" : (toolArea.containsMouse ? "#1c2a33" : "#cc101820")
        border.color: checked ? "#d6a873" : "#35444c"
        Text { anchors.centerIn: parent; text: tool.glyph; color: tool.checked ? "#f0be7e" : "#dce5e8"; font.pixelSize: 13 }
        MouseArea { id: toolArea; anchors.fill: parent; hoverEnabled: true; onClicked: tool.clicked() }
        ToolTip.visible: toolArea.containsMouse && tip.length > 0
        ToolTip.text: tip
        ToolTip.delay: 400
    }

    component Legend: Row {
        id: legend
        property color swatch: "white"
        property string text: ""
        property bool dashed: false
        spacing: 5
        Row {
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            Repeater {
                model: legend.dashed ? 3 : 1
                Rectangle { width: legend.dashed ? 4 : 14; height: 2; color: legend.swatch }
            }
        }
        Text { text: legend.text; color: "#dce5e8"; font.pixelSize: 9 }
    }
}
