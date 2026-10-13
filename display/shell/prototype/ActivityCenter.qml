import QtQuick

// Activity, 460 px wide. TRABAJOS LARGOS are real (status.json of archie-bridges via the bridge);
// there is no cancel button on purpose. AUTORIZACIÓN and AUDITORÍA are design placeholders:
// Policy cannot ask the shell yet.
Panel {
    id: ac
    kind: "panel"
    property var bridge
    width: 460
    height: content.implicitHeight + 40
    // Merged over full defaults; see SentryPanel.qml's `lan` for why.
    readonly property var jobs: Object.assign(
        { state: "not_configured", jobs: [], unreadable: [], skipped: 0 },
        (bridge && bridge.jobs) || {})
    readonly property var spec: ({
        starting:  { t: "iniciando", i: "waiting",  k: "info" },
        running:   { t: "en curso",  i: "activity", k: "info" },
        done:      { t: "listo",     i: "done",     k: "ok" },
        cancelled: { t: "cancelado", i: "cancel",   k: "neutral" },
        failed:    { t: "falló",     i: "bad",      k: "bad" }
    })
    function ago(ms) { var s = Math.round(ms / 1000); return s < 90 ? s + " s" : (s < 5400 ? Math.round(s / 60) + " min" : Math.round(s / 3600) + " h") }
    function hhmm(ms) { var d = new Date(ms); return ("0" + d.getHours()).slice(-2) + ":" + ("0" + d.getMinutes()).slice(-2) }

    component SectionHead: Item {
        property string title
        default property alias chips: chipRow.data
        width: parent ? parent.width : 300; height: 22
        Text { text: parent.title.toUpperCase(); color: Theme.t3; font.family: Theme.disp; font.pixelSize: 10; font.weight: Font.DemiBold
            font.letterSpacing: 1.2; anchors.verticalCenter: parent.verticalCenter }
        Row { id: chipRow; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; spacing: 8 }
    }
    component JobCard: Rectangle {
        id: jc
        property var job
        readonly property var s: ac.spec[job.state]
        width: parent ? parent.width : 300; height: col.implicitHeight + 28
        color: Theme.raised; border.width: 1; border.color: Theme.hair; radius: Theme.rCard
        Column { id: col; x: 16; y: 14; width: parent.width - 32; spacing: 9
            Item { width: parent.width; height: 22
                Text { text: jc.job.bridge + " · " + jc.job.kind; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter }
                Chip { anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; text: jc.s.t; icon: jc.s.i; tone: jc.s.k } }
            // a bar only when the job declared a total
            Rectangle { visible: jc.job.total > 0; width: parent.width; height: 4; radius: 2; color: Qt.rgba(1, 1, 1, 0.10)
                Rectangle { width: parent.width * Math.min(1, jc.job.done / Math.max(1, jc.job.total)); height: parent.height; radius: 2
                    color: jc.job.state === "done" ? Theme.ok : Theme.acc } }
            Text { text: (jc.job.total > 0 ? jc.job.done + " de " + jc.job.total : jc.job.done + " iteraciones, sin total declarado")
                         + " · " + jc.job.workspace + " · " + jc.job.id
                color: Theme.t2; font.family: Theme.font; font.pixelSize: 12; font.features: { "tnum": 1 } }
            Rectangle { visible: !!jc.job.error; width: parent.width; height: errT.implicitHeight + 16; radius: Theme.rRow
                color: Theme.tint(Theme.bad, 0.10); border.width: 1; border.color: Theme.tint(Theme.bad, 0.30)
                Row { x: 10; y: 8; spacing: 8; width: parent.width - 20
                    Icon { name: "bad"; size: 14; color: Theme.bad }
                    Text { id: errT; width: parent.width - 22; text: jc.job.error ? jc.job.error : ""; wrapMode: Text.Wrap; maximumLineCount: 3; elide: Text.ElideRight
                        color: Theme.bad; font.family: Theme.font; font.pixelSize: 12 } } }
            // Quiet is not stuck: say only what is known.
            Row { spacing: 6
                Icon { name: "stale"; size: 12; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter }
                Text { anchors.verticalCenter: parent.verticalCenter; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.features: { "tnum": 1 }
                    text: (jc.job.state === "running" || jc.job.state === "starting") ? "sin novedades hace " + ac.ago(jc.job.idle_ms)
                        : (jc.job.state === "done" ? "terminó a las " : "falló a las ") + ac.hhmm(jc.job.updated_ms) } }
        }
    }

    Column { id: content; x: 20; y: 20; width: parent.width - 40; spacing: 10
        Text { text: "Actividad"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 22; font.weight: Font.DemiBold; font.letterSpacing: -0.33 }
        Item { width: 1; height: 2 }
        SectionHead { title: "Trabajos largos"; Chip { src: true; icon: "ok"; text: "Real · bridge" } }
        Text { visible: jobs.state === "not_configured"; width: parent.width; wrapMode: Text.Wrap
            text: "Sin espacios de trabajo configurados: el bridge no sabe dónde mirar."; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12 }
        Text { visible: jobs.state === "ok" && jobs.jobs.length === 0; text: "Ningún trabajo largo en los espacios configurados."; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12 }
        Text { visible: jobs.state === "ok" && jobs.skipped > 0; text: jobs.skipped + " sin leer (estado dañado o fuera del límite)."; color: Theme.warn; font.family: Theme.font; font.pixelSize: 12 }
        Repeater { model: jobs.state === "ok" ? jobs.unreadable : []
            delegate: Text { width: content.width; wrapMode: Text.Wrap; text: "No se pudo leer " + modelData[0] + ": " + modelData[1]; color: Theme.warn; font.family: Theme.font; font.pixelSize: 12 } }
        Repeater { model: jobs.state === "ok" ? jobs.jobs : []; delegate: JobCard { job: modelData } }
        // cancel is not offered: say so
        Item { visible: jobs.state === "ok" && jobs.jobs.length > 0; width: parent.width; height: 34
            DashedRect { anchors.fill: parent; radius: Theme.rRow; color: Theme.hair }
            Row { x: 12; anchors.verticalCenter: parent.verticalCenter; spacing: 8
                Icon { name: "lock"; size: 14; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter }
                Text { text: "Cancelar un trabajo no está disponible desde la shell todavía"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 12; anchors.verticalCenter: parent.verticalCenter } } }

        Rectangle { width: parent.width; height: 1; color: Theme.hair }
        SectionHead { title: "Autorización"; Chip { design: true; icon: "sample"; text: "Diseño · sin contrato" } }
        Rectangle { width: parent.width; height: auth.implicitHeight + 28; radius: Theme.rCard; color: Theme.raised; border.width: 1; border.color: Theme.hair
            Column { id: auth; x: 16; y: 14; width: parent.width - 32; spacing: 9
                Item { width: parent.width; height: 22
                    Text { text: Synthetic.authorization.title; color: Theme.t1; font.family: Theme.font; font.pixelSize: 13; font.weight: Font.Medium; anchors.verticalCenter: parent.verticalCenter }
                    Chip { anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; text: "pendiente"; icon: "waiting"; tone: "warn" } }
                Text { text: Synthetic.authorization.path; color: Theme.t2; font.family: Theme.font; font.pixelSize: 12 }
                // Deny holds the focus: nothing is approved by omission.
                Row { spacing: 8
                    Rectangle { width: rej.implicitWidth + 24; height: 28; radius: Theme.rRow; color: Theme.hover; border.width: 1; border.color: Theme.hair
                        Rectangle { anchors.fill: parent; anchors.margins: -4; radius: Theme.rRow + 3; color: "transparent"; border.width: 2; border.color: Theme.acc }
                        Text { id: rej; anchors.centerIn: parent; text: "Rechazar"; color: Theme.t1; font.family: Theme.font; font.pixelSize: 12; font.weight: Font.Medium } }
                    Item { width: 8; height: 1 }
                    Rectangle { width: det.implicitWidth + 24; height: 28; radius: Theme.rRow; color: "transparent"; border.width: 1; border.color: Theme.hair
                        Text { id: det; anchors.centerIn: parent; text: "Revisar detalles…"; color: Theme.t1; font.family: Theme.font; font.pixelSize: 12; font.weight: Font.Medium } } } } }
        SectionHead { title: "Auditoría"; Chip { design: true; icon: "sample"; text: "Diseño" } }
        Column { width: parent.width
            Repeater { model: Synthetic.audit
                delegate: Item { width: content.width; height: 34
                    Rectangle { visible: index < Synthetic.audit.length - 1; anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
                    Text { x: 0; width: 54; anchors.verticalCenter: parent.verticalCenter; text: modelData.t; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11; font.features: { "tnum": 1 } }
                    Icon { x: 64; anchors.verticalCenter: parent.verticalCenter; name: modelData.kind; size: 14; color: modelData.kind === "done" ? Theme.ok : modelData.kind === "bad" ? Theme.bad : Theme.arch }
                    Text { x: 88; width: parent.width - 88; anchors.verticalCenter: parent.verticalCenter; text: modelData.text; elide: Text.ElideRight
                        color: Theme.t2; font.family: Theme.font; font.pixelSize: 12 } } } }
    }
}
