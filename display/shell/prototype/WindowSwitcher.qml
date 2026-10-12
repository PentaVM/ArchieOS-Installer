import QtQuick

// Alt-Tab / window overview, 640 px. DESIGN SAMPLE: the shell has no compositor adapter yet, so
// the windows are examples. Every card is icon + title, never a capture (the content of other
// windows is not verified). Grouped by domain (work / personal); the app is on every card.
Panel {
    id: sw
    kind: "panel"
    width: 640
    height: foot.y + foot.height
    property string st: Demo.state

    readonly property var work: [
        { title: "Catálogo de puentes", app: "Forge",  key: "forge" },
        { title: "motor-bracket",       app: "Vault",  key: "vault", on: true },
        { title: "Compositor",          app: "Stage",  key: "stage" } ]
    readonly property var personal: st === "degraded"
        ? [ { title: "Entregas", app: "Vault", key: "vault" },
            { title: "ventana sin título", app: "app sin determinar", key: "" } ]
        : [ { title: "Entregas", app: "Vault", key: "vault" },
            { title: "Red local", app: "Sentry", key: "sentry" } ]
    readonly property var dkIcon: ({ forge: "dkForge", vault: "dkVault", stage: "dkStage", sentry: "dkSentry" })

    component Tile: Rectangle {
        id: tile
        property var info
        property string domain
        property bool ring: false
        width: 168; height: tcol.implicitHeight + 26; radius: 12
        color: info.on ? Theme.sel : Theme.raised
        border.width: 1; border.color: info.on ? Theme.tint(Theme.acc, 0.45) : Theme.hair
        Rectangle { anchors.fill: parent; anchors.margins: -4; radius: 15; color: "transparent"; border.width: 2; border.color: Theme.acc; visible: tile.ring }
        Column { id: tcol; x: 12; y: 14; width: parent.width - 24; spacing: 8
            Item { width: 44; height: 44
                Rectangle { anchors.fill: parent; radius: 11; visible: info.key !== ""; color: Theme.dk[info.key]
                    Rectangle { anchors.fill: parent; radius: 11; gradient: Gradient { GradientStop { position: 0; color: Qt.rgba(1, 1, 1, 0.10) } GradientStop { position: 1; color: Qt.rgba(0, 0, 0, 0.14) } } }
                    Icon { anchors.centerIn: parent; name: sw.dkIcon[info.key] ? sw.dkIcon[info.key] : "devunk"; size: 20; color: "#ffffff" } }
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
    }
    component Group: Column {
        id: g
        property string domain
        property var items: []
        property bool firstRing: false
        width: parent.width; spacing: 0
        Row { spacing: 6; topPadding: 16; bottomPadding: 8
            DomainDot { domain: g.domain; anchors.verticalCenter: parent.verticalCenter }
            Eyebrow { text: (g.domain === "work" ? "Trabajo" : "Personal") + " · " + g.items.length } }
        Flow { width: parent.width; spacing: 12
            Repeater { model: g.items
                delegate: Tile { info: modelData; domain: g.domain; ring: Demo.focusRing && g.firstRing && index === 0 } } }
    }

    Column { id: col; x: 20; y: 20; width: parent.width - 40; spacing: 0
        Text { text: "Ventanas"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 22; font.weight: Font.DemiBold; font.letterSpacing: -0.33 }
        Flow { width: parent.width; topPadding: 12; bottomPadding: 4; spacing: 8
            Chip { design: true; icon: "sample"; text: "Diseño · datos de ejemplo" }
            StateChip { st: sw.st; noun: "ventanas" } }
        Item { width: 1; height: 6 }
        ErrBox { visible: sw.st === "error"; text: "No se pudo leer la lista de ventanas: el compositor respondió con un formato no válido." }
        VoidBox { visible: sw.st === "vacio"; icon: "windows"; title: "No hay ventanas abiertas"; text: "Cuando abras una app aparecerá aquí, agrupada por app y por dominio." }
        Column { visible: sw.st !== "error" && sw.st !== "vacio"; width: parent.width; opacity: sw.st === "stale" ? 0.62 : 1
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
