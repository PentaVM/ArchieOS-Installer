import QtQuick

// Activity. Two sections that say where their data comes from:
//  - TRABAJOS LARGOS: real, the `status.json` of archie-bridges jobs, read by archie-shell-bridge.
//    There is no cancel button: cancelling writes a file (an action with an effect) and the
//    shell has no authorised path for it yet.
//  - Authorization prompt and audit: DESIGN placeholders (no Policy escalation contract exists).
Panel {
    id: ac
    property var bridge
    fill: Theme.solid
    width: 440
    height: content.implicitHeight + 32
    readonly property var jobs: bridge.jobs
    readonly property var spec: ({
        starting:  { t: "iniciando",  g: "◌", c: Theme.unknown },
        running:   { t: "en curso",   g: "◔", c: Theme.info },
        done:      { t: "listo",      g: "✓", c: Theme.ok },
        cancelled: { t: "cancelado",  g: "■", c: Theme.unknown },
        failed:    { t: "falló",      g: "✕", c: Theme.bad }
    })
    function ago(ms) { var s = Math.round(ms / 1000); return s < 90 ? s + " s" : (s < 5400 ? Math.round(s / 60) + " min" : Math.round(s / 3600) + " h") }

    Column { id: content; x: 16; y: 16; width: parent.width - 32; spacing: 10
        Text { text: "Actividad"; color: Theme.ink; font.pixelSize: Theme.fsTitle; font.bold: true }

        Row { spacing: 8
            Text { text: "TRABAJOS LARGOS"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; font.letterSpacing: 1.2; anchors.verticalCenter: parent.verticalCenter }
            Chip { text: "REAL · bridge"; tone: Theme.info } }
        Text { visible: jobs.state === "not_configured"; width: parent.width; wrapMode: Text.Wrap
            text: "Sin espacios de trabajo configurados: el bridge no sabe dónde mirar."; color: Theme.soft; font.pixelSize: Theme.fsSmall }
        Text { visible: jobs.state === "ok" && jobs.jobs.length === 0; text: "Ningún trabajo largo en los espacios configurados."
            color: Theme.soft; font.pixelSize: Theme.fsSmall }
        Text { visible: jobs.state === "ok" && jobs.skipped > 0; text: jobs.skipped + " sin leer (estado dañado o fuera del límite)."
            color: Theme.warn; font.pixelSize: Theme.fsSmall }
        Repeater { model: jobs.state === "ok" ? jobs.unreadable : []
            delegate: Text { width: ac.width - 32; wrapMode: Text.Wrap; text: "No se pudo leer " + modelData[0] + ": " + modelData[1]
                color: Theme.warn; font.pixelSize: Theme.fsSmall } }

        Repeater { model: jobs.state === "ok" ? jobs.jobs : []
            delegate: Rectangle { width: ac.width - 32; height: card.implicitHeight + 20; radius: Theme.radius; color: Theme.raised; border.width: 1; border.color: Theme.line
                readonly property var s: ac.spec[modelData.state]
                Column { id: card; x: 12; y: 10; width: parent.width - 24; spacing: 5
                    Row { spacing: 8
                        Text { text: modelData.bridge + " · " + modelData.kind; color: Theme.ink; font.pixelSize: Theme.fsBody; font.bold: true }
                        Chip { text: s.t; glyph: s.g; tone: s.c } }
                    // A bar only when the job declared a total; otherwise the count alone.
                    Rectangle { visible: modelData.total > 0; width: parent.width; height: 5; radius: 3; color: Theme.bg
                        Rectangle { width: parent.width * Math.min(1, modelData.done / Math.max(1, modelData.total)); height: parent.height; radius: 3; color: s.c } }
                    Text { text: (modelData.total > 0 ? modelData.done + " de " + modelData.total : modelData.done + " iteraciones, sin total declarado")
                                 + "  ·  " + modelData.workspace + "  ·  " + modelData.id
                        color: Theme.soft; font.pixelSize: Theme.fsSmall }
                    // Quiet is not stuck: a long iteration writes nothing for a while. Say only what is known.
                    Text { visible: (modelData.state === "running" || modelData.state === "starting") && modelData.idle_ms > 60000
                        text: "sin novedades hace " + ac.ago(modelData.idle_ms); color: Theme.dim; font.pixelSize: Theme.fsSmall }
                    Text { visible: !!modelData.error; width: parent.width; wrapMode: Text.Wrap; text: modelData.error ? modelData.error : ""
                        color: Theme.bad; font.pixelSize: Theme.fsSmall; maximumLineCount: 3; elide: Text.ElideRight }
                }
            } }
        Text { visible: jobs.state === "ok" && jobs.jobs.length > 0; width: parent.width; wrapMode: Text.Wrap
            text: "Cancelar un trabajo no está disponible desde la shell todavía."; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1 }

        Rectangle { width: parent.width; height: 1; color: Theme.line }

        // DESIGN: what an authorization prompt will look like once Policy can be asked from here.
        Row { spacing: 8
            Text { text: "AUTORIZACIÓN"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; font.letterSpacing: 1.2; anchors.verticalCenter: parent.verticalCenter }
            Chip { text: "DISEÑO · sin contrato"; tone: Theme.warn } }
        Rectangle { width: parent.width; height: 96; radius: Theme.radius; color: Theme.raised; border.width: 1; border.color: Theme.line
            Column { x: 12; y: 10; width: parent.width - 24; spacing: 5
                Row { spacing: 8
                    Text { text: "Instalar herramienta kicad"; color: Theme.dim; font.pixelSize: Theme.fsBody; font.bold: true }
                    Chip { text: "ejemplo"; glyph: "!"; tone: Theme.unknown } }
                Text { text: "Así se pedirá una aprobación; hoy Policy no puede preguntar a la shell."; color: Theme.dim; font.pixelSize: Theme.fsSmall }
                // Deny is the default focus; nothing is approved implicitly.
                Row { spacing: 8
                    Rectangle { width: 92; height: 26; radius: 6; color: "transparent"; border.width: 2; border.color: Theme.dim
                        Text { anchors.centerIn: parent; text: "Rechazar"; color: Theme.dim; font.pixelSize: Theme.fsSmall } }
                    Rectangle { width: 150; height: 26; radius: 6; color: "transparent"; border.width: 1; border.color: Theme.line
                        Text { anchors.centerIn: parent; text: "Revisar detalles…"; color: Theme.dim; font.pixelSize: Theme.fsSmall } } }
            } }
        Row { spacing: 8
            Text { text: "AUDITORÍA"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; font.letterSpacing: 1.2; anchors.verticalCenter: parent.verticalCenter }
            Chip { text: "DISEÑO · ejemplo"; tone: Theme.warn } }
        Repeater { model: Synthetic.audit
            delegate: Row { spacing: 8; width: ac.width - 32
                Text { text: modelData.t; color: Theme.dim; font.pixelSize: Theme.fsSmall; font.features: { "tnum": 1 } }
                Text { text: modelData.kind === "ok" ? "✓" : modelData.kind === "bad" ? "✕" : "·"; color: Theme.dim; font.pixelSize: Theme.fsSmall }
                Text { width: parent.width - 70; text: modelData.text; color: Theme.dim; font.pixelSize: Theme.fsSmall; wrapMode: Text.Wrap }
            } }
    }
}
