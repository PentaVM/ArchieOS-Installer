pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Shapes
import QtQuick.Window
import QtWayland.Compositor
import QtWayland.Compositor.XdgShell

// The stage. Applications are hosted as surfaces on a desk that the director's camera
// frames; everything drawn over the desk — the agent's cursor, its ripples, the caption,
// the takeover banner — comes from StageController, which takes it from the tested model
// in display/visual. This file lays things out and draws them. It decides nothing.
WaylandCompositor {
    id: compositor

    required property var stage
    required property string stageSocketName
    required property bool fullscreen
    required property int initialWidth
    required property int initialHeight

    socketName: stageSocketName

    readonly property color ink: "#e6edf7"
    readonly property color soft: "#93a2b8"
    readonly property color dim: "#5b6879"
    readonly property color edge: "#23303d"
    readonly property color cyan: "#6fe3f5"
    readonly property color green: "#47d18b"
    readonly property color amber: "#f0b429"
    readonly property color red: "#f2626b"

    Component.onCompleted: stage.compositor = compositor

    // What each verb reads as, in the operator's language. The model hands over a verb, an
    // application and a count; there is no field that could carry typed text.
    function captionText() {
        const app = stage.captionApp
        switch (stage.captionVerb) {
        case "moving": return "moviendo el puntero · " + app
        case "clicking": return "clic · " + app
        case "double_clicking": return "doble clic · " + app
        case "dragging": return "arrastrando · " + app
        case "scrolling": return "desplazando · " + app
        case "typing": return "escribiendo " + stage.captionCount + " caracteres · " + app
        case "pressing_keys": return "pulsando " + stage.captionCount + (stage.captionCount === 1 ? " tecla · " : " teclas · ") + app
        case "focusing": return "enfocando · " + app
        case "interrupted": return "acción detenida · el control es tuyo"
        case "refused": return "rechazado · " + refusalText(stage.captionRefusal) + " · " + app
        }
        return ""
    }

    // The same verbs, past tense, for the strip of recent actions in the status line.
    function activityText(entry) {
        switch (entry.verb) {
        case "moving": return "movió el puntero · " + entry.app
        case "clicking": return "clic · " + entry.app
        case "double_clicking": return "doble clic · " + entry.app
        case "dragging": return "arrastró · " + entry.app
        case "scrolling": return "desplazó · " + entry.app
        case "typing": return "escribió " + entry.count + " car. · " + entry.app
        case "pressing_keys": return (entry.count === 1 ? "tecla · " : "atajo · ") + entry.app
        case "focusing": return "enfocó · " + entry.app
        case "interrupted": return "detenida por ti · " + entry.app
        case "refused": return "no se hizo: " + refusalText(entry.refusal) + " · " + entry.app
        }
        return entry.app
    }

    function refusalText(code) {
        switch (code) {
        case "operator_has_control": return "tienes el control"
        case "stage_paused": return "en pausa"
        case "surface_changed": return "la ventana cambió"
        case "surface_unknown":
        case "surface_unmapped": return "la ventana ya no existe"
        case "app_mismatch": return "otra aplicación"
        case "point_outside_surface": return "fuera de la ventana"
        case "command_expired": return "orden vencida"
        case "command_replayed": return "orden repetida"
        case "not_typeable": return "ese texto no se puede escribir aquí"
        }
        return code
    }

    // Tiles for n windows: one fills the desk, two share it, three give the first the left
    // half, more form a grid. Space is kept for the status line above and the caption below.
    function tileFor(index, count, width, height) {
        const top = 64, bottom = 104, side = 32, gap = 20
        const w = width - 2 * side
        const h = height - top - bottom
        if (count <= 1)
            return Qt.rect(side, top, w, h)
        if (count === 2)
            return Qt.rect(side + index * ((w + gap) / 2), top, (w - gap) / 2, h)
        if (count === 3) {
            if (index === 0)
                return Qt.rect(side, top, (w - gap) * 0.58, h)
            const x = side + (w - gap) * 0.58 + gap
            return Qt.rect(x, top + (index - 1) * ((h + gap) / 2), w - (x - side), (h - gap) / 2)
        }
        const columns = Math.ceil(Math.sqrt(count))
        const rows = Math.ceil(count / columns)
        const cw = (w - (columns - 1) * gap) / columns
        const ch = (h - (rows - 1) * gap) / rows
        return Qt.rect(side + (index % columns) * (cw + gap), top + Math.floor(index / columns) * (ch + gap), cw, ch)
    }

    ListModel { id: shellSurfaces }

    XdgShell {
        onToplevelCreated: (toplevel, xdgSurface) => shellSurfaces.append({ shellSurface: xdgSurface })
    }
    // Ask applications to leave window decoration to the stage, which draws one consistent
    // frame around every application. Clients that do not speak the protocol keep their own.
    XdgDecorationManagerV1 {
        preferredMode: XdgToplevel.ServerSideDecoration
    }
    // Applications that take text through an input method can be given characters the
    // keymap has no key for. Without one of these, such a character is refused, not dropped.
    TextInputManager {}
    TextInputManagerV3 {}
    QtTextInputMethodManager {}

    WaylandOutput {
        id: output
        sizeFollowsWindow: true
        window: Window {
            id: win
            width: compositor.initialWidth
            height: compositor.initialHeight
            visible: true
            visibility: compositor.fullscreen ? Window.FullScreen : Window.Windowed
            title: "Archie Stage"
            color: "#04070c"

            Component.onCompleted: {
                compositor.stage.window = win
                compositor.stage.setOutputSize(width, height)
            }
            onWidthChanged: compositor.stage.setOutputSize(width, height)
            onHeightChanged: compositor.stage.setOutputSize(width, height)

            // The desk: everything the camera frames. It zooms and pans as one piece, so a
            // window and the agent's work on it never drift apart.
            Item {
                id: desk
                width: win.width
                height: win.height
                Component.onCompleted: compositor.stage.desk = desk
                transform: [
                    Translate { x: -compositor.stage.viewX; y: -compositor.stage.viewY },
                    Scale { xScale: compositor.stage.viewZoom; yScale: compositor.stage.viewZoom }
                ]

                Rectangle {
                    anchors.fill: parent
                    gradient: Gradient {
                        GradientStop { position: 0.0; color: "#07101a" }
                        GradientStop { position: 1.0; color: "#030509" }
                    }
                }
                // A faint grid, so a zoom reads as moving closer rather than things growing.
                Repeater {
                    model: Math.ceil(win.width / 64)
                    Rectangle { required property int index; x: index * 64; width: 1; height: win.height; color: "#0a1420" }
                }
                Repeater {
                    model: Math.ceil(win.height / 64)
                    Rectangle { required property int index; y: index * 64; height: 1; width: win.width; color: "#0a1420" }
                }

                Repeater {
                    model: shellSurfaces
                    delegate: Item {
                        id: frame
                        required property var shellSurface
                        required property int index
                        readonly property rect tile: compositor.tileFor(index, shellSurfaces.count, win.width, win.height)
                        readonly property bool focused: compositor.stage.focusedItem === surfaceItem
                        x: tile.x
                        y: tile.y
                        width: tile.width
                        height: tile.height
                        Behavior on x { NumberAnimation { duration: 380; easing.type: Easing.OutCubic } }
                        Behavior on y { NumberAnimation { duration: 380; easing.type: Easing.OutCubic } }
                        Behavior on width { NumberAnimation { duration: 380; easing.type: Easing.OutCubic } }
                        Behavior on height { NumberAnimation { duration: 380; easing.type: Easing.OutCubic } }

                        function appId() {
                            return frame.shellSurface && frame.shellSurface.toplevel ? frame.shellSurface.toplevel.appId : ""
                        }
                        function sendSize() {
                            if (frame.shellSurface && frame.shellSurface.toplevel)
                                frame.shellSurface.toplevel.sendMaximized(Qt.size(Math.max(1, width - 2), Math.max(1, height - 36)))
                        }
                        onWidthChanged: sendSize()
                        onHeightChanged: sendSize()
                        onXChanged: compositor.stage.updateSurface(surfaceItem, appId())
                        onYChanged: compositor.stage.updateSurface(surfaceItem, appId())

                        Rectangle {
                            anchors.fill: parent
                            radius: 12
                            color: "#0b1119"
                            border.width: 1
                            border.color: frame.focused ? compositor.cyan : compositor.edge
                            Behavior on border.color { ColorAnimation { duration: 200 } }
                        }
                        Row {
                            x: 14
                            y: 9
                            spacing: 8
                            Rectangle { width: 8; height: 8; radius: 4; y: 5; color: frame.focused ? compositor.cyan : compositor.dim }
                            Text {
                                text: frame.appId().length ? frame.appId() : "aplicación sin nombre"
                                color: compositor.soft
                                font.family: "monospace"
                                font.pixelSize: 13
                            }
                        }

                        // The surface sits in its own container. When a client acknowledges the
                        // maximized state, its item moves itself to the output's origin; inside this
                        // container that origin is the content area rather than the frame's corner,
                        // so the window cannot slide up over the header.
                        Item {
                            id: content
                            x: 1
                            y: 35
                            width: frame.width - 2
                            height: frame.height - 36
                            clip: true
                            ShellSurfaceItem {
                                id: surfaceItem
                                shellSurface: frame.shellSurface
                                autoCreatePopupItems: true
                                // Only a person in control reaches applications through the
                                // scene; the agent's input goes through the seat directly.
                                inputEventsEnabled: compositor.stage.physicalInput
                                onSurfaceDestroyed: {
                                    compositor.stage.unregisterSurface(surfaceItem)
                                    shellSurfaces.remove(frame.index)
                                }
                                onXChanged: compositor.stage.updateSurface(surfaceItem, frame.appId())
                                onYChanged: compositor.stage.updateSurface(surfaceItem, frame.appId())
                                onWidthChanged: compositor.stage.updateSurface(surfaceItem, frame.appId())
                                onHeightChanged: compositor.stage.updateSurface(surfaceItem, frame.appId())
                                Component.onCompleted: {
                                    frame.sendSize()
                                    compositor.stage.registerSurface(surfaceItem, frame.appId())
                                }
                            }
                        }
                        Connections {
                            target: frame.shellSurface ? frame.shellSurface.toplevel : null
                            function onAppIdChanged() { compositor.stage.updateSurface(surfaceItem, frame.appId()) }
                        }
                    }
                }
            }

            // ---- Screen space: never zoomed. ----

            // The status line.
            Rectangle {
                width: parent.width
                height: 48
                color: "#cc070c12"
                Row {
                    id: brand
                    anchors.verticalCenter: parent.verticalCenter
                    x: 24
                    spacing: 18
                    Text { text: "ARCHIE·OS"; color: compositor.ink; font.pixelSize: 15; font.weight: Font.DemiBold; font.letterSpacing: 4 }
                    Text { text: "STAGE"; color: compositor.dim; font.family: "monospace"; font.pixelSize: 12; font.letterSpacing: 3; anchors.verticalCenter: parent.verticalCenter }
                }
                // What Archie did lately, newest first: someone who looks up, or who has just
                // taken over, can see the last few actions without a log. Verbs and
                // applications only, like the caption.
                Item {
                    anchors.left: brand.right
                    anchors.leftMargin: 28
                    anchors.right: statusRight.left
                    anchors.rightMargin: 20
                    height: parent.height
                    clip: true
                    Row {
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8
                        Repeater {
                            model: compositor.stage.activity
                            delegate: Rectangle {
                                id: chip
                                required property var modelData
                                required property int index
                                readonly property color tint: modelData.verb === "refused" ? compositor.amber
                                                              : modelData.verb === "interrupted" ? compositor.red : compositor.green
                                // Older entries recede, so the newest reads first.
                                opacity: modelData.opacity * Math.max(0.35, 1.0 - index * 0.16)
                                width: chipRow.implicitWidth + 18
                                height: 24
                                radius: 12
                                color: "#0f1822"
                                border.color: compositor.edge
                                Row {
                                    id: chipRow
                                    anchors.centerIn: parent
                                    spacing: 6
                                    Rectangle { width: 6; height: 6; radius: 3; color: chip.tint; anchors.verticalCenter: parent.verticalCenter }
                                    Text {
                                        text: compositor.activityText(chip.modelData)
                                        color: compositor.soft
                                        font.pixelSize: 11
                                    }
                                }
                            }
                        }
                    }
                }
                Row {
                    id: statusRight
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 24
                    spacing: 14
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: compositor.stage.surfaceCount === 1 ? "1 aplicación" : compositor.stage.surfaceCount + " aplicaciones"
                        color: compositor.dim
                        font.family: "monospace"
                        font.pixelSize: 11
                    }
                    Rectangle {
                        readonly property color tint: compositor.stage.authority === "autonomous" ? compositor.cyan
                                                      : compositor.stage.authority === "operator" ? compositor.amber : compositor.red
                        width: authorityText.implicitWidth + 22
                        height: 26
                        radius: 13
                        color: "transparent"
                        border.color: tint
                        Text {
                            id: authorityText
                            anchors.centerIn: parent
                            text: compositor.stage.authority === "autonomous" ? "ARCHIE TRABAJA"
                                  : compositor.stage.authority === "operator" ? "CONTROL HUMANO" : "PAUSADO"
                            color: parent.tint
                            font.family: "monospace"
                            font.pixelSize: 11
                            font.bold: true
                            font.letterSpacing: 1.5
                        }
                    }
                }
            }

            // An empty stage says what it is waiting for, instead of looking broken.
            Column {
                visible: compositor.stage.surfaceCount === 0
                anchors.centerIn: parent
                spacing: 10
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: "Archie Stage"; color: compositor.ink; font.pixelSize: 34; font.weight: Font.Light; font.letterSpacing: 3 }
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: "Las aplicaciones que Archie use aparecerán aquí."; color: compositor.soft; font.pixelSize: 16 }
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "WAYLAND_DISPLAY=" + compositor.socketName + (compositor.stage.listening ? "  ·  control listo" : "  ·  sin control: Archie no puede actuar")
                    color: compositor.dim
                    font.family: "monospace"
                    font.pixelSize: 12
                }
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: "Mueve el mouse o pulsa una tecla para tomar el control · Ctrl+Alt+Esc pausa · Ctrl+Alt+Enter reanuda"; color: compositor.dim; font.pixelSize: 12 }
            }

            // Ripples: a press that happened, expanding once.
            Repeater {
                model: compositor.stage.ripples
                delegate: Rectangle {
                    required property var modelData
                    x: modelData.x - modelData.r
                    y: modelData.y - modelData.r
                    width: modelData.r * 2
                    height: modelData.r * 2
                    radius: modelData.r
                    color: "transparent"
                    border.width: 3
                    border.color: compositor.cyan
                    opacity: modelData.opacity
                }
            }

            // A drag being carried.
            Shape {
                anchors.fill: parent
                visible: compositor.stage.dragging && compositor.stage.cursorVisible
                ShapePath {
                    strokeColor: compositor.cyan
                    strokeWidth: 2
                    strokeStyle: ShapePath.DashLine
                    dashPattern: [4, 3]
                    fillColor: "transparent"
                    startX: compositor.stage.dragFromX
                    startY: compositor.stage.dragFromY
                    PathLine { x: compositor.stage.cursorX; y: compositor.stage.cursorY }
                }
            }

            // The agent's cursor. Only while it holds the stage.
            Item {
                x: compositor.stage.cursorX
                y: compositor.stage.cursorY
                visible: compositor.stage.cursorVisible
                opacity: compositor.stage.cursorOpacity
                Rectangle {
                    x: -22; y: -22; width: 44; height: 44; radius: 22
                    color: compositor.cyan
                    opacity: compositor.stage.cursorPressed ? 0.35 : 0.0
                    Behavior on opacity { NumberAnimation { duration: 90 } }
                }
                Shape {
                    ShapePath {
                        strokeColor: "#04070c"
                        strokeWidth: 2
                        fillColor: compositor.ink
                        startX: 0; startY: 0
                        PathLine { x: 0; y: 28 }
                        PathLine { x: 7; y: 21 }
                        PathLine { x: 12; y: 33 }
                        PathLine { x: 18; y: 30 }
                        PathLine { x: 13; y: 19 }
                        PathLine { x: 23; y: 19 }
                        PathLine { x: 0; y: 0 }
                    }
                }
                Rectangle {
                    x: 20; y: 28; width: badge.implicitWidth + 12; height: 18; radius: 9
                    color: "#cc0b1119"; border.color: compositor.cyan
                    Text { id: badge; anchors.centerIn: parent; text: "Archie"; color: compositor.cyan; font.pixelSize: 10; font.bold: true }
                }
            }

            // The caption.
            Rectangle {
                visible: compositor.stage.captionOpacity > 0.01
                opacity: compositor.stage.captionOpacity
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.bottom: parent.bottom
                anchors.bottomMargin: 30
                width: Math.min(parent.width - 80, captionLabel.implicitWidth + 48)
                height: 52
                radius: 26
                color: "#e60b1119"
                border.color: compositor.stage.captionVerb === "refused" ? compositor.amber : "#3c5169"
                Text {
                    id: captionLabel
                    anchors.centerIn: parent
                    text: "Archie · " + compositor.captionText()
                    color: compositor.ink
                    font.pixelSize: 19
                }
            }

            // The banner: whenever the agent does not hold the stage.
            Rectangle {
                visible: compositor.stage.banner.length > 0
                opacity: compositor.stage.bannerOpacity
                anchors.horizontalCenter: parent.horizontalCenter
                y: 64
                width: bannerLabel.implicitWidth + 56
                height: 54
                radius: 10
                color: compositor.stage.banner === "paused" ? "#ee2b1216" : "#ee2a1f07"
                border.color: compositor.stage.banner === "paused" ? compositor.red : compositor.amber
                Text {
                    id: bannerLabel
                    anchors.centerIn: parent
                    text: compositor.stage.banner === "paused"
                          ? "PAUSADO · Archie no actuará hasta que pulses Ctrl+Alt+Enter"
                          : "CONTROL HUMANO · Archie retoma cuando dejes de usar el mouse y el teclado"
                    color: compositor.stage.banner === "paused" ? compositor.red : compositor.amber
                    font.pixelSize: 19
                }
            }
        }
    }
}
