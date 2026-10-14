import QtQuick

// Alt-Tab / window overview, 640 px. Lists the shell's real windows (Wm): the front one is marked,
// minimised ones say so, and picking one restores and raises it. What is inside each window is still
// sample content. Every card is icon + title, never a capture (the content of other windows is not
// verified). Grouped by domain (work / personal, which is a design sample too); the app is on every card.
Panel {
    id: sw
    kind: "panel"
    width: 640
    height: foot.y + foot.height
    property string st: Demo.state
    signal chosen(string key)

    function card(w) { return { title: w.title, app: w.min ? "minimizada" : (w.front ? "en primer plano" : "abierta"), key: w.id, on: w.front } }
    readonly property var all: Wm.list()
    readonly property var work: all.filter(function (w) { return w.domain === "work" }).map(card)
    readonly property var personal: all.filter(function (w) { return w.domain === "personal" }).map(card)
        .concat(st === "degraded" ? [ { title: "ventana sin título", app: "app sin determinar", key: "" } ] : [])
    readonly property var flat: work.concat(personal).filter(function (c) { return c.key !== "" })
    readonly property bool none: st === "vacio" || flat.length === 0
    // Keyboard selection (Tab / Shift+Tab / Enter), starting on the previous window as Alt-Tab does.
    property int sel: 0
    readonly property string selKey: flat.length > 0 ? flat[Math.min(sel, flat.length - 1)].key : ""
    onShownChanged: if (shown) sel = flat.length > 1 ? 1 : 0
    function step(n) { if (flat.length > 0) sel = (sel + n + flat.length) % flat.length }
    function choose() { if (selKey !== "") chosen(selKey) }

    component Tile: Rectangle {
        id: tile
        property var info
        property string domain
        property bool ring: false
        // Local on purpose: an inline component's bindings run before it can see the outer `sw`.
        readonly property var iconOf: ({ forge: "dkForge", vault: "dkVault", stage: "dkStage", sentry: "dkSentry", set: "dkSet" })
        width: 168; height: tcol.implicitHeight + 26; radius: 12
        color: info.on ? Theme.sel : Theme.raised
        border.width: 1; border.color: info.on ? Theme.tint(Theme.acc, 0.45) : Theme.hair
        Rectangle { anchors.fill: parent; anchors.margins: -4; radius: 15; color: "transparent"; border.width: 2; border.color: Theme.acc; visible: tile.ring }
        Column { id: tcol; x: 12; y: 14; width: parent.width - 24; spacing: 8
            Item { width: 44; height: 44
                Rectangle { anchors.fill: parent; radius: 11; visible: info.key !== ""; color: Theme.dk[info.key]
                    Rectangle { anchors.fill: parent; radius: 11; gradient: Gradient { GradientStop { position: 0; color: Qt.rgba(1, 1, 1, 0.10) } GradientStop { position: 1; color: Qt.rgba(0, 0, 0, 0.14) } } }
                    Icon { anchors.centerIn: parent; name: tile.iconOf[info.key] ? tile.iconOf[info.key] : "devunk"; size: 20; color: "#ffffff" } }
                DashedRect { anchors.fill: parent; visible: info.key === ""; radius: 11; color: Qt.rgba(141 / 255, 140 / 255, 156 / 255, 0.6)
                    Icon { anchors.centerIn: parent; name: "devunk"; size: 20; color: Theme.t3 } } }
            Column { width: parent.width
                Text { width: parent.width; elide: Text.ElideRight; text: info.title; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium }
                Text { width: parent.width; elide: Text.ElideRight; text: info.app; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } }
            Row { spacing: 6
                DomainDot { domain: tile.domain; anchors.verticalCenter: parent.verticalCenter }
                Text { text: tile.domain === "work" ? "Trabajo" : "Personal"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter } }
            // Offline: the authority per window cannot be read, and says so.
            Chip { visible: sw.st === "offline"; small: true; tone: "unk"; icon: "unknown"; text: "Autoridad"; sub: "sin lectura" } }
        MouseArea { anchors.fill: parent; enabled: info.key !== ""; hoverEnabled: true; onClicked: sw.chosen(info.key) }
    }
    component Group: Column {
        id: g
        property string domain
        property var items: []
        property bool firstRing: false
        width: parent.width; spacing: 0
        visible: g.items.length > 0
        Row { spacing: 6; topPadding: 16; bottomPadding: 8
            DomainDot { domain: g.domain; anchors.verticalCenter: parent.verticalCenter }
            Eyebrow { text: (g.domain === "work" ? "Trabajo" : "Personal") + " · " + g.items.length } }
        Flow { width: parent.width; spacing: 12
            Repeater { model: g.items
                delegate: Tile { info: modelData; domain: g.domain; ring: (Demo.focusRing && g.firstRing && index === 0) || (modelData.key !== "" && modelData.key === sw.selKey) } } }
    }

    Column { id: col; x: 20; y: 20; width: parent.width - 40; spacing: 0
        Text { text: "Ventanas"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 22; font.weight: Font.DemiBold; font.letterSpacing: -0.33 }
        Flow { width: parent.width; topPadding: 12; bottomPadding: 4; spacing: 8
            Chip { design: true; icon: "sample"; text: "Ventanas de esta shell · contenido de ejemplo" }
            StateChip { st: sw.st; noun: "ventanas" } }
        Item { width: 1; height: 6 }
        ErrBox { visible: sw.st === "error"; text: "No se pudo leer la lista de ventanas: el compositor respondió con un formato no válido." }
        VoidBox { visible: sw.none && sw.st !== "error"; icon: "windows"; title: "No hay ventanas abiertas"; text: "Cuando abras una app aparecerá aquí, agrupada por app y por dominio." }
        Column { visible: sw.st !== "error" && !sw.none; width: parent.width; opacity: sw.st === "stale" ? 0.62 : 1
            Group { domain: "work"; items: sw.work; firstRing: true }
            Group { domain: "personal"; items: sw.personal } }
        Item { visible: sw.st === "stale"; width: 1; height: 12 }
        NoteBox { visible: sw.st === "stale"; icon: "recents"; text: "La lista puede no incluir ventanas abiertas en los últimos 18 min." }
        Item { width: 1; height: 14 }
        NoteBox { text: "Sin vistas previas: el contenido de otras ventanas no está verificado, así que cada tarjeta muestra icono y título, no una captura." }
    }
    FootHints { id: foot; y: col.y + col.implicitHeight + 16
        hints: [["Tab", "siguiente"], ["⇧ Tab", "anterior"], ["↵", "abrir"], ["Esc", "salir"]] }
}
