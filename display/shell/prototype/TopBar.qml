import QtQuick

Rectangle {
    id: bar
    height: Theme.barH
    color: Qt.rgba(0, 0, 0, 0.34)
    property string overlay: ""
    property var bridge
    signal toggle(string name)

    Row {
        anchors.left: parent.left; anchors.leftMargin: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 14
        Text { text: "◉  Archie"; color: Theme.ink; font.pixelSize: Theme.fsBody; font.bold: true }
        Text { text: "Vault"; color: Theme.soft; font.pixelSize: Theme.fsBody }
        Text { text: "Archivo   Editar   Vista   Ir   Ventana"; color: Theme.dim; font.pixelSize: Theme.fsBody }
    }
    Row {
        anchors.right: parent.right; anchors.rightMargin: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 12
        // Link state comes from the bridge. The privacy and domain chips have no source wired yet
        // (GUI-SENTRY-002, GUI-CEREBRA-002), so they say "sin lectura" instead of claiming a state.
        LinkChip { bridge: bar.bridge }
        Chip { text: "Cámara y micro: sin lectura"; glyph: "?"; tone: Theme.unknown }
        Chip { text: "Dominio: sin lectura"; glyph: "?"; tone: Theme.unknown }
        Text { text: "⌕"; color: Theme.ink; font.pixelSize: Theme.fsTitle; MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: bar.toggle("palette") } }
        // Running long jobs, from the bridge; "—" when no workspace is configured.
        Text { text: "◔ " + (bar.bridge.jobs.state === "ok" ? bar.bridge.runningJobs : "—"); color: bar.overlay === "activity" ? Theme.accent : Theme.ink; font.pixelSize: Theme.fsBody
            MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: bar.toggle("activity") } }
        Text { text: "☰"; color: bar.overlay === "control" ? Theme.accent : Theme.ink; font.pixelSize: Theme.fsTitle
            MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: bar.toggle("control") } }
        Text { text: "Sáb 11 oct  09:41"; color: Theme.ink; font.pixelSize: Theme.fsBody }
    }
}
