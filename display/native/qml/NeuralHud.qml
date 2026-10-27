import QtQuick
import QtQuick.Controls

// The neural sphere's HUD (hud.ts): a label over each folder of the first level and over
// every running program, and the selection's card. Positions come from Rust in the
// field's coordinates; the view projects them. Set in Inter, the project's typeface.
Item {
    id: hud
    required property var field
    required property var view
    property bool labelsVisible: true
    property bool lightMode: true
    property color textColor: "#142236"
    property color mutedColor: "#566c80"
    property color errorColor: "#ba263b"
    property color surfaceColor: "#eef3f8"
    property string rootLabel: ""
    property bool navigationEnabled: true
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
        function onSourceChanged() { hud.refresh() }
    }
    // Camera/input and resizes must reproject labels even while the model is frozen.
    Connections {
        target: hud.view.camera
        function onPositionChanged() { hud.frame++ }
        function onRotationChanged() { hud.frame++ }
    }
    Connections {
        target: hud.view
        function onWidthChanged() { hud.frame++ }
        function onHeightChanged() { hud.frame++ }
    }
    property var seen: -1
    onLabelsVisibleChanged: refresh()
    Component.onCompleted: refresh()

    // Provenance remains inspectable without competing with active model readings.
    Rectangle {
        id: detailsButton
        objectName: "neuralDetailsButton"
        anchors.right: parent.right; anchors.rightMargin: 18
        anchors.top: parent.top; anchors.topMargin: 18
        width: 86; height: 28; radius: 6
        visible: hud.labelsVisible
        color: hud.surfaceColor; border.color: hud.mutedColor
        Text {
            anchors.centerIn: parent
            text: hud.field.error.length > 0 ? "!  Detalles" : hud.field.loading ? "·  Leyendo" : "Detalles"
            color: hud.field.error.length > 0 ? hud.errorColor : hud.mutedColor
            font.family: hud.family; font.pixelSize: 10
        }
        MouseArea {
            anchors.fill: parent
            onClicked: details.opened ? details.close() : details.open()
        }
        Accessible.role: Accessible.Button
        Accessible.name: "Detalles de la esfera"
        Accessible.onPressAction: details.open()
    }
    Popup {
        id: details
        objectName: "neuralDetails"
        x: Math.max(12, hud.width - width - 18); y: 52
        width: Math.min(360, Math.max(0, hud.width - 24))
        padding: 14
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        background: Rectangle { color: hud.surfaceColor; radius: 8; border.color: hud.mutedColor }
        contentItem: Column {
            spacing: 8
            Text { width: parent.width; text: "Instantánea de archivos"; color: hud.textColor; font.family: hud.family; font.pixelSize: 12; font.weight: Font.Medium }
            Text { width: parent.width; text: hud.rootLabel; textFormat: Text.PlainText; wrapMode: Text.WrapAnywhere; color: hud.mutedColor; font.family: hud.family; font.pixelSize: 11 }
            Text {
                width: parent.width
                text: "Hasta " + hud.field.limit + " nodos · " + hud.field.filesPerFolder + " archivos por carpeta\n" + hud.field.scannedAt
                    + "\nOmite ocultos y carpetas de compilación. Destellos decorativos; enlaces de estructura visual."
                color: hud.mutedColor; font.family: hud.family; font.pixelSize: 11; wrapMode: Text.Wrap
            }
            Text { width: parent.width; visible: text.length > 0; text: hud.field.error; textFormat: Text.PlainText; color: hud.errorColor; font.family: hud.family; font.pixelSize: 11; wrapMode: Text.Wrap }
            Button {
                text: "Actualizar"
                enabled: hud.navigationEnabled && !hud.field.loading
                onClicked: hud.field.refresh()
                palette.button: hud.surfaceColor; palette.buttonText: hud.textColor
            }
        }
    }

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
        visible: hud.labelsVisible && hud.field.ready && hud.card.length >= 2 && hud.card[0].length > 0
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
