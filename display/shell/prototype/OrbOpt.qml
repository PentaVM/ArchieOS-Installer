import QtQuick

// Settings row with mutually exclusive options (the picked one is highlighted).
Item {
    id: o
    property string label
    property var options: []
    property int index: 0
    signal picked(int i)
    width: parent ? parent.width : 400
    height: 34
    Text { anchors.verticalCenter: parent.verticalCenter; text: o.label; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13 }
    Row { anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; spacing: 6
        Repeater { model: o.options
            delegate: Rectangle { height: 26; width: t.implicitWidth + 22; radius: 8
                color: o.index === index ? Theme.sel : (ma.containsMouse ? Theme.pressed : Theme.hover)
                border.width: 1; border.color: o.index === index ? Theme.acc : Theme.hair
                Text { id: t; anchors.centerIn: parent; text: modelData; color: Theme.t1; font.family: Theme.font; font.pixelSize: 12 }
                MouseArea { id: ma; anchors.fill: parent; hoverEnabled: true; onClicked: o.picked(index) } } }
    }
}
