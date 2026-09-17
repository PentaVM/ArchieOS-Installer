// A window that rearranges itself, for teaching a local model what controls look like.
//
// Every second and a half it draws a different set of buttons, fields and checkboxes, in
// different places, sizes and words, and it names every one of them for the accessibility
// bus. So each arrangement is a labelled example: the picture, and the application's own
// account of where its controls are. Nobody's screen is involved.
//
//     QT_QUICK_CONTROLS_STYLE=Fusion QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1 \
//         qml controls.qml
import QtQuick
import QtQuick.Controls
import QtQuick.Window

Window {
    id: root
    width: 900
    height: 600
    visible: true
    title: "Controles de prueba"
    color: grounds[Math.floor(Math.random() * grounds.length)]

    property var grounds: ["#f3f5f9", "#ffffff", "#e8eef5", "#faf7f2", "#eef1ec"]
    property var words: ["Guardar", "Cancelar", "Abrir", "Enviar", "Buscar", "Aceptar",
                         "Borrar", "Filtrar", "Añadir", "Siguiente", "Atrás", "Aplicar",
                         "Nombre", "Correo", "Nota rápida", "Asunto", "Etiqueta", "Ruta"]

    function word() {
        return words[Math.floor(Math.random() * words.length)]
    }

    ListModel { id: plan }

    function rearrange() {
        plan.clear()
        var rows = 2 + Math.floor(Math.random() * 5)
        var top = 20 + Math.random() * 60
        for (var row = 0; row < rows; ++row) {
            var perRow = 1 + Math.floor(Math.random() * 3)
            var left = 20 + Math.random() * 120
            var height = 24 + Math.floor(Math.random() * 16)
            for (var column = 0; column < perRow; ++column) {
                var kind = Math.random()
                var width = 70 + Math.floor(Math.random() * 180)
                if (left + width > root.width - 20)
                    break
                plan.append({
                    "kind": kind < 0.45 ? "button" : (kind < 0.8 ? "field" : "check"),
                    "label": word(),
                    "px": left,
                    "py": top,
                    "pw": width,
                    "ph": height,
                    "fontSize": 12 + Math.floor(Math.random() * 6)
                })
                left += width + 12 + Math.random() * 40
            }
            top += height + 14 + Math.random() * 40
            if (top > root.height - 60)
                break
        }
        console.log("controls: arrangement of", plan.count)
    }

    Repeater {
        model: plan
        delegate: Loader {
            x: model.px
            y: model.py
            width: model.pw
            height: model.ph
            sourceComponent: model.kind === "button" ? asButton
                           : (model.kind === "field" ? asField : asCheck)

            Component {
                id: asButton
                Button {
                    text: model.label
                    font.pixelSize: model.fontSize
                    Accessible.name: model.label
                }
            }
            Component {
                id: asField
                TextField {
                    placeholderText: model.label
                    font.pixelSize: model.fontSize
                    Accessible.name: model.label
                }
            }
            Component {
                id: asCheck
                CheckBox {
                    text: model.label
                    font.pixelSize: model.fontSize
                    Accessible.name: model.label
                }
            }
        }
    }

    Timer {
        interval: 1500
        running: true
        repeat: true
        onTriggered: root.rearrange()
    }
    Component.onCompleted: root.rearrange()
}
