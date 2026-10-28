import QtQuick
import Archie.Observatory

// The neural sphere's HUD (hud.ts): a label over each folder of the first level and over
// every running program, and the selection's card. Positions come from Rust in the
// field's coordinates; the view projects them. Set in Inter, the project's typeface.
Item {
    id: hud
    required property var field
    required property var view
    property bool labelsVisible: true
    property color textColor: "#bfeaff"
    property color mutedColor: "#6f9bb0"
    property string rootLabel: ""
    // Advances whenever the sphere or the camera moved, so projections are taken again.
    property int frame: 0

    readonly property string family: inter.status === FontLoader.Ready ? inter.font.family : "sans-serif"
    FontLoader { id: inter; source: "qrc:/archie/shell/fonts/inter/Inter-Variable.ttf" }

    property var rows: []
    property var card: []
    function refresh() {
        rows = labelsVisible ? field.labels() : []
        card = field.selection(rootLabel)
    }
    Connections {
        target: hud.field
        function onUpdated() { hud.frame++; if (hud.field.revision !== hud.seen) { hud.seen = hud.field.revision; hud.refresh() } }
        function onSelectionChanged() { hud.refresh() }
    }
    property var seen: -1
    onLabelsVisibleChanged: refresh()
    Component.onCompleted: refresh()

    Repeater {
        model: hud.rows
        delegate: Text {
            required property var modelData
            readonly property vector3d at: (hud.frame, hud.view.mapFrom3DScene(Qt.vector3d(modelData.x, modelData.y, modelData.z)))
            visible: at.z > 0
            x: at.x - width / 2
            y: at.y - height - 6
            text: modelData.kind === "p" ? modelData.text : modelData.text.toUpperCase()
            color: modelData.kind === "p" ? hud.textColor : hud.mutedColor
            font.family: hud.family
            font.pixelSize: 10
            font.weight: modelData.kind === "p" ? Font.Medium : Font.Normal
            font.letterSpacing: modelData.kind === "p" ? 0.4 : 1.2
            opacity: modelData.kind === "p" ? 0.95 : 0.75
        }
    }

    // The selection: id, title, what it holds, and whether it is folded.
    Column {
        visible: hud.card.length >= 2 && hud.card[0].length > 0
        anchors.left: parent.left; anchors.leftMargin: 18
        anchors.bottom: parent.bottom; anchors.bottomMargin: 18
        spacing: 3
        Repeater {
            model: hud.card.slice(1)
            delegate: Text {
                required property var modelData
                required property int index
                visible: modelData.length > 0
                text: modelData
                color: index === 0 ? hud.textColor : hud.mutedColor
                font.family: hud.family
                font.pixelSize: index === 0 ? 13 : 11
                font.weight: index === 0 ? Font.Medium : Font.Normal
            }
        }
    }
}
