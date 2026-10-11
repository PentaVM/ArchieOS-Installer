import QtQuick

Panel {
    fill: Theme.solid
    id: ac
    width: 400; height: 520
    readonly property var tone: ({ running: Theme.info, done: Theme.ok, needs: Theme.warn })
    readonly property var word: ({ running: "en curso", done: "listo", needs: "necesita aprobación" })
    readonly property var gl: ({ running: "◔", done: "✓", needs: "!" })
    Column { x: 16; y: 16; width: parent.width - 32; spacing: 10
        Text { text: "Actividad"; color: Theme.ink; font.pixelSize: Theme.fsTitle; font.bold: true }
        Repeater { model: Synthetic.jobs
            delegate: Rectangle { width: ac.width - 32; height: modelData.state === "needs" ? 96 : 66; radius: Theme.radius; color: Theme.raised; border.width: 1; border.color: Theme.line
                Column { x: 12; y: 10; width: parent.width - 24; spacing: 5
                    Row { spacing: 8
                        Text { text: modelData.name; color: Theme.ink; font.pixelSize: Theme.fsBody; font.bold: true }
                        Chip { text: ac.word[modelData.state]; glyph: ac.gl[modelData.state]; tone: ac.tone[modelData.state] }
                    }
                    Rectangle { visible: modelData.state !== "needs"; width: parent.width; height: 5; radius: 3; color: Theme.bg
                        Rectangle { width: parent.width * modelData.progress; height: parent.height; radius: 3; color: ac.tone[modelData.state] } }
                    Text { text: modelData.detail; color: Theme.soft; font.pixelSize: Theme.fsSmall }
                    // Authorization prompt: serious, cancellable, no implicit consent. Deny is the default focus.
                    Row { visible: modelData.state === "needs"; spacing: 8
                        Rectangle { width: 92; height: 26; radius: 6; color: "transparent"; border.width: 2; border.color: Theme.ink
                            Text { anchors.centerIn: parent; text: "Rechazar"; color: Theme.ink; font.pixelSize: Theme.fsSmall } }
                        Rectangle { width: 150; height: 26; radius: 6; color: "transparent"; border.width: 1; border.color: Theme.line
                            Text { anchors.centerIn: parent; text: "Revisar detalles…"; color: Theme.soft; font.pixelSize: Theme.fsSmall } }
                    }
                }
            } }
        Text { text: "AUDITORÍA RECIENTE"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; font.letterSpacing: 1.2; topPadding: 4 }
        Repeater { model: Synthetic.audit
            delegate: Row { spacing: 8; width: ac.width - 32
                Text { text: modelData.t; color: Theme.dim; font.pixelSize: Theme.fsSmall; font.features: { "tnum": 1 } }
                Text { text: modelData.kind === "ok" ? "✓" : modelData.kind === "bad" ? "✕" : "·"; color: modelData.kind === "ok" ? Theme.ok : modelData.kind === "bad" ? Theme.bad : Theme.info; font.pixelSize: Theme.fsSmall }
                Text { width: parent.width - 70; text: modelData.text; color: Theme.soft; font.pixelSize: Theme.fsSmall; wrapMode: Text.Wrap }
            } }
        DesignBanner {}
    }
}
