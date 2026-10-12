import QtQuick

// Domain mark: personal = circle (info), work = diamond (warn). Shape differs, not just colour.
Item {
    property string domain
    width: 7; height: 7
    Rectangle { visible: domain === "personal"; anchors.fill: parent; radius: 3.5; color: Theme.info }
    Rectangle { visible: domain === "work"; anchors.fill: parent; radius: 2; color: Theme.warn; rotation: 45 }
}
