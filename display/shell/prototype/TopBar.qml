import QtQuick

// Top bar, 32 px. Order is frozen: left = brand, app, menus; right = link chip, camera/mic,
// domain, search, activity count, control center, clock.
Rectangle {
    id: bar
    height: Theme.barH
    color: Theme.bar
    property string overlay: ""
    property string app: "Vault"
    property var bridge
    signal toggle(string name)
    property string menu: ""                 // name of the open drop-down, "" if none
    signal menuClicked(string name, real x)

    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.inner }

    component MenuItem: Rectangle {
        id: mi
        property string label
        objectName: "bar.menu." + label
        height: 24; radius: 6; width: mt.implicitWidth + 18
        color: bar.menu === label ? Theme.pressed : (mma.containsMouse ? Theme.hover : "transparent")
        Text { id: mt; anchors.centerIn: parent; text: mi.label; color: (bar.menu === mi.label || mma.containsMouse) ? Theme.t1 : Theme.t2; font.family: Theme.font; font.pixelSize: 13 }
        MouseArea { id: mma; anchors.fill: parent; hoverEnabled: true; onClicked: bar.menuClicked(mi.label, mi.mapToItem(bar, 0, 0).x) }
    }
    component IconButton: Rectangle {
        id: ib
        property string icon
        property string count: ""
        property bool active: false
        signal clicked()
        width: count === "" ? 28 : cr.implicitWidth + 16; height: 26; radius: 7
        color: active ? Theme.pressed : (ma.containsMouse ? Theme.hover : "transparent")
        Row { id: cr; anchors.centerIn: parent; spacing: 6
            Icon { name: ib.icon; size: 16; color: ib.active || ma.containsMouse || ib.count !== "" ? Theme.t1 : Theme.t2; anchors.verticalCenter: parent.verticalCenter }
            Text { visible: ib.count !== ""; text: ib.count; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.DemiBold; font.features: { "tnum": 1 }; anchors.verticalCenter: parent.verticalCenter }
        }
        MouseArea { id: ma; anchors.fill: parent; hoverEnabled: true; onClicked: ib.clicked() }
    }

    Row {
        x: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 4
        Row { spacing: 8; rightPadding: 14; anchors.verticalCenter: parent.verticalCenter
            Icon { name: "archie"; size: 18; color: Theme.t1; anchors.verticalCenter: parent.verticalCenter }
            Text { text: "Archie"; color: Theme.t1; font.family: Theme.font; font.pixelSize: 14; font.weight: Font.DemiBold; anchors.verticalCenter: parent.verticalCenter } }
        Text { text: bar.app; rightPadding: 12; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.DemiBold; anchors.verticalCenter: parent.verticalCenter }
        MenuItem { label: "Archivo"; anchors.verticalCenter: parent.verticalCenter } MenuItem { label: "Editar"; anchors.verticalCenter: parent.verticalCenter } MenuItem { label: "Vista"; anchors.verticalCenter: parent.verticalCenter } MenuItem { label: "Ir"; anchors.verticalCenter: parent.verticalCenter } MenuItem { label: "Ventana"; anchors.verticalCenter: parent.verticalCenter }
    }
    Row {
        anchors.right: parent.right; anchors.rightMargin: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 6
        ThemeToggle { objectName: "bar.theme"; compact: true; anchors.verticalCenter: parent.verticalCenter }
        LinkChip { bridge: bar.bridge; anchors.verticalCenter: parent.verticalCenter }
        // No source is wired for these two yet (GUI-SENTRY-002, GUI-CEREBRA-002): they say so.
        Chip { text: "Cámara y micro"; sub: "sin lectura"; icon: "unknown"; tone: "unk"; anchors.verticalCenter: parent.verticalCenter }
        // Opens the workspace menu (decision 5 of the handoff: the selector lives in this chip).
        Item { objectName: "bar.domain"; width: dom.width; height: dom.height; anchors.verticalCenter: parent.verticalCenter
            Chip { id: dom; text: "Dominio"; sub: "sin lectura"; icon: "unknown"; tone: "unk" }
            MouseArea { anchors.fill: parent; onClicked: bar.toggle("workspace") } }
        Item { width: 13; height: 14; anchors.verticalCenter: parent.verticalCenter
            Rectangle { anchors.centerIn: parent; width: 1; height: 14; color: Theme.hair } }
        IconButton { objectName: "bar.palette"; icon: "search"; active: bar.overlay === "palette"; onClicked: bar.toggle("palette"); anchors.verticalCenter: parent.verticalCenter }
        // Running long jobs from the bridge; "—" when no workspace is configured, never a made-up 0.
        IconButton { objectName: "bar.activity"; icon: "activity"; count: bar.bridge.jobs.state === "ok" ? String(bar.bridge.runningJobs) : "—"
            active: bar.overlay === "activity"; onClicked: bar.toggle("activity"); anchors.verticalCenter: parent.verticalCenter }
        IconButton { objectName: "bar.control"; icon: "control"; active: bar.overlay === "control"; onClicked: bar.toggle("control"); anchors.verticalCenter: parent.verticalCenter }
        Text { leftPadding: 8; text: Demo.barClock; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium
            font.features: { "tnum": 1 }; anchors.verticalCenter: parent.verticalCenter }
    }
}
