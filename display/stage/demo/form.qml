// A small form for demonstrating and testing the stage: a field, a button, a result.
//
// Run it inside the stage with the QML runtime:
//     WAYLAND_DISPLAY=archie-stage-0 QT_QPA_PLATFORM=wayland /usr/lib/qt6/bin/qml form.qml
//
// It logs where its controls are, in window coordinates, so a script can aim at them
// without guessing, and logs what happens to them, so a script can check the result
// without reading pixels.
import QtQuick
import QtQuick.Controls
import QtQuick.Window

Window {
    id: root
    width: 720
    height: 560
    visible: true
    title: "Formulario de prueba"
    color: "#f3f5f9"

    Column {
        anchors.centerIn: parent
        spacing: 16

        Label {
            text: "Nota rápida"
            font.pixelSize: 22
            color: "#1b2533"
        }
        TextField {
            id: field
            width: 380
            placeholderText: "escribe aquí"
            font.pixelSize: 16
            onTextChanged: console.log("form: text length", text.length)
        }
        Button {
            id: save
            text: "Guardar"
            font.pixelSize: 16
            onClicked: {
                result.text = "guardado: " + field.text
                console.log("form: saved", JSON.stringify(field.text))
            }
        }
        Label {
            id: result
            text: "sin guardar"
            font.pixelSize: 16
            color: "#44536a"
        }
    }

    function report() {
        const f = field.mapToItem(null, field.width / 2, field.height / 2)
        const b = save.mapToItem(null, save.width / 2, save.height / 2)
        console.log("form: layout", Math.round(root.width), Math.round(root.height),
                    "field", Math.round(f.x), Math.round(f.y),
                    "button", Math.round(b.x), Math.round(b.y))
    }
    onWidthChanged: Qt.callLater(report)
    onHeightChanged: Qt.callLater(report)
    Component.onCompleted: Qt.callLater(report)
}
