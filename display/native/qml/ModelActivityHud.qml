import QtQuick
import QtQuick.Controls

// Read-only observations supplied by Core's projection. Catalog membership and a
// healthy server alone do not establish current model activity or a measured rate.
Column {
    id: hud
    property var models: null
    property color textColor: "#40566b"
    property string family: "sans-serif"
    readonly property var activeModels: (Array.isArray(models) ? models : []).filter(function(model) {
        return model && typeof model.id === "string" && model.id.length > 0
            && model.active === true && model.stale === false
    })
    function rate(model) {
        return model.tps_available === true && typeof model.tps === "number"
            && Number.isFinite(model.tps) && model.tps >= 0
            ? model.tps.toFixed(1) + " tok/s" : "—"
    }
    spacing: 3
    Text {
        objectName: "modelActivity.unavailable"
        visible: !Array.isArray(hud.models)
        text: "Sin telemetría"
        color: hud.textColor; font.family: hud.family; font.pixelSize: 11
    }
    Text {
        visible: hud.activeModels.length > 0
        text: "Modelos"
        color: hud.textColor; font.family: hud.family; font.pixelSize: 10
    }
    Repeater {
        model: hud.activeModels
        delegate: Text {
            required property var modelData
            objectName: "modelActivity." + modelData.id
            width: hud.width
            text: (modelData.label || modelData.id) + (modelData.state ? " · " + modelData.state : "") + "  " + hud.rate(modelData)
            textFormat: Text.PlainText
            color: hud.textColor
            font.family: hud.family
            font.pixelSize: 11
            elide: Text.ElideRight
            Accessible.name: text + (hud.rate(modelData) === "—" ? ", velocidad sin medición" : ", velocidad efectiva de la última ejecución")
            HoverHandler { id: hover }
            ToolTip.visible: hover.hovered
            ToolTip.delay: 500
            ToolTip.text: "Velocidad efectiva de la última ejecución medida, incluido el tiempo de la solicitud."
        }
    }
}
