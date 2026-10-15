import QtQuick

// Vault window (file manager) in the Archie Mac look. List or grid; sidebar with the Sealed
// store as a status row only; operation queue; footer with provenance. The window is
// Tabs and layout are local UI state. No filesystem contract is available; the operational
// view contains no file rows. Explicit designSamples enables static reference data only.
Panel {
    id: win
    kind: "window"
    property bool gridView: false
    property int selected: -1
    readonly property string wid: "vault"
    property real dx: 0                      // drag offset, applied by the host
    property real dy: 0
    signal closeRequested()
    signal zoomRequested()
    readonly property color noteColor: Theme.t3
    property bool designSamples: false
    readonly property bool fileOperationsAvailable: false
    readonly property string operationReason: "Vault aún no expone un contrato para leer o modificar archivos desde la shell."

    // ---- state: tabs, navigation, sort, copy queue ------------------------------------------
    property var tabsModel: [ { place: 0, path: [] } ]
    property int tabIndex: 0
    readonly property var cur: tabsModel[Math.min(tabIndex, tabsModel.length - 1)]
    readonly property int place: cur.place
    property int sortMode: 0                 // 0 as listed, 1 name A→Z, 2 name Z→A
    property string note: ""
    readonly property var rows: {
        if (!designSamples) return []
        var depth = cur.path.length
        var base = depth > 1 ? (Synthetic.children[cur.path[depth - 1]] || []) : Synthetic.placeFiles(cur.place)
        var out = base.slice()
        if (sortMode !== 0)
            out.sort(function (a, b) {
                var fa = a.icon === "folder", fb = b.icon === "folder"
                if (fa !== fb) return fa ? -1 : 1
                var c = a.name.localeCompare(b.name)
                return sortMode === 1 ? c : -c })
        return out
    }

    // Stable command API; no copy, permission or success is inferred from a UI event.
    readonly property int qDone: designSamples ? 3 : 0
    readonly property string qState: "unavailable"
    readonly property string qTitle: designSamples ? "Diseño · cola de ejemplo" : "Operaciones de archivos no disponibles"
    readonly property string qPermission: designSamples ? "Muestra visual · no representa una operación ni un permiso" : operationReason
    readonly property bool canUndo: false
    function cancelCopy() { flash(operationReason) }
    function undoCopy() { flash(operationReason) }
    function repeatCopy() { flash(operationReason) }

    Timer { id: noteTimer; interval: 3500; onTriggered: win.note = "" }
    function flash(text) { note = text; noteTimer.restart() }

    function setCur(p) { var t = tabsModel.slice(); t[tabIndex] = p; tabsModel = t }
    function select(i) { selected = (i >= 0 && i < rows.length) ? i : (rows.length ? 0 : -1) }
    function goPlace(i) { if (!designSamples || i < 0 || i >= Synthetic.places.length) { flash(operationReason); return }; setCur({ place: i, path: [Synthetic.places[i].name] }); sortMode = 0; select(0) }
    function setTab(i) { tabIndex = i; select(0) }
    function newTab() { var t = tabsModel.concat([ { place: 0, path: designSamples ? ["Entregas"] : [] } ]); tabsModel = t; tabIndex = t.length - 1; select(0) }
    function closeTab(i) {
        if (tabsModel.length <= 1) { closeRequested(); return }
        var t = tabsModel.slice(); t.splice(i, 1)
        var ni = tabIndex > i ? tabIndex - 1 : tabIndex
        tabsModel = t; tabIndex = Math.min(ni, t.length - 1); select(0)
    }
    function crumbTo(n) {                    // 0 Inicio, 1 Archie, then the path elements
        if (!designSamples) { flash(operationReason); return }
        if (n <= 1) { goPlace(n === 0 ? 0 : cur.place); return }
        setCur({ place: cur.place, path: cur.path.slice(0, n - 1) }); select(0)
    }
    function openItem(i) {
        if (!designSamples) { flash(operationReason); return }
        var f = rows[i]
        if (!f) return
        if (f.icon === "folder") { setCur({ place: cur.place, path: cur.path.concat([f.name]) }); sortMode = 0; select(0) }
        else flash("«" + f.name + "»: en este ejemplo Vault aún no tiene backend, no se abre ningún archivo.")
    }
    function openSelected() { openItem(selected) }
    function up() {
        if (!designSamples) { flash(operationReason); return }
        if (cur.path.length > 1) { setCur({ place: cur.place, path: cur.path.slice(0, cur.path.length - 1) }); select(0) }
        else flash("Ya estás en la raíz de " + Synthetic.places[cur.place].name + ".")
    }
    function move(d) { if (rows.length) selected = Math.max(0, Math.min(rows.length - 1, (selected < 0 ? -1 : selected) + d)) }
    function toggleSort() { sortMode = (sortMode + 1) % 3 }
    function reveal(name) {                  // from the palette: show a file in its place
        if (!designSamples) { flash(operationReason); return }
        goPlace(0)
        for (var i = 0; i < rows.length; i++) if (rows[i].name === name) { selected = i; return }
        for (var p = 2; p <= 3; p++) { goPlace(p); for (var j = 0; j < rows.length; j++) if (rows[j].name === name) { selected = j; return } }
        goPlace(0)
    }

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
        TitleDrag { wid: win.wid; reserveRight: 100 }
        Lights { x: 16; anchors.verticalCenter: parent.verticalCenter; wid: win.wid }
        Row { anchors.centerIn: parent
            Text { text: "Vault"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.DemiBold; font.letterSpacing: -0.17 }
            Text { text: win.designSamples ? " — Diseño · " + Synthetic.places[win.cur.place].name : " — sin backend de archivos"; color: Theme.t2; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.Medium; font.letterSpacing: -0.17 } }
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
            Repeater { model: win.tabsModel
                delegate: Rectangle { height: 28; radius: Theme.rRow; width: tl.implicitWidth + 8 + 12 + 19
                    color: index === win.tabIndex ? Theme.hover : (tma.containsMouse ? Qt.rgba(1, 1, 1, 0.04) : "transparent")
                    MouseArea { id: tma; anchors.fill: parent; hoverEnabled: true; onClicked: win.setTab(index) }
                    Row { x: 11; anchors.verticalCenter: parent.verticalCenter; spacing: 8
                        Text { id: tl; text: win.designSamples ? Synthetic.places[modelData.place].name : "Archivos"; color: index === win.tabIndex ? Theme.t1 : Theme.t2; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter }
                        Item { width: 12; height: 12; anchors.verticalCenter: parent.verticalCenter
                            Icon { anchors.centerIn: parent; name: "close"; size: 12; color: Theme.t3 }
                            MouseArea { anchors.fill: parent; anchors.margins: -4; onClicked: win.closeTab(index) } } } } }
            Item { width: 28; height: 26; anchors.verticalCenter: parent.verticalCenter; Icon { anchors.centerIn: parent; name: "plus"; size: 14; color: Theme.t2 }
                MouseArea { anchors.fill: parent; onClicked: win.newTab() } } }
        Row { anchors.right: parent.right; anchors.rightMargin: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 4
            Repeater { model: win.designSamples ? ["Inicio", "Archie"].concat(win.cur.path) : []
                delegate: Row { id: crumb; spacing: 4
                    readonly property bool last: index === win.cur.path.length + 1
                    Text { text: modelData; color: crumb.last ? Theme.t1 : (cma.containsMouse ? Theme.t2 : Theme.t3); font.family: Theme.font; font.pixelSize: 12
                        font.weight: crumb.last ? Font.Medium : Font.Normal; anchors.verticalCenter: parent.verticalCenter
                        MouseArea { id: cma; anchors.fill: parent; anchors.margins: -3; hoverEnabled: true; enabled: !crumb.last; onClicked: win.crumbTo(index) } }
                    Icon { visible: !crumb.last; name: "chevR"; size: 12; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter } } } } }

    // ---- footer ----------------------------------------------------------------------------
    Item { id: foot; y: parent.height - 30; width: parent.width; height: 30
        Rectangle { width: parent.width; height: 1; color: Theme.hair }
        Row { x: 16; anchors.verticalCenter: parent.verticalCenter; spacing: 14
            Row { spacing: 4
                Text { text: win.designSamples ? String(win.rows.length) : "—"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium; font.features: { "tnum": 1 } }
                Text { text: win.gridView && win.selected >= 0 ? "elementos · 1 seleccionado" : "elementos"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } }
            Text { text: win.note !== "" ? win.note : (win.designSamples ? "Diseño · datos de ejemplo" : "Sin lectura de archivos"); color: win.note !== "" ? Theme.warn : Theme.t3; font.family: Theme.font; font.pixelSize: 11 } }
        Row { anchors.right: parent.right; anchors.rightMargin: 16; anchors.verticalCenter: parent.verticalCenter; spacing: 6
            Icon { name: "stale"; size: 12; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter }
            Text { text: win.designSamples ? "Muestra estática" : "Sin backend"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter } } }

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
                    MouseArea { id: sma; anchors.fill: parent; hoverEnabled: true; enabled: win.designSamples; onClicked: win.goPlace(index) } } }
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
                        Text { text: win.qTitle; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium }
                        Text { visible: win.designSamples; text: " · " + win.qDone + " de 9 archivos de ejemplo"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13; font.features: { "tnum": 1 } } }
                    Item { visible: win.designSamples; width: parent.width; height: visible ? 22 : 8
                        Rectangle { anchors.verticalCenter: parent.verticalCenter; width: parent.width; height: 4; radius: 2; color: Theme.track
                            Rectangle { width: parent.width * win.qDone / 9; height: parent.height; radius: 2; color: win.qState === "undone" ? Theme.t3 : Theme.acc } } }
                    Row { spacing: 6
                        Icon { name: "lock"; size: 12; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter }
                        Text { width: parent.parent.width - 20; wrapMode: Text.Wrap; text: win.qPermission; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter } } }
                Column { id: qbtns; spacing: 6
                    Row { spacing: 8; layoutDirection: Qt.RightToLeft; anchors.right: parent.right
                        Btn { text: "Deshacer"; dis: !win.canUndo; onClicked: win.undoCopy() }
                        Btn { text: "Cancelar"; dis: !win.fileOperationsAvailable; onClicked: win.cancelCopy() } }
                    Text { visible: win.qState === "running" || win.qState === "undone"; anchors.right: parent.right
                        text: win.qState === "running" ? Synthetic.queue.undoWhy : "Deshacer: ya no queda nada que deshacer"
                        color: Theme.warn; font.family: Theme.font; font.pixelSize: 11 } } } }

        VoidBox { visible: win.rows.length === 0; x: 14; y: 14; width: parent.width - 28; icon: "folder"; title: win.designSamples ? "Carpeta vacía de ejemplo" : "Archivos sin lectura"
            text: win.designSamples ? "No hay elementos en esta ubicación de ejemplo." : win.operationReason }

        // ---- list ----
        Item { visible: !win.gridView && win.rows.length > 0; x: 14; y: 8; width: parent.width - 28; height: queue.y - 8
            readonly property int cName: width - 24 - 92 - 140 - 250 - 36
            Item { id: head; width: parent.width; height: 32
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
                Row { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 4
                    Text { text: "Nombre"; color: win.sortMode === 0 ? Theme.t2 : Theme.t1; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter }
                    Icon { name: "chevD"; size: 12; color: win.sortMode === 0 ? Theme.t3 : Theme.t1; rotation: win.sortMode === 2 ? 180 : 0; anchors.verticalCenter: parent.verticalCenter } }
                MouseArea { x: 0; width: 120; height: parent.height; onClicked: win.toggleSort() }
                Text { x: parent.width - 12 - 250 - 12 - 140 - 12 - 92; width: 92; horizontalAlignment: Text.AlignRight; anchors.verticalCenter: parent.verticalCenter
                    text: "Tamaño"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium }
                Text { x: parent.width - 12 - 250 - 12 - 140; anchors.verticalCenter: parent.verticalCenter; text: "Modificado"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium }
                Text { x: parent.width - 12 - 250; anchors.verticalCenter: parent.verticalCenter; text: "Contexto"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.weight: Font.Medium } }
            Column { y: head.height + 4; width: parent.width
                Repeater { model: win.rows
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
                        MouseArea { id: ra; anchors.fill: parent; hoverEnabled: true; onClicked: win.selected = index; onDoubleClicked: win.openItem(index) } } } } }

        // ---- grid ----
        Grid { visible: win.gridView && win.rows.length > 0; x: 14; y: 14; width: parent.width - 28; columns: 4; spacing: 14
            Repeater { model: win.rows
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
                    MouseArea { anchors.fill: parent; onClicked: win.selected = index; onDoubleClicked: win.openItem(index) } } } }
    }
}
