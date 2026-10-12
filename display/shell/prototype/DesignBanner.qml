import QtQuick

// Shown on every prototype screen: this data is not from the system.
Rectangle {
    height: 22; radius: 4
    width: label.implicitWidth + 20
    color: "#33f0b429"; border.width: 1; border.color: Theme.warn
    Text {
        id: label; anchors.centerIn: parent
        text: "▲ DATOS DE DISEÑO · no son del sistema"
        color: Theme.warn; font.pixelSize: Theme.fsSmall; font.bold: true
    }
}
