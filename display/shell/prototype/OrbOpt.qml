import QtQuick
import QtQuick.Controls

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
    ComboBox {
        visible:o.options.length>7
        anchors.right:parent.right; anchors.verticalCenter:parent.verticalCenter
        width:240; height:30; model:o.options; currentIndex:o.index
        onActivated:function(index){o.picked(index)}
        font.family:Theme.font; font.pixelSize:12
        contentItem:Text { text:parent.displayText; color:Theme.t1; font:parent.font; verticalAlignment:Text.AlignVCenter; leftPadding:10 }
        background:Rectangle { color:Theme.hover; radius:8; border.color:Theme.hair }
    }
    Row { visible:o.options.length<=7; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; spacing: 6
        Repeater { model: o.options
            delegate: Rectangle { height: 26; width: t.implicitWidth + 22; radius: 8
                color: o.index === index ? Theme.sel : (ma.containsMouse ? Theme.pressed : Theme.hover)
                border.width: 1; border.color: o.index === index ? Theme.acc : Theme.hair
                Text { id: t; anchors.centerIn: parent; text: modelData; color: Theme.t1; font.family: Theme.font; font.pixelSize: 12 }
                MouseArea { id: ma; anchors.fill: parent; hoverEnabled: true; onClicked: o.picked(index) } } }
    }
}
