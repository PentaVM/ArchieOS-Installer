import QtQuick

// Vault window (file manager) in the Archie Mac look. List or grid; sidebar with the Sealed
// store as a status row only; operation queue; footer with provenance. Everything here is
// DESIGN data (no file backend exists yet) and says so. Nothing touches a filesystem.
Panel {
    id: win
    kind: "window"
    property bool gridView: false
    property int selected: 1
    property int place: 0
    readonly property color noteColor: Theme.t3

    function catColor(f) { return f.cat === "" ? Theme.t2 : Theme.cat[f.cat] }

    component DomainMark: Item {
        property string domain
        width: 7; height: 7
        Rectangle { visible: domain === "personal"; anchors.fill: parent; radius: 3.5; color: Theme.info }
        Rectangle { visible: domain === "work"; anchors.fill: parent; radius: 2; color: Theme.warn; rotation: 45 }
    }

    // ---- title bar -------------------------------------------------------------------------
    Item { id: tbar; width: parent.width; height: 52
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
        Row { x: 16; anchors.verticalCenter: parent.verticalCenter; spacing: 8
            Rectangle { width: 12; height: 12; radius: 6; color: Theme.tlClose; border.width: 1; border.color: Qt.rgba(0, 0, 0, 0.22) }
            Rectangle { width: 12; height: 12; radius: 6; color: Theme.tlMin; border.width: 1; border.color: Qt.rgba(0, 0, 0, 0.22) }
            Rectangle { width: 12; height: 12; radius: 6; color: Theme.tlZoom; border.width: 1; border.color: Qt.rgba(0, 0, 0, 0.22) } }
        Row { anchors.centerIn: parent
            Text { text: "Vault"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.DemiBold; font.letterSpacing: -0.17 }
            Text { text: " — Entregas"; color: Theme.t2; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.Medium; font.letterSpacing: -0.17 } }
        Rectangle { id: seg; anchors.right: parent.right; anchors.rightMargin: 16; anchors.verticalCenter: parent.verticalCenter
            width: 76; height: 28; radius: 9; color: Theme.raised; border.width: 1; border.color: Theme.hair
            Rectangle { x: win.gridView ? 38 : 2; y: 2; width: 34; height: 24; radius: 7; color: Theme.pressed
                Behavior on x { NumberAnimation { duration: Theme.durNav; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.easeStd } } }
            Item { x: 2; y: 2; width: 34; height: 24; Icon { anchors.centerIn: parent; name: "list"; size: 16; color: win.gridView ? Theme.t3 : Theme.t1 }
                MouseArea { anchors.fill: parent; onClicked: win.gridView = false } }
            Item { x: 38; y: 2; width: 34; height: 24; Icon { anchors.centerIn: parent; name: "grid"; size: 16; color: win.gridView ? Theme.t1 : Theme.t3 }
                MouseArea { anchors.fill: parent; onClicked: win.gridView = true } } } }

    // ---- tabs + breadcrumbs ----------------------------------------------------------------
    Item { id: tabs; y: tbar.height; width: parent.width; height: 42
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
        Row { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 6
            Repeater { model: Synthetic.tabs
                delegate: Rectangle { height: 28; radius: Theme.rRow; width: tl.implicitWidth + 8 + 12 + 19; color: index === 0 ? Theme.hover : "transparent"
                    Row { x: 11; anchors.verticalCenter: parent.verticalCenter; spacing: 8
                        Text { id: tl; text: modelData; color: index === 0 ? Theme.t1 : Theme.t2; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter }
                        Icon { name: "close"; size: 12; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter } } } }
            Item { width: 28; height: 26; anchors.verticalCenter: parent.verticalCenter; Icon { anchors.centerIn: parent; name: "plus"; size: 14; color: Theme.t2 } } }
        Row { anchors.right: parent.right; anchors.rightMargin: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 4
            Repeater { model: Synthetic.crumbs
                delegate: Row { spacing: 4
                    Text { text: modelData; color: index === Synthetic.crumbs.length - 1 ? Theme.t1 : Theme.t3; font.family: Theme.font; font.pixelSize: 12
                        font.weight: index === Synthetic.crumbs.length - 1 ? Font.Medium : Font.Normal; anchors.verticalCenter: parent.verticalCenter }
                    Icon { visible: index < Synthetic.crumbs.length - 1; name: "chevR"; size: 12; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter } } } } }

    // ---- footer ----------------------------------------------------------------------------
    Item { id: foot; y: parent.height - 30; width: parent.width; height: 30
        Rectangle { width: parent.width; height: 1; color: Theme.hair }
        Row { x: 16; anchors.verticalCenter: parent.verticalCenter; spacing: 14
            Row { spacing: 4
                Text { text: String(Synthetic.files.length); color: Theme.t2; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium; font.features: { "tnum": 1 } }
                Text { text: win.gridView ? "elementos · 1 seleccionado" : "elementos"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } }
            Text { text: "Diseño · datos de ejemplo"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } }
        Row { anchors.right: parent.right; anchors.rightMargin: 16; anchors.verticalCenter: parent.verticalCenter; spacing: 6
            Icon { name: "stale"; size: 12; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter }
            Text { text: Synthetic.queue.updated; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter } } }

    // ---- sidebar ---------------------------------------------------------------------------
    Rectangle { id: side; x: 0; y: tbar.height + tabs.height; width: 228; height: parent.height - y - foot.height
        color: Theme.side
        Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.hair }
        Column { x: 10; y: 16; width: parent.width - 20; spacing: 2
            Text { text: "UBICACIONES"; leftPadding: 10; bottomPadding: 6; color: Theme.t3; font.family: Theme.disp; font.pixelSize: 10; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
            Repeater { model: Synthetic.places
                delegate: Rectangle { width: parent.width; height: 30; radius: Theme.rRow; color: win.place === index ? Theme.sel : (sma.containsMouse ? Theme.hover : "transparent")
                    Row { x: 10; anchors.verticalCenter: parent.verticalCenter; spacing: 10
                        Icon { name: modelData.icon; size: 16; color: win.place === index ? Theme.acc : Theme.t3; anchors.verticalCenter: parent.verticalCenter }
                        Text { text: modelData.name; color: win.place === index ? Theme.t1 : Theme.t2; font.family: Theme.font; font.pixelSize: 13; anchors.verticalCenter: parent.verticalCenter } }
                    DomainMark { visible: modelData.domain !== ""; domain: modelData.domain; anchors.right: parent.right; anchors.rightMargin: 12; anchors.verticalCenter: parent.verticalCenter }
                    MouseArea { id: sma; anchors.fill: parent; hoverEnabled: true; onClicked: win.place = index } } }
            Item { width: 1; height: 14 }
            Text { text: "SISTEMA"; leftPadding: 10; bottomPadding: 6; color: Theme.t3; font.family: Theme.disp; font.pixelSize: 10; font.weight: Font.DemiBold; font.letterSpacing: 1.2 }
            // Sealed is a status row, never a folder you can open.
            Item { width: parent.width; height: 30
                Row { x: 10; anchors.verticalCenter: parent.verticalCenter; spacing: 10
                    Icon { name: "sealed"; size: 16; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter }
                    Text { text: "Sealed"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13; anchors.verticalCenter: parent.verticalCenter } }
                Text { anchors.right: parent.right; anchors.rightMargin: 10; anchors.verticalCenter: parent.verticalCenter; text: "solo estado"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } } } }

    // ---- main: list | grid, then the operation queue ----------------------------------------
    Item { id: main; x: side.width; y: side.y; width: parent.width - side.width; height: side.height
        // queue first, so the list/grid can take what is left
        Rectangle { id: queue; x: 14; y: parent.height - height - 14; width: parent.width - 28; height: qc.implicitHeight + 24
            radius: Theme.rCard; color: Theme.raised; border.width: 1; border.color: Theme.hair
            Row { id: qc; x: 14; y: 12; width: parent.width - 28; spacing: 24
                Column { width: parent.width - qbtns.width - 24; spacing: 0
                    Row { spacing: 0
                        Text { text: Synthetic.queue.title; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium }
                        Text { text: " · " + Synthetic.queue.detail; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13; font.features: { "tnum": 1 } } }
                    Item { width: parent.width; height: 22
                        Rectangle { anchors.verticalCenter: parent.verticalCenter; width: parent.width; height: 4; radius: 2; color: Qt.rgba(1, 1, 1, 0.10)
                            Rectangle { width: parent.width * Synthetic.queue.progress; height: parent.height; radius: 2; color: Theme.acc } } }
                    Row { spacing: 6
                        Icon { name: "done"; size: 12; color: Theme.ok; anchors.verticalCenter: parent.verticalCenter }
                        Text { text: Synthetic.queue.permission; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter } } }
                Column { id: qbtns; spacing: 6
                    Row { spacing: 8; layoutDirection: Qt.RightToLeft; anchors.right: parent.right
                        // disabled: dashed, with its reason below
                        Item { width: 78; height: 28
                            DashedRect { anchors.fill: parent; radius: Theme.rRow; color: Theme.hair }
                            Text { anchors.centerIn: parent; text: "Deshacer"; color: Theme.tdis; font.family: Theme.font; font.pixelSize: 12; font.weight: Font.Medium } }
                        Rectangle { width: 74; height: 28; radius: Theme.rRow; color: Theme.hover; border.width: 1; border.color: Theme.hair
                            Text { anchors.centerIn: parent; text: "Cancelar"; color: Theme.t1; font.family: Theme.font; font.pixelSize: 12; font.weight: Font.Medium } } }
                    Text { anchors.right: parent.right; text: Synthetic.queue.undoWhy; color: Theme.warn; font.family: Theme.font; font.pixelSize: 11 } } } }

        // ---- list ----
        Item { visible: !win.gridView; x: 14; y: 8; width: parent.width - 28; height: queue.y - 8
            readonly property int cName: width - 24 - 92 - 140 - 250 - 36
            Item { id: head; width: parent.width; height: 32
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
                Row { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 4
                    Text { text: "Nombre"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter }
                    Icon { name: "chevD"; size: 12; color: Theme.t2; anchors.verticalCenter: parent.verticalCenter } }
                Text { x: parent.width - 12 - 250 - 12 - 140 - 12 - 92; width: 92; horizontalAlignment: Text.AlignRight; anchors.verticalCenter: parent.verticalCenter
                    text: "Tamaño"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium }
                Text { x: parent.width - 12 - 250 - 12 - 140; anchors.verticalCenter: parent.verticalCenter; text: "Modificado"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium }
                Text { x: parent.width - 12 - 250; anchors.verticalCenter: parent.verticalCenter; text: "Contexto"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium } }
            Column { y: head.height + 4; width: parent.width
                Repeater { model: Synthetic.files
                    delegate: Rectangle { width: parent.width; height: 42; radius: Theme.rRow; color: win.selected === index ? Theme.sel : (ra.containsMouse ? Theme.hover : "transparent")
                        Row { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 10; width: parent.width - 24 - 92 - 140 - 250 - 36
                            Icon { name: modelData.icon; size: 18; color: win.catColor(modelData); anchors.verticalCenter: parent.verticalCenter }
                            Text { text: modelData.name; elide: Text.ElideRight; width: parent.width - 28; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter } }
                        Text { x: parent.width - 12 - 250 - 12 - 140 - 12 - 92; width: 92; horizontalAlignment: Text.AlignRight; anchors.verticalCenter: parent.verticalCenter
                            text: modelData.size; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13; font.features: { "tnum": 1 } }
                        Text { x: parent.width - 12 - 250 - 12 - 140; anchors.verticalCenter: parent.verticalCenter; text: modelData.when; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13; font.features: { "tnum": 1 } }
                        Row { x: parent.width - 12 - 250; anchors.verticalCenter: parent.verticalCenter; spacing: 6
                            Chip { visible: modelData.archie !== ""; small: true; tone: "arch"; icon: "archie"; text: "Archie lo tocó"; sub: modelData.archie }
                            Row { visible: modelData.domain !== ""; spacing: 6
                                Chip { small: true; domain: modelData.domain; text: modelData.domain === "work" ? "Trabajo" : "Personal" } }
                            Text { visible: modelData.archie === "" && modelData.domain === ""; text: "—"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 13 } }
                        MouseArea { id: ra; anchors.fill: parent; hoverEnabled: true; onClicked: win.selected = index } } } } }

        // ---- grid ----
        Grid { visible: win.gridView; x: 14; y: 14; width: parent.width - 28; columns: 4; spacing: 14
            Repeater { model: Synthetic.files
                delegate: Rectangle { width: (main.width - 28 - 42) / 4; height: tc.implicitHeight + 16; radius: 12
                    color: win.selected === index ? Theme.sel : "transparent"; border.width: 1; border.color: win.selected === index ? Theme.tint(Theme.acc, 0.45) : "transparent"
                    Column { id: tc; x: 8; y: 8; width: parent.width - 16; spacing: 4
                        Item { width: parent.width; height: 66 + 6
                            Rectangle { width: parent.width; height: 66; radius: 9; color: Qt.rgba(1, 1, 1, 0.04); border.width: modelData.loading ? 0 : 1; border.color: Theme.hair
                                DashedRect { visible: modelData.loading === true; anchors.fill: parent; radius: 9; color: Theme.hair }
                                Icon { anchors.centerIn: parent; name: modelData.icon; size: 36; color: win.catColor(modelData); opacity: modelData.loading ? 0.4 : 1 }
                                Rectangle { visible: modelData.loading === true; x: 12; y: parent.height - 11; width: parent.width - 24; height: 3; radius: 2; color: Qt.rgba(1, 1, 1, 0.12) } } }
                        Text { text: modelData.name; elide: Text.ElideRight; width: parent.width; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium }
                        Text { text: (modelData.kind !== "" ? modelData.kind : modelData.size) + " · " + modelData.when; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.features: { "tnum": 1 } }
                        Item { width: parent.width; height: 22
                            Chip { visible: modelData.archie !== ""; small: true; tone: "arch"; icon: "archie"; text: "Archie lo tocó"; sub: modelData.archie }
                            Chip { visible: modelData.domain !== ""; small: true; domain: modelData.domain; text: modelData.domain === "work" ? "Trabajo" : "Personal" } } }
                    MouseArea { anchors.fill: parent; onClicked: win.selected = index } } } }
    }
}
