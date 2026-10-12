import QtQuick

// Sentry: devices on the owner's LAN, from the inventory `archie-lan watch` keeps (read by
// archie-shell-bridge). Passive: opening this panel starts no scan and changes nothing on any
// device. There are deliberately no action buttons here: no control contract exists.
Panel {
    id: sp
    property var bridge
    fill: Theme.solid
    width: 640; height: 560
    readonly property var lan: bridge.lan
    readonly property var spec: ({
        not_configured: { t: "inventario no configurado",        g: "?", c: Theme.unknown },
        missing:        { t: "el escáner LAN aún no ha corrido", g: "!", c: Theme.warn },
        unreadable:     { t: "sin permiso para leer el inventario", g: "✕", c: Theme.bad },
        invalid:        { t: "inventario no válido",             g: "!", c: Theme.warn },
        ok:             { t: "inventario",                       g: "●", c: Theme.ok }
    })
    function ageText(s) { return s < 90 ? s + " s" : (s < 5400 ? Math.round(s / 60) + " min" : Math.round(s / 3600) + " h") }
    readonly property var s: spec[lan.state] || spec.invalid
    readonly property string headline: lan.state === "ok"
        ? (lan.stale ? "desactualizado · hace " + ageText(lan.age_seconds) : "actualizado hace " + ageText(lan.age_seconds))
        : s.t
    readonly property color tone: lan.state === "ok" && lan.stale ? Theme.warn : s.c
    readonly property string detail: lan.state === "missing" ? lan.path
        : (lan.state === "unreadable" || lan.state === "invalid" ? lan.reason : "")

    Column { x: 16; y: 16; width: parent.width - 32; spacing: 10
        Text { text: "Sentry · red local"; color: Theme.ink; font.pixelSize: Theme.fsTitle; font.bold: true }
        Row { spacing: 8
            Chip { text: "REAL · bridge"; tone: Theme.info }
            Chip { text: sp.headline; glyph: lan.state === "ok" && lan.stale ? "!" : sp.s.g; tone: sp.tone }
            Chip { visible: lan.state === "ok"; text: lan.total + (lan.total === 1 ? " dispositivo" : " dispositivos"); tone: Theme.unknown } }
        Text { visible: sp.detail !== ""; width: parent.width; wrapMode: Text.Wrap; text: sp.detail
            color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; maximumLineCount: 2; elide: Text.ElideRight }
        // An empty inventory and a missing scanner are different facts, and say so.
        Text { visible: lan.state === "ok" && lan.total === 0; text: "El escáner corrió y no encontró dispositivos que se anuncien."
            color: Theme.soft; font.pixelSize: Theme.fsBody }
        Text { visible: lan.state === "ok" && lan.total > lan.devices.length
            text: "Se muestran " + lan.devices.length + " de " + lan.total + "."; color: Theme.warn; font.pixelSize: Theme.fsSmall }

        Column { width: parent.width; spacing: 6
            Repeater { model: lan.state === "ok" ? lan.devices : []
                delegate: Rectangle { width: parent.width; height: card.implicitHeight + 16; radius: Theme.radius; color: Theme.raised; border.width: 1; border.color: Theme.line
                    Column { id: card; x: 12; y: 8; spacing: 3; width: parent.width - 24
                        Row { spacing: 8
                            Text { text: modelData.name !== "" ? modelData.name : modelData.address; color: Theme.ink; font.pixelSize: Theme.fsBody; font.bold: true }
                            Chip { text: modelData.class ? modelData.class : "tipo sin determinar"; tone: modelData.class ? Theme.info : Theme.unknown }
                            Repeater { model: modelData.seen_by; delegate: Chip { text: modelData; tone: Theme.unknown } } }
                        Text { text: [modelData.manufacturer, modelData.model].filter(function (x) { return x !== "" }).join(" ")
                                     + (modelData.manufacturer || modelData.model ? "  ·  " : "") + modelData.address
                            color: Theme.soft; font.pixelSize: Theme.fsSmall }
                        Text { visible: Object.keys(modelData.state).length > 0
                            text: Object.keys(modelData.state).map(function (k) { return k + ": " + modelData.state[k] }).join("   ")
                            color: Theme.dim; font.pixelSize: Theme.fsSmall }
                    }
                } } }
        Text { width: parent.width; wrapMode: Text.Wrap
            text: "Inventario pasivo (SSDP y mDNS). No escanea puertos ni cambia nada en los dispositivos; lo que un dispositivo dice de sí mismo no es una identidad verificada."
            color: Theme.dim; font.pixelSize: Theme.fsSmall - 1 }
    }
}
