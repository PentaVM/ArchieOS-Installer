import QtQuick

// A status chip. Meaning is carried by glyph + word + colour, never colour alone.
Rectangle {
    id: c
    property string text: ""
    property string glyph: ""
    property color tone: Theme.unknown
    implicitHeight: 20
    implicitWidth: row.implicitWidth + 14
    radius: 10
    color: Qt.rgba(tone.r, tone.g, tone.b, 0.16)
    border.width: 1; border.color: Qt.rgba(tone.r, tone.g, tone.b, 0.45)
    Row {
        id: row; anchors.centerIn: parent; spacing: 5
        Text { visible: c.glyph !== ""; text: c.glyph; color: c.tone; font.pixelSize: Theme.fsSmall; anchors.verticalCenter: parent.verticalCenter }
        Text { text: c.text; color: Theme.ink; font.pixelSize: Theme.fsSmall; anchors.verticalCenter: parent.verticalCenter }
    }
}
