// A window with a real menu, for the one thing a window alone cannot show: a surface that
// an application opens over its window and closes as soon as it is done with.
//
// `popupType: Popup.Window` is what makes the menu a surface of its own on the compositor
// rather than an item drawn inside the window. That is what Archie has to be able to name.
//
//     QT_QUICK_CONTROLS_STYLE=Fusion QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1 qml menu.qml
import QtQuick
import QtQuick.Controls
import QtQuick.Window

Window {
    id: root
    width: 720
    height: 480
    visible: true
    color: "#f3f5f9"
    title: "Menú de prueba"

    property int chosen: 0

    function report() {
        console.log("menu: layout", root.width, root.height,
                    "archivo", Math.round(archivo.x + archivo.width / 2),
                    Math.round(archivo.y + archivo.height / 2))
    }
    Component.onCompleted: report()
    onWidthChanged: report()
    onHeightChanged: report()

    Button {
        id: archivo
        x: 32
        y: 28
        text: "Archivo"
        Accessible.name: "Archivo"
        onClicked: archivo.menu.popup(archivo, 0, archivo.height)
        property Menu menu: fileMenu
    }

    Menu {
        id: fileMenu
        popupType: Popup.Window
        onOpened: console.log("menu: open", Math.round(fileMenu.width), Math.round(fileMenu.height))
        onClosed: console.log("menu: closed")
        MenuItem {
            text: "Nuevo"
            Accessible.name: "Nuevo"
            onTriggered: console.log("menu: chose Nuevo")
        }
        MenuItem {
            text: "Guardar como"
            Accessible.name: "Guardar como"
            onTriggered: {
                root.chosen += 1
                console.log("menu: chose Guardar como", root.chosen)
            }
        }
        MenuItem {
            text: "Salir"
            Accessible.name: "Salir"
            onTriggered: console.log("menu: chose Salir")
        }
    }

    Text {
        x: 32
        y: 120
        color: "#26303d"
        font.pixelSize: 16
        text: root.chosen > 0 ? "Guardar como: " + root.chosen : "nada elegido todavía"
    }
}
