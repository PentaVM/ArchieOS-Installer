import QtQuick

// Vault: the file manager (prototype). Behaviour model: COSMIC Files; look: ArchieOS.
// Nothing here touches a filesystem. In phase C every mutating action goes to archie-vaultd,
// which asks Policy for a one-operation lease (docs/gui/ARCHITECTURE_GUI.md, ADR-GUI-003).
Panel {
    id: win
    property bool gridView: false
    property int place: 0
    property int selected: 1
    property bool queueOpen: true
    readonly property var kindGlyph: ({ folder: "▸", cad: "⬡", doc: "▤", video: "▶", sheet: "▦", text: "≡", image: "◩" })

    function domainTone(d) { return d === "work" ? Theme.warn : Theme.info }
    function domainText(d) { return d === "work" ? "Trabajo" : "Personal" }

    // Title bar
    Rectangle {
        id: title; height: 40; width: parent.width; color: Theme.raised; radius: win.radius
        Rectangle { y: parent.height - win.radius; height: win.radius; width: parent.width; color: parent.color }
        Row { x: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 8
            Rectangle { width: 12; height: 12; radius: 6; color: "#f2626b" }
            Rectangle { width: 12; height: 12; radius: 6; color: "#f0b429" }
            Rectangle { width: 12; height: 12; radius: 6; color: "#47d18b" }
        }
        Text { anchors.centerIn: parent; text: "Vault — Entregas"; color: Theme.ink; font.pixelSize: Theme.fsBody; font.bold: true }
        Row { anchors.right: parent.right; anchors.rightMargin: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 10
            Rectangle { width: 26; height: 24; radius: 6; color: !win.gridView ? Theme.accent : "transparent"; border.width: 1; border.color: Theme.line
                Text { anchors.centerIn: parent; text: "☰"; color: !win.gridView ? "#07090d" : Theme.ink }
                MouseArea { anchors.fill: parent; onClicked: win.gridView = false } }
            Rectangle { width: 26; height: 24; radius: 6; color: win.gridView ? Theme.accent : "transparent"; border.width: 1; border.color: Theme.line
                Text { anchors.centerIn: parent; text: "▦"; color: win.gridView ? "#07090d" : Theme.ink }
                MouseArea { anchors.fill: parent; onClicked: win.gridView = true } }
        }
    }
    // Tabs + breadcrumbs
    Rectangle { id: tabs; y: 40; height: 34; width: parent.width; color: Theme.surface
        Row { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 6
            Rectangle { width: 130; height: 24; radius: 6; color: Theme.raised; border.width: 1; border.color: Theme.accent
                Text { anchors.centerIn: parent; text: "Entregas"; color: Theme.ink; font.pixelSize: Theme.fsSmall } }
            Rectangle { width: 24; height: 24; radius: 6; color: "transparent"; border.width: 1; border.color: Theme.line
                Text { anchors.centerIn: parent; text: "+"; color: Theme.soft } }
        }
        Text { anchors.right: parent.right; anchors.rightMargin: 14; anchors.verticalCenter: parent.verticalCenter
            text: "Inicio  ›  Archie  ›  motor-bracket"; color: Theme.soft; font.pixelSize: Theme.fsSmall }
    }

    // Sidebar
    Rectangle { id: side; x: 0; y: 74; width: 190; height: parent.height - 74 - (win.queueOpen ? queue.height : 0); color: Theme.surface
        Column { x: 10; y: 10; spacing: 2; width: parent.width - 20
            Text { text: "UBICACIONES"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; font.letterSpacing: 1.2; bottomPadding: 4 }
            Repeater { model: Synthetic.places
                delegate: Rectangle { width: parent.width; height: 30; radius: 6
                    color: win.place === index ? Theme.raised : "transparent"
                    Row { x: 8; anchors.verticalCenter: parent.verticalCenter; spacing: 8
                        Text { text: modelData.icon; color: win.place === index ? Theme.accent : Theme.soft; font.pixelSize: Theme.fsBody }
                        Text { text: modelData.name; color: Theme.ink; font.pixelSize: Theme.fsBody }
                    }
                    Rectangle { visible: modelData.domain !== ""; anchors.right: parent.right; anchors.rightMargin: 8; anchors.verticalCenter: parent.verticalCenter
                        width: 8; height: 8; radius: 4; color: win.domainTone(modelData.domain) }
                    MouseArea { anchors.fill: parent; onClicked: win.place = index }
                } }
            Item { width: 1; height: 12 }
            Text { text: "SISTEMA"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; font.letterSpacing: 1.2; bottomPadding: 4 }
            // The Sealed store is a status row, never a browsable folder (GUI-VAULT-023).
            Rectangle { width: parent.width; height: 44; radius: 6; color: "transparent"; border.width: 1; border.color: Theme.line
                Column { x: 8; anchors.verticalCenter: parent.verticalCenter; spacing: 1
                    Text { text: "✦ Sealed"; color: Theme.ink; font.pixelSize: Theme.fsBody }
                    Text { text: "almacén cifrado · sólo estado"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1 } } }
        }
    }

    // Content
    Rectangle { x: side.width; y: 74; width: parent.width - side.width; height: side.height; color: Theme.bg
        // List header
        Row { visible: !win.gridView; x: 16; y: 8; spacing: 0
            Text { width: 330; text: "Nombre"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
            Text { width: 100; text: "Tamaño"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
            Text { width: 110; text: "Modificado"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
            Text { text: "Contexto"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
        }
        Column { visible: !win.gridView; x: 8; y: 30; width: parent.width - 16; spacing: 1
            Repeater { model: Synthetic.files
                delegate: Rectangle { width: parent.width; height: Math.round(30 * Theme.density + 4); radius: 6
                    color: win.selected === index ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.22) : "transparent"
                    border.width: win.selected === index ? 1 : 0; border.color: Theme.accent
                    Row { x: 8; anchors.verticalCenter: parent.verticalCenter
                        Row { width: 322; spacing: 8
                            Text { text: win.kindGlyph[modelData.kind]; color: modelData.kind === "folder" ? Theme.accent : Theme.soft; font.pixelSize: Theme.fsBody }
                            Text { text: modelData.name; color: Theme.ink; font.pixelSize: Theme.fsBody; elide: Text.ElideRight }
                        }
                        Text { width: 100; text: modelData.size; color: Theme.soft; font.pixelSize: Theme.fsBody; font.features: { "tnum": 1 } }
                        Text { width: 110; text: modelData.when; color: Theme.soft; font.pixelSize: Theme.fsBody }
                        Row { spacing: 6
                            Chip { visible: modelData.domain !== ""; text: win.domainText(modelData.domain); glyph: "◐"; tone: win.domainTone(modelData.domain) }
                            Chip { visible: modelData.archie; text: "Archie lo tocó"; glyph: "◉"; tone: Theme.accent }
                        }
                    }
                    MouseArea { anchors.fill: parent; onClicked: win.selected = index }
                } }
        }
        Flow { visible: win.gridView; x: 16; y: 16; width: parent.width - 32; spacing: 14
            Repeater { model: Synthetic.files
                delegate: Rectangle { width: 118; height: 112; radius: 10
                    color: win.selected === index ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.22) : "transparent"
                    border.width: win.selected === index ? 1 : 0; border.color: Theme.accent
                    Column { anchors.centerIn: parent; spacing: 6
                        Rectangle { anchors.horizontalCenter: parent.horizontalCenter; width: 60; height: 60; radius: 10; color: Theme.raised; border.width: 1; border.color: Theme.line
                            Text { anchors.centerIn: parent; text: win.kindGlyph[modelData.kind]; color: modelData.kind === "folder" ? Theme.accent : Theme.soft; font.pixelSize: 28 } }
                        Text { width: 108; horizontalAlignment: Text.AlignHCenter; text: modelData.name; color: Theme.ink; font.pixelSize: Theme.fsSmall; elide: Text.ElideMiddle }
                    }
                    MouseArea { anchors.fill: parent; onClicked: win.selected = index }
                } }
        }
    }

    // Operation queue (real progress only in phase C; here it is synthetic and labelled)
    Rectangle { id: queue; visible: win.queueOpen; x: 0; y: parent.height - height; width: parent.width; height: 74; color: Theme.surface
        border.width: 1; border.color: Theme.line; radius: win.radius
        Row { x: 16; y: 10; spacing: 24
            Column { spacing: 4; width: 330
                Text { text: "Copiando a Entregas · 3 de 9 archivos"; color: Theme.ink; font.pixelSize: Theme.fsSmall }
                Rectangle { width: parent.width; height: 6; radius: 3; color: Theme.raised
                    Rectangle { width: parent.width * 0.35; height: parent.height; radius: 3; color: Theme.accent } }
                Text { text: "Permiso: lectura + escritura · 1 operación · concedido por Policy (diseño)"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1 }
            }
            Rectangle { width: 84; height: 26; radius: 6; color: "transparent"; border.width: 1; border.color: Theme.line
                Text { anchors.centerIn: parent; text: "Cancelar"; color: Theme.ink; font.pixelSize: Theme.fsSmall } }
            Rectangle { width: 120; height: 26; radius: 6; color: "transparent"; border.width: 1; border.color: Theme.line
                Text { anchors.centerIn: parent; text: "Deshacer (si hay)"; color: Theme.dim; font.pixelSize: Theme.fsSmall } }
        }
    }
}
