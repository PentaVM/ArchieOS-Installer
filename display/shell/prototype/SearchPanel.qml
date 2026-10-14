import QtQuick

// Search, 720 px. DESIGN SAMPLE. Searches the catalogue of actions and capabilities, filtered by
// permissions. It does not search the web while typing and draws no relations between results
// (there is no graph to read). A capability that needs escalation is disabled and says why.
Panel {
    id: sp
    kind: "panel"
    width: 720
    height: foot.y + foot.height
    enterDy: -8
    durIn: Theme.durPalette; durOut: Theme.durPaletteClose
    property string st: Demo.state
    property string query: inp.text
    signal activated(string act)
    function m(a, b) { var q = query.trim().toLowerCase(); return q === "" || (a + " " + b).toLowerCase().indexOf(q) >= 0 }
    readonly property var acts: [
        { t: "Mostrar actividad de Archie", s: "Abre el panel de trabajos y autorizaciones", a: "overlay.activity" },
        { t: "Abrir la vista general de ventanas", s: "Muestra las ventanas por app y por dominio", a: "overlay.switcher" },
        { t: "Leer espacios de trabajo", s: "Capacidad de Archie sobre Vault", a: "app.vault" },
        { t: "Escribir en motor-bracket", s: "Capacidad de Archie sobre Vault", a: "" } ]
    readonly property bool noMatch: !(m(acts[0].t, acts[0].s) || m(acts[1].t, acts[1].s) || m(acts[2].t, acts[2].s) || m(acts[3].t, acts[3].s))
    onShownChanged: if (shown) { inp.text = ""; inp.forceActiveFocus() }

    component Result: Rectangle {
        id: r
        property string glyph
        property string title
        property string sub
        property string kind
        property string statusText
        property string statusIcon
        property string statusTone: "neutral"
        property bool on: false
        property bool dis: false
        property bool ring: false
        property string act: ""
        width: parent ? parent.width : 600; height: 50; radius: 9
        color: (on || ring) ? Theme.sel : "transparent"; border.width: 1; border.color: on ? Theme.tint(Theme.acc, 0.35) : "transparent"
        Rectangle { anchors.fill: parent; radius: 9; color: "transparent"; border.width: 2; border.color: Theme.acc; visible: r.ring }
        Rectangle { x: 12; anchors.verticalCenter: parent.verticalCenter; width: 32; height: 32; radius: 8; color: Qt.rgba(1, 1, 1, 0.05); border.width: 1; border.color: Theme.hair
            Icon { anchors.centerIn: parent; name: r.glyph; size: 16; color: r.dis ? Theme.tdis : Theme.t2 } }
        Column { x: 56; anchors.verticalCenter: parent.verticalCenter; width: parent.width - 56 - 230
            Text { width: parent.width; elide: Text.ElideRight; text: r.title; color: r.dis ? Theme.tdis : Theme.t1; font.family: Theme.font; font.pixelSize: 14; font.weight: Font.Medium }
            Text { width: parent.width; elide: Text.ElideRight; text: r.sub; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12 } }
        Row { anchors.right: parent.right; anchors.rightMargin: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 8
            Chip { kind: true; small: true; text: r.kind; anchors.verticalCenter: parent.verticalCenter }
            Chip { visible: r.statusText !== ""; small: true; text: r.statusText; icon: r.statusIcon; tone: r.statusTone; anchors.verticalCenter: parent.verticalCenter } }
        MouseArea { anchors.fill: parent; enabled: !r.dis && r.act !== ""; hoverEnabled: true; onClicked: sp.activated(r.act) }
    }

    Column { id: col; x: 16; y: 16; width: parent.width - 32; spacing: 0
        Item { width: parent.width; height: 40
            Rectangle { anchors.fill: parent; radius: 12; color: Theme.raised; border.width: 1; border.color: Theme.hair }
            Row { x: 14; anchors.verticalCenter: parent.verticalCenter; spacing: 10
                Icon { name: "search"; size: 18; color: Theme.t2; anchors.verticalCenter: parent.verticalCenter }
                Item { width: 440; height: 26; anchors.verticalCenter: parent.verticalCenter
                    TextInput { id: inp; anchors.fill: parent; verticalAlignment: TextInput.AlignVCenter; clip: true
                        color: Theme.t1; font.family: Theme.font; font.pixelSize: 17; selectionColor: Theme.acc; cursorDelegate: Rectangle { width: 1.5; color: Theme.acc }
                        Keys.onPressed: function (e) {
                            if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter) {
                                for (var i = 0; i < sp.acts.length; i++) if (sp.acts[i].a !== "" && sp.m(sp.acts[i].t, sp.acts[i].s)) { sp.activated(sp.acts[i].a); break }
                                e.accepted = true } } }
                    Text { visible: inp.text === ""; text: "Buscar acciones y capacidades"; color: Theme.tdis; font.family: Theme.font; font.pixelSize: 14; anchors.verticalCenter: parent.verticalCenter } } }
            Row { anchors.right: parent.right; anchors.rightMargin: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 10
                Chip { visible: sp.st !== "normal" || Demo.focusRing; anchors.verticalCenter: parent.verticalCenter
                    text: sp.st === "vacio" ? "0 resultados" : (sp.st === "error" ? "error de lectura" : (sp.st === "offline" ? "archied no responde" : (sp.st === "degraded" ? "datos parciales" : (sp.st === "stale" ? "desactualizado" : "actualizado"))))
                    sub: sp.st === "offline" ? "último estado hace 2 min" : (sp.st === "stale" ? "hace 18 min" : (sp.st === "normal" ? "hace 4 s" : ""))
                    icon: sp.st === "vacio" ? "recents" : (sp.st === "error" || sp.st === "offline" ? "bad" : (sp.st === "degraded" ? "warn" : (sp.st === "stale" ? "recents" : "done")))
                    tone: sp.st === "vacio" ? "neutral" : (sp.st === "error" || sp.st === "offline" ? "bad" : (sp.st === "degraded" || sp.st === "stale" ? "warn" : "ok")) }
                Kbd { label: "Esc"; anchors.verticalCenter: parent.verticalCenter } } }
        Item { width: 1; height: 4 }

        ErrBox { visible: sp.st === "error"; text: "No se pudo consultar el catálogo: respuesta no válida del bridge." }
        VoidBox { visible: sp.st === "vacio" || sp.noMatch; icon: "search"; title: sp.query === "" ? "Sin resultados" : "Sin resultados para «" + sp.query + "»"; text: "No hay acciones ni capacidades que coincidan y estén permitidas para ti." }

        Column { visible: sp.st !== "error" && sp.st !== "vacio" && !sp.noMatch; width: parent.width; opacity: sp.st === "stale" ? 0.62 : 1
            Eyebrow { text: "Acciones"; leftPadding: 12; topPadding: 12; bottomPadding: 4 }
            Result { visible: sp.m(title, sub); act: "overlay.activity"; glyph: "activity"; title: "Mostrar actividad de Archie"; sub: "Abre el panel de trabajos y autorizaciones"; kind: "Acción"; on: true; ring: Demo.focusRing
                statusText: sp.st === "offline" ? "" : "permitida"; statusIcon: "done"; statusTone: "ok" }
            Result { visible: sp.st !== "offline" && sp.st !== "degraded" && sp.m(title, sub); act: "overlay.switcher"; glyph: "windows"; title: "Abrir la vista general de ventanas"; sub: "Muestra las ventanas por app y por dominio"; kind: "Acción"
                statusText: "permitida"; statusIcon: "done"; statusTone: "ok" }
            Eyebrow { text: "Capacidades"; leftPadding: 12; topPadding: 12; bottomPadding: 4 }
            Column { visible: sp.st === "normal" || sp.st === "stale"; width: parent.width
                Result { visible: sp.m(title, sub); act: "app.vault"; glyph: "dkVault"; title: "Leer espacios de trabajo"; sub: "Capacidad de Archie sobre Vault"; kind: "Capacidad"; statusText: "permitida"; statusIcon: "done"; statusTone: "ok" }
                Result { visible: sp.m(title, sub); glyph: "lock"; dis: true; title: "Escribir en motor-bracket"; sub: "Capacidad de Archie sobre Vault"; kind: "Capacidad"
                    statusText: "requiere autorización"; statusIcon: "pause"; statusTone: "warn" }
                Item { width: parent.width; height: 36; Why { x: 12; width: parent.width - 24; anchors.verticalCenter: parent.verticalCenter; text: "Requiere autorización: la escalada de Policy todavía no tiene vía." } } }
            Result { visible: sp.st === "offline"; glyph: "unknown"; title: "Capacidades de Archie"; sub: "sin lectura mientras archied no responda"; kind: "Capacidad"
                statusText: "no medido"; statusIcon: "unknown"; statusTone: "unk" }
            Result { visible: sp.st === "degraded"; glyph: "unknown"; title: "Algunas capacidades"; sub: "bridge sin datos: lista parcial"; kind: "Capacidad"
                statusText: "datos parciales"; statusIcon: "warn"; statusTone: "warn" } }
        Item { visible: sp.st === "offline" || sp.st === "stale"; width: 1; height: 10 }
        NoteBox { visible: sp.st === "offline"; text: "El catálogo de capacidades no se puede filtrar por permisos sin archied." }
        NoteBox { visible: sp.st === "stale"; icon: "recents"; text: "Catálogo leído hace 18 min." }
        Item { width: 1; height: 8 }
    }
    FootHints { id: foot; y: col.y + col.implicitHeight + 4; x: 0; width: sp.width
        hints: [["↑↓", "elegir"], ["↵", "abrir"], ["Esc", "cerrar"]]; note: "filtrado por permisos" }
}
