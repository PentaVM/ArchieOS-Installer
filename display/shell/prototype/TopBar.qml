import QtQuick

Rectangle {
    id: bar
    height: Theme.barH
    color: Qt.rgba(0, 0, 0, 0.34)
    property string overlay: ""
    signal toggle(string name)

    Row {
        anchors.left: parent.left; anchors.leftMargin: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 14
        Text { text: "◉  Archie"; color: Theme.ink; font.pixelSize: Theme.fsBody; font.bold: true }
        Text { text: "Vault"; color: Theme.soft; font.pixelSize: Theme.fsBody }
        Text { text: "Archivo   Editar   Vista   Ir   Ventana"; color: Theme.dim; font.pixelSize: Theme.fsBody }
    }
    Row {
        anchors.right: parent.right; anchors.rightMargin: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 12
        // Honest link state. In the prototype nothing is connected, and it says so.
        Chip { text: "DISEÑO · sin conexión a archied"; glyph: "○"; tone: Theme.unknown }
        Chip { text: "Cámara y micro inactivos"; glyph: "●"; tone: Theme.ok }
        Chip { text: "Personal"; glyph: "◐"; tone: Theme.info }
        Text { text: "⌕"; color: Theme.ink; font.pixelSize: Theme.fsTitle; MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: bar.toggle("palette") } }
        Text { text: "◔ 3"; color: bar.overlay === "activity" ? Theme.accent : Theme.ink; font.pixelSize: Theme.fsBody
            MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: bar.toggle("activity") } }
        Text { text: "☰"; color: bar.overlay === "control" ? Theme.accent : Theme.ink; font.pixelSize: Theme.fsTitle
            MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: bar.toggle("control") } }
        Text { text: "Sáb 11 oct  09:41"; color: Theme.ink; font.pixelSize: Theme.fsBody }
    }
}
