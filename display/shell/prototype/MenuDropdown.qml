import QtQuick

// Drop-down of the top-bar menus (Archivo, Editar, Vista, Ir, Ventana). Each entry runs an
// action by id in Main.run(); entries that cannot apply right now are disabled and dim.
Panel {
    id: md
    kind: "panel"
    property string name: "Archivo"
    property var vault
    signal activated(string id)
    width: 270
    height: col.implicitHeight + 12

    readonly property var items: {
        var v = vault
        var m = {
            "Archivo": [
                { l: "Nueva pestaña", id: "vault.newTab", k: "Ctrl T" },
                { l: "Cerrar pestaña", id: "vault.closeTab" },
                { l: "Abrir seleccionado", id: "vault.open", k: "↵", dis: !v || v.selected < 0 },
                { l: "Subir un nivel", id: "vault.up", k: "⌫" },
                { sep: true },
                { l: "Buscar…", id: "overlay.search", k: "/" },
                { l: "Paleta de comandos", id: "overlay.palette", k: "Ctrl K" },
                { sep: true },
                { l: "Bloquear pantalla", id: "overlay.lock", k: "L" } ],
            "Editar": [
                { l: "Cancelar copia", id: "vault.cancel", dis: !v || v.qState !== "running" },
                { l: "Deshacer copia", id: "vault.undo", dis: !v || !v.canUndo },
                { l: "Repetir copia", id: "vault.redo", dis: !v || v.qState === "running" },
                { sep: true },
                { l: "Ordenar por nombre", id: "vault.sort" } ],
            "Vista": [
                { l: "Lista", id: "vault.list", k: "G", on: !!v && !v.gridView },
                { l: "Cuadrícula", id: "vault.grid", k: "G", on: !!v && v.gridView },
                { sep: true },
                { l: "Movimiento reducido", id: "theme.motion", k: "M", on: Theme.reducedMotion },
                { l: "Superficies opacas", id: "theme.opaque", k: "O", on: Theme.opaque },
                { l: "Cristal con desenfoque", id: "theme.blur", k: "B", on: Theme.blur },
                { sep: true },
                { l: "Navegar la esfera en 3D", id: "orb.nav", k: "Ctrl Espacio", on: Demo.orbNav },
                { l: "Escritorio vacío", id: "desk.toggle", k: "H", on: Demo.bare } ],
            "Ir": [
                { l: "Inicio", id: "go:0" },
                { l: "Entregas", id: "go:0" },
                { l: "Workspaces", id: "go:1" },
                { l: "Documentos", id: "go:2" },
                { l: "Trabajo", id: "go:3" },
                { l: "Recientes", id: "go:4" },
                { l: "Papelera", id: "go:5" } ],
            "Ventana": [
                { l: "Vault", id: "app.vault" },
                { l: "Forge", id: "app.forge", k: "F" },
                { l: "Sentry", id: "app.sentry", k: "S" },
                { l: "Ajustes", id: "app.set", k: "," },
                { sep: true },
                { l: "Cambiar de ventana", id: "overlay.switcher", k: "W" },
                { l: "Notificaciones", id: "overlay.notif", k: "N" },
                { l: "Actividad", id: "overlay.activity", k: "A" },
                { l: "Centro de control", id: "overlay.control", k: "C" },
                { l: "Inspector", id: "overlay.inspector", k: "I" } ]
        }
        return m[name] || []
    }

    Column { id: col; x: 6; y: 6; width: parent.width - 12
        Repeater { model: md.items
            delegate: Item { width: col.width; height: modelData.sep ? 9 : 30
                Rectangle { visible: modelData.sep === true; x: 8; y: 4; width: parent.width - 16; height: 1; color: Theme.hair }
                Rectangle { visible: modelData.sep !== true; anchors.fill: parent; radius: 7
                    color: (ima.containsMouse && !modelData.dis) ? Theme.hover : "transparent"
                    Icon { visible: modelData.on === true; x: 8; anchors.verticalCenter: parent.verticalCenter; name: "done"; size: 14; color: Theme.acc }
                    Text { x: 28; anchors.verticalCenter: parent.verticalCenter; text: modelData.l || ""
                        color: modelData.dis ? Theme.tdis : Theme.t1; font.family: Theme.font; font.pixelSize: 13 }
                    Text { anchors.right: parent.right; anchors.rightMargin: 10; anchors.verticalCenter: parent.verticalCenter; text: modelData.k || ""
                        color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 }
                    MouseArea { id: ima; anchors.fill: parent; hoverEnabled: true; enabled: !modelData.dis
                        onClicked: md.activated(modelData.id) } } } }
    }
}
