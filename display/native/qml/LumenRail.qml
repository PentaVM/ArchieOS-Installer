import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The design's left rail: where the field comes from, search, the types, the modules
// that can be opened, and the structure under the current focus.
Rectangle {
    id: rail
    color: "#101820"

    readonly property color ink: "#dce5e8"
    readonly property color muted: "#81949e"
    readonly property color label: "#91a4ae"
    readonly property color accent: "#f0be7e"
    readonly property color line: "#25323b"
    property var results: []

    function focusSearch() { searchField.forceActiveFocus() }

    // Search runs over the model's own frame: every visible entity, not only the ones the
    // current view has unfolded. Choosing one opens whatever hides it.
    function search(text) {
        const needle = text.trim().toLowerCase()
        if (needle.length === 0) { rail.results = []; return }
        const nodes = (archieBridge.orb && archieBridge.orb.nodes) ? archieBridge.orb.nodes : []
        const found = []
        for (let i = 0; i < nodes.length && found.length < 40; ++i) {
            const n = nodes[i]
            if (String(n.label).toLowerCase().indexOf(needle) >= 0 || String(n.id).toLowerCase().indexOf(needle) >= 0)
                found.push(n)
        }
        rail.results = found
    }
    function ancestorsOf(id) {
        const nodes = (archieBridge.orb && archieBridge.orb.nodes) ? archieBridge.orb.nodes : []
        const parent = {}
        for (let i = 0; i < nodes.length; ++i) parent[nodes[i].id] = nodes[i].parent_id
        const chain = []
        let at = parent[id]
        for (let guard = 0; at && guard < 64; ++guard) { chain.unshift(at); at = parent[at] }
        return chain
    }

    Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: rail.line }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Brand
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 70
            Row {
                anchors.left: parent.left; anchors.leftMargin: 20
                anchors.verticalCenter: parent.verticalCenter
                spacing: 12
                Hexagon { width: 34; height: 34; stroke: rail.accent; glyph: "A" }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    Text { text: "ARCHIE"; color: rail.ink; font.pixelSize: 16; font.bold: true; font.letterSpacing: 2.2 }
                    Text { text: "LUMENFIELD / TFOG"; color: rail.muted; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.6 }
                }
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: rail.line }
        }

        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentWidth: availableWidth

            Column {
                width: rail.width - 1
                spacing: 0

                // Origin
                Section {
                    title: "ORIGEN"; note: lumen.loaded ? "ACTIVO" : "ESPERANDO"
                    RailButton {
                        width: parent.width
                        height: 56
                        onClicked: lumen.explore(lumen.scene.rootId || "")
                        Column {
                            anchors.left: parent.left; anchors.leftMargin: 12
                            anchors.right: parent.right; anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 3
                            Text { width: parent.width; text: "ArchieOS · campo TFOG"; color: rail.ink; font.pixelSize: 12; font.bold: true; elide: Text.ElideRight }
                            Text {
                                width: parent.width
                                text: lumen.scene.focusName ? "foco: " + lumen.scene.focusName : "todo el sistema observado"
                                color: rail.muted; font.pixelSize: 10; elide: Text.ElideRight
                            }
                        }
                    }
                }

                // Search
                Section {
                    title: "EXPLORAR"
                    Rectangle {
                        width: parent.width; height: 36; radius: 5
                        color: "#0c141b"; border.color: searchField.activeFocus ? "#d6a873" : "#34444c"
                        TextField {
                            id: searchField
                            objectName: "lumenSearch"
                            anchors.fill: parent; anchors.leftMargin: 8; anchors.rightMargin: 28
                            placeholderText: "Buscar entidad, proceso o servicio"
                            placeholderTextColor: "#6c838e"
                            color: rail.ink; font.pixelSize: 11
                            background: Item {}
                            onTextChanged: searchDelay.restart()
                            Keys.onEscapePressed: { text = ""; focus = false }
                        }
                        Text { anchors.right: parent.right; anchors.rightMargin: 10; anchors.verticalCenter: parent.verticalCenter; text: "/"; color: "#77909a"; font.pixelSize: 10 }
                        Timer { id: searchDelay; interval: 180; onTriggered: rail.search(searchField.text) }
                    }
                    Column {
                        width: parent.width
                        visible: rail.results.length > 0 || searchField.text.length > 0
                        spacing: 2
                        Repeater {
                            model: rail.results
                            delegate: RailButton {
                                required property var modelData
                                width: parent ? parent.width : 200
                                height: 30
                                onClicked: {
                                    lumen.reveal(modelData.id, rail.ancestorsOf(modelData.id), modelData.kind)
                                    searchField.text = ""
                                }
                                Text {
                                    anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 10
                                    verticalAlignment: Text.AlignVCenter
                                    text: modelData.label + "  ·  " + modelData.kind
                                    color: rail.ink; font.pixelSize: 10; elide: Text.ElideMiddle
                                }
                            }
                        }
                        Text { visible: rail.results.length === 0; text: "Sin resultados"; color: rail.muted; font.pixelSize: 10 }
                    }
                }

                // Types
                Section {
                    title: "TIPOS"; note: (lumen.scene.nodeCount || 0) + " NODOS LÓGICOS"
                    Repeater {
                        model: lumen.categories
                        delegate: RailButton {
                            required property var modelData
                            width: parent ? parent.width : 200
                            height: 28
                            opacity: modelData.hidden ? 0.42 : 1.0
                            onClicked: lumen.toggleCategory(modelData.key)
                            Row {
                                anchors.left: parent.left; anchors.leftMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 10
                                Rectangle {
                                    width: 9; height: 9; radius: 5; anchors.verticalCenter: parent.verticalCenter
                                    color: modelData.color
                                    border.color: Qt.lighter(modelData.color, 1.4); border.width: 1
                                }
                                Text { text: modelData.label; color: rail.ink; font.pixelSize: 11 }
                            }
                            Text {
                                anchors.right: parent.right; anchors.rightMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                text: modelData.count; color: rail.muted; font.pixelSize: 10
                            }
                        }
                    }
                }

                // Modules
                Section {
                    title: "MÓDULOS"; note: "EXPANSIÓN INTERACTIVA"
                    visible: lumen.lens !== "entities"
                    Repeater {
                        model: lumen.modules
                        delegate: RailButton {
                            required property var modelData
                            width: parent ? parent.width : 200
                            height: 44
                            active: lumen.selection.id === modelData.id
                            onClicked: { lumen.select(modelData.id); lumen.focusRequested(lumen.positionOf(modelData.id)) }
                            onDoubleClicked: lumen.toggleModule(modelData.id)
                            Rectangle {
                                id: glyphBox
                                anchors.left: parent.left; anchors.leftMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                width: 24; height: 24; radius: 4
                                color: "transparent"; border.color: modelData.color
                                Text { anchors.centerIn: parent; text: modelData.kind === "group" ? "◌" : "⬡"; color: modelData.color; font.pixelSize: 12 }
                            }
                            Column {
                                anchors.left: glyphBox.right; anchors.leftMargin: 10
                                anchors.right: toggle.left; anchors.rightMargin: 6
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 2
                                Text { width: parent.width; text: modelData.name; color: rail.ink; font.pixelSize: 11; font.bold: true; elide: Text.ElideRight }
                                Text { width: parent.width; text: modelData.role + " · " + modelData.memberCount + " elementos"; color: rail.muted; font.pixelSize: 9; elide: Text.ElideRight }
                            }
                            RailButton {
                                id: toggle
                                anchors.right: parent.right; anchors.rightMargin: 6
                                anchors.verticalCenter: parent.verticalCenter
                                width: 24; height: 24
                                visible: modelData.expandable || modelData.expanded
                                onClicked: lumen.toggleModule(modelData.id)
                                Text { anchors.centerIn: parent; text: modelData.expanded ? "−" : "+"; color: rail.accent; font.pixelSize: 14; font.bold: true }
                            }
                        }
                    }
                    Text {
                        visible: lumen.modules.length === 0
                        width: parent.width; wrapMode: Text.Wrap
                        text: "Ningún módulo en esta vista."
                        color: rail.muted; font.pixelSize: 10
                    }
                }

                // Structure
                Section {
                    title: "ESTRUCTURA"
                    note: lumen.scene.canGoUp ? "↑ SUBIR" : ""
                    onNoteClicked: lumen.up()
                    Repeater {
                        model: lumen.tree
                        delegate: RailButton {
                            required property var modelData
                            width: parent ? parent.width : 200
                            height: 28
                            active: modelData.current
                            onClicked: lumen.explore(modelData.id)
                            Text {
                                anchors.fill: parent; anchors.leftMargin: modelData.current ? 10 : 22; anchors.rightMargin: 8
                                verticalAlignment: Text.AlignVCenter
                                text: (modelData.current ? "◉  " : "▸  ") + modelData.name
                                color: modelData.current ? rail.accent : rail.ink; font.pixelSize: 11; elide: Text.ElideRight
                            }
                        }
                    }
                }
            }
        }

        // Footer
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 34
            color: "#0d151c"
            Rectangle { width: parent.width; height: 1; color: rail.line }
            Text {
                anchors.left: parent.left; anchors.leftMargin: 20; anchors.verticalCenter: parent.verticalCenter
                text: (lumen.scene.entityCount || 0) + " ENTIDADES REGISTRADAS"
                color: rail.muted; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.2
            }
            Text {
                anchors.right: parent.right; anchors.rightMargin: 16; anchors.verticalCenter: parent.verticalCenter
                text: lumen.error.length > 0 ? "SIN CAMPO" : "LISTO"
                color: lumen.error.length > 0 ? "#f2626b" : rail.muted; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.2
            }
        }
    }

    component Section: Column {
        id: section
        property string title: ""
        property string note: ""
        signal noteClicked()
        default property alias content: body.data
        width: parent ? parent.width : 200
        topPadding: 16
        bottomPadding: 16
        leftPadding: 20
        rightPadding: 20
        spacing: 10
        Item {
            width: section.width - 40; height: 14
            Text { text: section.title; color: "#91a4ae"; font.pixelSize: 10; font.bold: true; font.letterSpacing: 1.8 }
            Text {
                anchors.right: parent.right
                text: section.note; color: "#6c838e"; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.2
                MouseArea { anchors.fill: parent; onClicked: section.noteClicked() }
            }
        }
        Column { id: body; width: section.width - 40; spacing: 4 }
        Rectangle { width: section.width - 40; height: 0 }
    }

    component RailButton: Rectangle {
        id: button
        property bool active: false
        signal clicked()
        signal doubleClicked()
        radius: 5
        color: active ? "#223440" : (area.containsMouse ? "#1c2a33" : "#18232c")
        border.color: active ? "#d6a873" : "#2c3b44"
        MouseArea {
            id: area
            anchors.fill: parent
            hoverEnabled: true
            onClicked: button.clicked()
            onDoubleClicked: button.doubleClicked()
        }
    }

    component Hexagon: Canvas {
        property color stroke: "#f0be7e"
        property string glyph: ""
        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            ctx.strokeStyle = stroke
            ctx.lineWidth = 2
            ctx.beginPath()
            const w = width, h = height
            ctx.moveTo(w * 0.5, 1); ctx.lineTo(w - 1, h * 0.22); ctx.lineTo(w - 1, h * 0.78)
            ctx.lineTo(w * 0.5, h - 1); ctx.lineTo(1, h * 0.78); ctx.lineTo(1, h * 0.22); ctx.closePath()
            ctx.stroke()
        }
        Text { anchors.centerIn: parent; text: parent.glyph; color: parent.stroke; font.pixelSize: 17; font.bold: true }
    }
}
