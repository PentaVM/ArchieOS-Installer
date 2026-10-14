import QtQuick

// Workspace selector, a 300 px menu under the "Dominio" chip of the bar. DESIGN SAMPLE.
// The switch itself is done by the compositor (as archie-session.sh does today); the shell only
// shows the menu. The Orbe is one of the entries and is never drawn here.
Panel {
    id: wm
    kind: "panel"
    width: 300
    height: col.implicitHeight + 16
    enterScale: 1; enterDy: -8
    property string st: Demo.state
    property string sel: "work"
    signal chosen(string which)

    component Row2: Rectangle {
        id: rw
        property string domain
        property string icon
        property string name
        property bool on: false
        property bool ring: false
        default property alias trailing: rt.data
        signal clicked()
        width: parent ? parent.width : 280; height: 42; radius: 9
        color: (on || ring) ? Theme.sel : "transparent"
        Rectangle { anchors.fill: parent; radius: 9; color: "transparent"; border.width: 2; border.color: Theme.acc; visible: rw.ring }
        Row { x: 10; anchors.verticalCenter: parent.verticalCenter; spacing: 10
            Icon { visible: rw.icon !== ""; name: rw.icon; size: 16; color: Theme.t2; anchors.verticalCenter: parent.verticalCenter }
            DomainDot { visible: rw.domain !== ""; domain: rw.domain; anchors.verticalCenter: parent.verticalCenter }
            Text { text: rw.name; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter } }
        Row { id: rt; anchors.right: parent.right; anchors.rightMargin: 10; anchors.verticalCenter: parent.verticalCenter; spacing: 6 }
        MouseArea { anchors.fill: parent; hoverEnabled: true; onClicked: rw.clicked() }
    }

    Column { id: col; x: 8; y: 8; width: parent.width - 16; spacing: 0
        Eyebrow { text: "Espacio de trabajo"; leftPadding: 10; topPadding: 8; bottomPadding: 6 }
        Row2 { icon: "dkOrb"; name: "Orbe"; on: wm.st === "vacio" || wm.sel === "orb"; ring: Demo.focusRing; onClicked: { wm.sel = "orb"; wm.chosen("orb") } }
        Row2 { visible: wm.st !== "vacio"; domain: "personal"; name: "Personal"; on: wm.sel === "personal"; onClicked: { wm.sel = "personal"; wm.chosen("personal") }
            Icon { visible: wm.st !== "offline" && wm.sel === "personal"; name: "done"; size: 14; color: Theme.ok }
            Chip { visible: wm.st === "offline"; small: true; tone: "unk"; icon: "unknown"; text: "Archie"; sub: "sin lectura" }
            Chip { visible: wm.st === "degraded"; small: true; tone: "unk"; icon: "unknown"; text: "ventanas"; sub: "sin lectura" } }
        Row2 { visible: wm.st !== "vacio"; domain: "work"; name: "Trabajo"; on: wm.sel === "work"; onClicked: { wm.sel = "work"; wm.chosen("work") }
            Chip { visible: wm.st === "offline"; small: true; tone: "unk"; icon: "unknown"; text: "Archie"; sub: "sin lectura" }
            Icon { visible: wm.st !== "offline" && wm.sel === "work"; name: "done"; size: 14; color: Theme.ok } }
        Item { visible: wm.st === "vacio"; width: parent.width; height: vb.height + 12
            VoidBox { id: vb; x: 4; y: 8; width: parent.width - 8; icon: "spaces"; title: "Sin espacios de trabajo"; text: "Solo existe el Orbe. Cuando haya espacios aparecerán aquí." } }
        Item { visible: wm.st === "error"; width: parent.width; height: eb.height + 8
            ErrBox { id: eb; x: 4; y: 8; width: parent.width - 8; text: "El compositor no respondió al cambio de espacio. Sigues en Trabajo." } }
        Item { visible: wm.st === "stale"; width: parent.width; height: nb.height + 8
            NoteBox { id: nb; x: 4; y: 8; width: parent.width - 8; icon: "recents"; text: "Lista de espacios leída hace 18 min." } }
        Row { leftPadding: 10; topPadding: 12; bottomPadding: 4; spacing: 4
            Kbd { label: "↑↓"; anchors.verticalCenter: parent.verticalCenter } Text { text: "elegir"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12; anchors.verticalCenter: parent.verticalCenter; leftPadding: 2 }
            Kbd { label: "↵"; anchors.verticalCenter: parent.verticalCenter } Text { text: "cambiar"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12; anchors.verticalCenter: parent.verticalCenter; leftPadding: 2 }
            Kbd { label: "Esc"; anchors.verticalCenter: parent.verticalCenter } Text { text: "salir"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12; anchors.verticalCenter: parent.verticalCenter; leftPadding: 2 } }
    }
}
