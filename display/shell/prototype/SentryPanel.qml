import QtQuick

// Sentry · red local, 780 px wide, two-column device grid. Passive: opening it starts no scan
// and changes nothing on any device; there are no action buttons because no control contract
// exists. States of the inventory are all distinct: not configured, scanner not run yet,
// unreadable, invalid, empty ("ran and found nothing"), stale, truncated.
Panel {
    id: sp
    kind: "window"
    readonly property string wid: "sentry"
    property var bridge
    width: 780
    height: tbar.height + content.implicitHeight + 46
    enterScale: 0.96
    durIn: Theme.durOpen; durOut: Theme.durClose
    // Merged over full defaults: the bridge's own fallback (state only, before `bridge` is even
    // assigned, or for frames older than this field) omits devices/path/etc., and every binding
    // below reads them unconditionally (QML property bindings do not share another property's
    // `visible` guard), so a bare { state: ... } would throw on first paint.
    readonly property var lan: Object.assign(
        { state: "not_configured", devices: [], total: 0, age_seconds: 0, path: "", reason: "", stale: false },
        (bridge && bridge.lan) || {})
    function ageText(s) { return s < 90 ? s + " s" : (s < 5400 ? Math.round(s / 60) + " min" : Math.round(s / 3600) + " h") }
    function deviceIcon(c) { return ({ television: "tv", printer: "printer", speaker: "speaker", camera: "camera" })[c] || "devunk" }

    // A dashed explanatory box ("what this state means").
    component Void: Item {
        property string title
        property string body
        property string iconName: "unknown"
        default property alias extra: extraCol.data
        width: parent ? parent.width : 300; height: vc.implicitHeight + 64
        DashedRect { anchors.fill: parent; radius: 12; color: Qt.rgba(141 / 255, 140 / 255, 156 / 255, 0.45) }
        Column { id: vc; anchors.horizontalCenter: parent.horizontalCenter; y: 34; spacing: 6; width: Math.min(parent.width - 80, 470)
            Item { width: 84; height: 84; anchors.horizontalCenter: parent.horizontalCenter
                Canvas { anchors.fill: parent
                    onPaint: { var c = getContext("2d"); c.clearRect(0, 0, 84, 84); c.lineCap = "round"
                        c.strokeStyle = "rgba(141,140,156,.55)"; c.lineWidth = 1.5; c.setLineDash([3, 7]); c.beginPath(); c.arc(42, 42, 36, 0, 6.2832); c.stroke()
                        c.strokeStyle = "rgba(141,140,156,.35)"; c.setLineDash([2, 6]); c.beginPath(); c.arc(42, 42, 20, 0, 6.2832); c.stroke()
                        c.setLineDash([]); c.strokeStyle = "rgba(185,184,196,.8)"; c.lineWidth = 5; c.beginPath(); c.moveTo(42, 42); c.lineTo(42.01, 42); c.stroke() } } }
            Item { width: 1; height: 10 }
            Text { text: title; width: parent.width; horizontalAlignment: Text.AlignHCenter; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.DemiBold }
            Text { text: body; width: parent.width; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13; lineHeight: 18; lineHeightMode: Text.FixedHeight }
            Column { id: extraCol; width: parent.width; spacing: 8 } }
    }

    Item { id: tbar; width: parent.width; height: 52
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
        TitleDrag { wid: sp.wid }
        Lights { x: 16; anchors.verticalCenter: parent.verticalCenter; wid: sp.wid }
        Text { anchors.centerIn: parent; text: "Sentry"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.DemiBold; font.letterSpacing: -0.17 } }

    Column { id: content; x: 22; y: tbar.height + 20; width: parent.width - 46; spacing: 0
        Text { text: "Sentry · red local"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 22; font.weight: Font.DemiBold; font.letterSpacing: -0.33 }
        Flow { width: parent.width; topPadding: 12; spacing: 8
            Chip { src: true; icon: "ok"; text: "Real · bridge" }
            // inventory state
            Chip { visible: lan.state === "ok"; icon: lan.stale ? "stale" : "done"; tone: lan.stale ? "warn" : "ok"
                text: lan.stale ? "inventario desactualizado" : "inventario actualizado"; sub: "hace " + sp.ageText(lan.age_seconds) }
            Chip { visible: lan.state === "missing"; icon: "unknown"; tone: "unk"; text: "escáner"; sub: "sin ejecutar" }
            Chip { visible: lan.state === "not_configured"; icon: "unknown"; tone: "unk"; text: "inventario"; sub: "no configurado" }
            Chip { visible: lan.state === "unreadable"; icon: "bad"; tone: "bad"; text: "sin permiso para leer el inventario" }
            Chip { visible: lan.state === "invalid"; icon: "warn"; tone: "warn"; text: "inventario no válido" }
            // device counter: "—" when unknown, never 0
            Chip { visible: lan.state === "ok"; text: lan.total + (lan.total === 1 ? " dispositivo" : " dispositivos") }
            Chip { visible: lan.state !== "ok"; icon: "unknown"; tone: "unk"; text: "dispositivos"; sub: "—" }
        }
        Item { width: 1; height: 16 }
        Text { visible: lan.state === "ok" && lan.total > lan.devices.length; width: parent.width
            text: "Se muestran " + lan.devices.length + " de " + lan.total + "."; color: Theme.warn; font.family: Theme.font; font.pixelSize: 12; bottomPadding: 8 }

        // ok + devices: two-column grid
        Grid { visible: lan.state === "ok" && lan.devices.length > 0; columns: 2; spacing: 12; width: parent.width
            Repeater { model: lan.state === "ok" ? lan.devices : []
                delegate: Rectangle { width: (content.width - 12) / 2; height: dv.implicitHeight + 28; radius: Theme.rCard
                    color: Theme.raised; border.width: 1; border.color: Theme.hair
                    Column { id: dv; x: 14; y: 14; width: parent.width - 28; spacing: 8
                        Item { width: parent.width; height: 36
                            Rectangle { width: 36; height: 36; radius: 9; color: Qt.rgba(1, 1, 1, 0.05); border.width: 1; border.color: Theme.hair
                                Icon { anchors.centerIn: parent; name: sp.deviceIcon(modelData.class); size: 20; color: Theme.t2 } }
                            Column { x: 46; width: parent.width - 46 - kindChip.width - 8; anchors.verticalCenter: parent.verticalCenter
                                Text { width: parent.width; text: modelData.name !== "" ? modelData.name : modelData.address; elide: Text.ElideRight
                                    color: Theme.t1; font.family: Theme.font; font.pixelSize: 14; font.weight: Font.Medium }
                                Text { text: modelData.name !== "" ? modelData.address : "sin nombre"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } }
                            Chip { id: kindChip; small: true; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                                text: modelData.class ? modelData.class : "tipo sin determinar"; tone: modelData.class ? "neutral" : "unk" } }
                        Text { width: parent.width; elide: Text.ElideRight; color: Theme.t2; font.family: Theme.font; font.pixelSize: 12
                            text: (modelData.manufacturer || modelData.model) ? [modelData.manufacturer, modelData.model, modelData.address].filter(function (x) { return x !== "" }).join(" · ")
                                                                               : "fabricante y modelo no declarados · " + modelData.address }
                        Text { width: parent.width; wrapMode: Text.Wrap; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11
                            text: Object.keys(modelData.state).length > 0 ? Object.keys(modelData.state).map(function (k) { return k + " " + modelData.state[k] }).join("   ") : "no declara nada de sí mismo" }
                        Row { spacing: 6
                            Repeater { model: modelData.seen_by; delegate: Chip { small: true; text: modelData } } } } } } }

        // ok but empty: a real reading, different from "not run"
        Void { visible: lan.state === "ok" && lan.total === 0; title: "El escáner corrió y no encontró nada"
            body: "Sí hay lectura: «0 dispositivos» con la hora de la pasada. Es otro estado que «aún no ha corrido», y se muestra distinto." }
        // missing: scanner not run
        Void { visible: lan.state === "missing"; title: "El escáner aún no ha corrido"
            body: "Todavía no hay ningún inventario que mostrar. Hasta que Sentry termine su primera pasada, esta lista no tiene datos, ni positivos ni negativos."
            Row { spacing: 8; anchors.horizontalCenter: parent.horizontalCenter
                Text { text: "se espera en"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; anchors.verticalCenter: parent.verticalCenter }
                Text { text: lan.path; color: Theme.t1; font.family: "monospace"; font.pixelSize: 12; anchors.verticalCenter: parent.verticalCenter }
                Chip { small: true; tone: "unk"; text: "archivo inexistente" } } }
        Void { visible: lan.state === "not_configured"; title: "El inventario no está configurado"
            body: "El bridge no sabe dónde está el inventario de la red local, así que no puede decir nada de ella." }
        Void { visible: lan.state === "unreadable" || lan.state === "invalid"; title: lan.state === "unreadable" ? "No se puede leer el inventario" : "El inventario no es válido"
            body: lan.reason ? lan.reason : "" }
        // the two states side by side, so they are never confused
        Grid { visible: lan.state === "missing"; columns: 2; spacing: 12; width: parent.width; topPadding: 12
            Rectangle { width: (content.width - 12) / 2; height: c1.implicitHeight + 24; radius: Theme.rCard; color: Theme.raised; border.width: 1; border.color: Theme.hair
                Column { id: c1; x: 14; y: 12; spacing: 8; width: parent.width - 28
                    Chip { small: true; icon: "unknown"; tone: "unk"; text: "aún no ha corrido" }
                    Text { width: parent.width; wrapMode: Text.Wrap; color: Theme.t2; font.family: Theme.font; font.pixelSize: 12; lineHeight: 17; lineHeightMode: Text.FixedHeight
                        text: "No se sabe cuántos dispositivos hay. El contador queda en «—», nunca en 0." } } }
            Rectangle { width: (content.width - 12) / 2; height: c2.implicitHeight + 24; radius: Theme.rCard; color: Theme.raised; border.width: 1; border.color: Theme.hair
                Column { id: c2; x: 14; y: 12; spacing: 8; width: parent.width - 28
                    Chip { small: true; icon: "done"; tone: "ok"; text: "corrió y no encontró nada" }
                    Text { width: parent.width; wrapMode: Text.Wrap; color: Theme.t2; font.family: Theme.font; font.pixelSize: 12; lineHeight: 17; lineHeightMode: Text.FixedHeight
                        text: "Sí hay lectura: «0 dispositivos» con la hora de la pasada. Es otro estado, y se muestra distinto." } } } }

        Item { width: 1; height: 18 }
        Rectangle { width: parent.width; height: 1; color: Theme.hair }
        Row { topPadding: 14; spacing: 8; width: parent.width
            Icon { name: "lock"; size: 14; color: Theme.t3; y: 15 }
            Text { width: parent.width - 24; wrapMode: Text.Wrap; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12; lineHeight: 17; lineHeightMode: Text.FixedHeight
                text: "Inventario pasivo: solo se listan los dispositivos que se anuncian por sí mismos (ssdp, mdns). Sentry no escanea ni modifica nada, y por eso aquí no hay botones de acción." } }
    }
}
