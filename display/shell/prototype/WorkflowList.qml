import QtQuick

Column {
    id:list
    property var records: []
    width: parent ? parent.width : 400
    spacing: 2
    Repeater {
        model: list.records
        delegate: ListRow {
            glyph: modelData.observed ? "activity" : "unknown"
            title: String(modelData.component || "componente")
            sub: modelData.observed
                ? String(modelData.phase || "sin fase") + " · " + modelData.count + " señales · edad " + Math.floor(modelData.age_ms / 1000) + " s"
                : "Sin señal observada; no demuestra que el servicio esté detenido"
            Chip { small: true; tone: modelData.observed ? "neutral" : "unk"
                text: !modelData.observed ? "sin señal" : modelData.active ? "actividad observada" : "última observación" }
        }
    }
}
