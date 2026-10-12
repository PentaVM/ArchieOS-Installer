import QtQuick

// Why a control is disabled. Amber text on a raised pill; wraps instead of cutting the reason.
Rectangle {
    id: w
    property string text
    width: parent ? parent.width : 300
    implicitHeight: t.implicitHeight + 14
    height: implicitHeight
    radius: 8; color: Theme.raised; border.width: 1; border.color: Theme.hair
    Text { id: t; x: 12; y: 7; width: parent.width - 20; text: w.text; wrapMode: Text.Wrap
        color: Theme.warn; font.family: Theme.font; font.pixelSize: 11; lineHeight: 15; lineHeightMode: Text.FixedHeight }
}
