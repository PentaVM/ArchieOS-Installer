import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The design's control desk: composition profiles, the before/after comparison, every
// setting the design exposes, and what is known about the selected orb.
Rectangle {
    id: desk
    color: "#101820"

    readonly property color ink: "#dce5e8"
    readonly property color muted: "#81949e"
    readonly property color label: "#91a4ae"
    readonly property color accent: "#f0be7e"
    readonly property color line: "#25323b"
    property string mode: "basic"
    property string tab: "shape"

    // Key, label, then min, max, step for numbers. Booleans and colours need only a label.
    readonly property var controls: ({
        basic: [["visibleLimit", "Densidad de contenido", 12, 420, 4], ["nodeSize", "Tamaño de nodos reales", 0.5, 4, 0.05],
                ["links", "Enlaces reales"], ["ambient", "Partículas fantasma"], ["rotate", "Movimiento orbital"],
                ["bloomStrength", "Brillo activo", 0, 1.5, 0.01], ["background", "Fondo"]],
        shape: [["categoryPull", "Separación por tipo", 0, 1, 0.01], ["hierarchyPull", "Cohesión de módulos", 0, 1, 0.01],
                ["radialLayers", "Profundidad radial", 0, 0.85, 0.01], ["sizeInfluence", "Influencia de masa", 0, 0.4, 0.01],
                ["scale", "Escala de esfera", 0.4, 2, 0.01], ["nodeSize", "Tamaño de nodos", 0.3, 4, 0.05],
                ["visibleLimit", "Máximo visible", 12, 420, 4], ["focusSpread", "Expansión de módulo", 0, 0.6, 0.01],
                ["shell", "Envolvente esférica"], ["shellScale", "Radio de envolvente", 0.65, 1.3, 0.01],
                ["shellOpacity", "Opacidad de envolvente", 0, 0.4, 0.005], ["guides", "Guías orbitales"]],
        surface: [["metalness", "Metalidad", 0, 1, 0.01], ["roughness", "Rugosidad", 0, 1, 0.01],
                  ["emissive", "Luminancia de nodos", 0, 2, 0.01], ["textureContrast", "Relieve de textura", 0, 3, 0.05],
                  ["textureDepth", "Profundidad de textura", 0, 0.15, 0.005], ["ambient", "Partículas fantasma"],
                  ["ambientCount", "Cantidad fantasma", 0, 3500, 50], ["ambientSize", "Tamaño fantasma", 0.1, 2, 0.05],
                  ["ghostOpacity", "Opacidad fantasma", 0, 0.6, 0.01], ["links", "Enlaces reales"],
                  ["linkOpacity", "Enlaces de fondo", 0, 0.25, 0.005], ["linkBudget", "Cantidad de fondo", 0, 2000, 10],
                  ["highlightOpacity", "Enlaces activos", 0, 1, 0.01], ["highlightBudget", "Vecindad máxima", 10, 500, 10],
                  ["pulses", "Señales reales"]],
        motion: [["physics", "Física interactiva"], ["reaction", "Respuesta al puntero", 0, 500, 5],
                 ["reach", "Radio de interacción", 5, 180, 1], ["spring", "Retorno al campo", 1, 60, 1],
                 ["damping", "Amortiguación", 0.5, 0.98, 0.01], ["rotate", "Rotación automática"],
                 ["pauseOnInspect", "Pausar rotación al inspeccionar"], ["rotationSpeed", "Velocidad de rotación", 0, 2, 0.01],
                 ["pulseSpeed", "Velocidad de señales", 0, 1, 0.01]],
        render: [["bloom", "Bloom"], ["bloomStrength", "Intensidad de bloom", 0, 2, 0.01], ["bloomRadius", "Radio de bloom", 0, 1, 0.01],
                 ["bloomThreshold", "Umbral selectivo", 0, 1, 0.01], ["exposure", "Exposición", 0.4, 2.5, 0.01],
                 ["ambientLight", "Luz ambiente", 0, 2, 0.05], ["warmLight", "Luz cálida", 0, 40000, 500],
                 ["coolLight", "Contraluz frío", 0, 30000, 500], ["background", "Fondo"], ["accentWarm", "Ámbar estructural"],
                 ["accentCool", "Acento frío"], ["colorprocesses", "Procesos"], ["colorservices", "Servicios"],
                 ["colordevices", "Dispositivos"], ["colorinterface", "Interfaz"], ["colorcognition", "Cognición"],
                 ["colormodules", "Módulos"], ["colorstorage", "Almacenamiento"], ["colorsystem", "Sistema"]]
    })
    readonly property var tabTitles: ({ shape: "FORMA", surface: "SUPERFICIE", motion: "MOVIMIENTO", render: "RENDER" })
    readonly property var profiles: [["field", "Regiones vivas"], ["crown", "Corona"], ["atlas", "Atlas"], ["dense", "Denso"]]

    Rectangle { anchors.left: parent.left; width: 1; height: parent.height; color: desk.line }

    ScrollView {
        anchors.fill: parent
        anchors.leftMargin: 1
        clip: true
        contentWidth: availableWidth

        Column {
            width: desk.width - 1
            spacing: 0

            Item {
                width: parent.width; height: 70
                Column {
                    anchors.left: parent.left; anchors.leftMargin: 20; anchors.verticalCenter: parent.verticalCenter
                    spacing: 3
                    Text { text: "MESA DE CONTROL"; color: desk.label; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.8 }
                    Text { text: "Configuración"; color: desk.ink; font.pixelSize: 17; font.bold: true }
                }
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: desk.line }
            }

            Block {
                Row {
                    width: parent.width; spacing: 6
                    Repeater {
                        model: [["basic", "Esencial"], ["advanced", "Avanzado"]]
                        delegate: Chip {
                            required property var modelData
                            width: (parent.width - 6) / 2
                            text: modelData[1]
                            checked: desk.mode === modelData[0]
                            onClicked: desk.mode = modelData[0]
                        }
                    }
                }
                Heading { text: "PERFIL DE COMPOSICIÓN" }
                ComboBox {
                    id: profileBox
                    width: parent.width
                    enabled: !lumen.comparing
                    model: desk.profiles.map(function (p) { return p[1] })
                    currentIndex: Math.max(0, desk.profiles.findIndex(function (p) { return p[0] === lumen.profile }))
                    onActivated: function (index) { lumen.applyProfile(desk.profiles[index][0]) }
                    font.pixelSize: 11
                    background: Rectangle { radius: 5; color: "#18232c"; border.color: "#35444c" }
                    contentItem: Text { leftPadding: 10; text: profileBox.displayText; color: desk.ink; font.pixelSize: 11; verticalAlignment: Text.AlignVCenter }
                }
                Row {
                    width: parent.width; spacing: 6
                    Chip { width: (parent.width - 6) / 2; text: "↺  Restablecer"; enabled: !lumen.comparing; onClicked: lumen.resetProfile() }
                    Chip { width: (parent.width - 6) / 2; text: lumen.comparing ? "◫  Ahora" : "◫  Antes"; checked: lumen.comparing; onClicked: lumen.toggleCompare() }
                }
                Text {
                    visible: lumen.comparing
                    width: parent.width; wrapMode: Text.Wrap
                    text: "Mostrando el diseño anterior (V2) sobre los mismos datos."
                    color: desk.accent; font.pixelSize: 10
                }
            }

            Block {
                visible: desk.mode === "advanced"
                Flow {
                    width: parent.width; spacing: 5
                    Repeater {
                        model: ["shape", "surface", "motion", "render"]
                        delegate: Chip {
                            required property string modelData
                            width: (parent.width - 15) / 4
                            text: desk.tabTitles[modelData]
                            small: true
                            checked: desk.tab === modelData
                            onClicked: desk.tab = modelData
                        }
                    }
                }
            }

            Block {
                Heading { text: desk.mode === "basic" ? "COMPOSICIÓN" : desk.tabTitles[desk.tab] }
                Repeater {
                    model: desk.controls[desk.mode === "basic" ? "basic" : desk.tab]
                    delegate: Loader {
                        required property var modelData
                        width: parent ? parent.width : 260
                        readonly property var value: lumen.settings[modelData[0]]
                        sourceComponent: typeof value === "boolean" ? toggleControl
                                         : typeof value === "string" ? colourControl : sliderControl
                        // The range first, then the key: a value bound before its range would be clamped to
                        // the default 0..1 and stay there.
                        onLoaded: { if (modelData.length > 2) { item.low = modelData[2]; item.high = modelData[3]; item.step = modelData[4] } item.title = modelData[1]; item.key = modelData[0] }
                    }
                }
            }

            Block {
                Heading { text: "SELECCIÓN" }
                Text {
                    visible: !lumen.selection.id
                    width: parent.width; wrapMode: Text.Wrap
                    text: "Selecciona un módulo o una entidad para inspeccionarla."
                    color: desk.muted; font.pixelSize: 11
                }
                Column {
                    visible: !!lumen.selection.id
                    width: parent.width
                    spacing: 8
                    Row {
                        spacing: 8
                        Rectangle {
                            width: tagText.implicitWidth + 12; height: 18; radius: 3
                            color: "#2a2217"; border.color: desk.accent
                            Text { id: tagText; anchors.centerIn: parent; text: lumen.selection.tag || ""; color: desk.accent; font.pixelSize: 8; font.bold: true; font.letterSpacing: 1.2 }
                        }
                    }
                    Text { width: parent.width; text: lumen.selection.name || ""; color: desk.ink; font.pixelSize: 15; font.bold: true; wrapMode: Text.WrapAnywhere }
                    Row {
                        width: parent.width; spacing: 6
                        Stat { width: (parent.width - 12) / 3; value: lumen.selection.memberCount || 1; caption: "Entradas" }
                        Stat { width: (parent.width - 12) / 3; value: Math.round((lumen.selection.activity || 0) * 100) + "%"; caption: "Actividad" }
                        Stat { width: (parent.width - 12) / 3; value: (lumen.selection.relations || []).length; caption: "Relaciones" }
                    }
                    Fact { name: "ID"; value: lumen.selection.id || "" }
                    Fact { name: "Tipo"; value: (lumen.selection.category || "") + " · " + (lumen.selection.entityKind || "") }
                    Fact { name: "Función"; value: lumen.selection.role || "" }
                    Fact { name: "Profundidad"; value: String(lumen.selection.depth || 0) }
                    Fact { name: "Foco / voz"; value: Math.round((lumen.selection.focus || 0) * 100) + "% · " + Math.round((lumen.selection.voice || 0) * 100) + "%" }
                    Fact { name: "Incertidumbre"; value: Math.round((lumen.selection.uncertainty || 0) * 100) + "%" }
                    Fact { name: "Stage"; value: lumen.selection.status ? String(lumen.selection.status).toUpperCase() : "sin acción" }
                    Heading { text: "SEÑALES OBSERVADAS" }
                    Flow {
                        width: parent.width; spacing: 5
                        Repeater {
                            model: (lumen.selection.signals && lumen.selection.signals.length) ? lumen.selection.signals : ["sin relaciones observadas"]
                            delegate: Rectangle {
                                required property string modelData
                                width: signalText.implicitWidth + 14; height: 20; radius: 10
                                color: "#18232c"; border.color: "#35444c"
                                Text { id: signalText; anchors.centerIn: parent; text: modelData; color: desk.ink; font.pixelSize: 9 }
                            }
                        }
                    }
                    Row {
                        width: parent.width; spacing: 6
                        Chip { width: (parent.width - 12) / 3; text: "Centrar"; small: true; onClicked: lumen.focusRequested(lumen.positionOf(lumen.selection.id)) }
                        Chip {
                            width: (parent.width - 12) / 3; small: true
                            visible: !!lumen.selection.expandable && lumen.lens !== "entities"
                            text: lumen.selection.expanded ? "Contraer" : "Expandir"; checked: true
                            onClicked: lumen.toggleModule(lumen.selection.id)
                        }
                        Chip { width: (parent.width - 12) / 3; text: "Explorar"; small: true; visible: !!lumen.selection.explorable; onClicked: lumen.explore(lumen.selection.id) }
                    }
                    Heading { text: "RELACIONES · ORIGEN VISIBLE" }
                    Repeater {
                        model: lumen.selection.relations || []
                        delegate: Rectangle {
                            required property var modelData
                            width: parent ? parent.width : 260
                            height: relationColumn.implicitHeight + 12; radius: 5
                            color: "#18232c"; border.color: modelData.kind === "OBSERVADA" ? "#5a4631" : "#2d4a50"
                            MouseArea { anchors.fill: parent; onClicked: { lumen.select(modelData.id); lumen.focusRequested(lumen.positionOf(modelData.id)) } }
                            Column {
                                id: relationColumn
                                anchors.left: parent.left; anchors.right: parent.right; anchors.margins: 8
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 2
                                Text { text: modelData.kind + (modelData.count > 1 ? " · ×" + modelData.count : ""); color: modelData.kind === "OBSERVADA" ? desk.accent : "#78c2ca"; font.pixelSize: 8; font.bold: true; font.letterSpacing: 1.2 }
                                Text { width: parent.width; text: modelData.name; color: desk.ink; font.pixelSize: 10; elide: Text.ElideRight }
                                Text { width: parent.width; text: modelData.relations || modelData.evidence; color: desk.muted; font.pixelSize: 9; elide: Text.ElideRight }
                            }
                        }
                    }
                    Text {
                        visible: (lumen.selection.relations || []).length === 0
                        text: "Sin relación observada en esta vista."; color: desk.muted; font.pixelSize: 10
                    }
                }
            }

            Block {
                Heading { text: "FLUJOS OBSERVADOS" }
                Repeater {
                    model: lumen.scene.workflows || []
                    delegate: Column {
                        required property var modelData
                        width: parent ? parent.width : 260
                        spacing: 1
                        Text {
                            text: String(modelData.component).toUpperCase() + "  ·  " + (modelData.active ? "ACTIVO" : modelData.observed ? "RECIENTE" : "ESPERANDO")
                            color: modelData.active ? desk.accent : desk.muted; font.pixelSize: 10; font.bold: true; font.letterSpacing: 1
                        }
                        Text {
                            text: modelData.observed ? modelData.phase + " · hace " + Math.floor(modelData.age_ms / 1000) + " s" : "ningún evento todavía"
                            color: "#6c838e"; font.pixelSize: 9
                        }
                    }
                }
                Text {
                    width: parent.width; wrapMode: Text.Wrap
                    text: "El orbe observa; no decide ni ejecuta. Política y Stage conservan la autoridad."
                    color: "#6c838e"; font.pixelSize: 9
                }
            }
        }
    }

    Component {
        id: sliderControl
        Column {
            property string key: ""
            property string title: ""
            property real low: 0
            property real high: 1
            property real step: 0.01
            width: parent ? parent.width : 260
            spacing: 2
            Item {
                width: parent.width; height: 16
                Text { text: parent.parent.title; color: desk.ink; font.pixelSize: 11 }
                Text {
                    anchors.right: parent.right
                    text: Number(lumen.settings[parent.parent.key]).toFixed(parent.parent.step < 0.01 ? 3 : parent.parent.step < 1 ? 2 : 0)
                    color: desk.accent; font.pixelSize: 10; font.bold: true
                }
            }
            Slider {
                id: slider
                width: parent.width
                from: parent.low; to: parent.high; stepSize: parent.step
                value: Number(lumen.settings[parent.key])
                onMoved: lumen.setSetting(parent.key, value)
                background: Rectangle {
                    x: slider.leftPadding; y: slider.topPadding + slider.availableHeight / 2 - 2
                    width: slider.availableWidth; height: 4; radius: 2; color: "#243139"
                    Rectangle { width: slider.visualPosition * parent.width; height: parent.height; radius: 2; color: "#d6a873" }
                }
                handle: Rectangle {
                    x: slider.leftPadding + slider.visualPosition * (slider.availableWidth - width)
                    y: slider.topPadding + slider.availableHeight / 2 - height / 2
                    width: 14; height: 14; radius: 7; color: "#f0be7e"; border.color: "#101820"; border.width: 2
                }
            }
        }
    }

    Component {
        id: toggleControl
        Item {
            property string key: ""
            property string title: ""
            property real low
            property real high
            property real step
            width: parent ? parent.width : 260
            height: 30
            Text { anchors.verticalCenter: parent.verticalCenter; text: parent.title; color: desk.ink; font.pixelSize: 11 }
            Switch {
                id: toggle
                anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                checked: !!lumen.settings[parent.key]
                onToggled: lumen.setSetting(parent.key, checked)
                indicator: Rectangle {
                    implicitWidth: 34; implicitHeight: 18; radius: 9
                    color: toggle.checked ? "#b98552" : "#243139"
                    Rectangle { x: toggle.checked ? parent.width - width - 2 : 2; y: 2; width: 14; height: 14; radius: 7; color: toggle.checked ? "#f0be7e" : "#6c838e" }
                }
            }
        }
    }

    Component {
        id: colourControl
        Item {
            property string key: ""
            property string title: ""
            property real low
            property real high
            property real step
            width: parent ? parent.width : 260
            height: 32
            Text { anchors.verticalCenter: parent.verticalCenter; text: parent.title; color: desk.ink; font.pixelSize: 11 }
            Row {
                anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                spacing: 6
                Rectangle { width: 22; height: 22; radius: 4; color: String(lumen.settings[parent.parent.key]); border.color: "#35444c" }
                TextField {
                    width: 78; height: 24
                    text: String(lumen.settings[parent.parent.key])
                    color: desk.ink; font.pixelSize: 10
                    validator: RegularExpressionValidator { regularExpression: /#[0-9a-fA-F]{6}/ }
                    background: Rectangle { radius: 4; color: "#0c141b"; border.color: "#34444c" }
                    onEditingFinished: lumen.setSetting(parent.parent.key, text)
                }
            }
        }
    }

    component Block: Column {
        id: block
        default property alias content: inner.data
        width: parent ? parent.width : 300
        topPadding: 16; bottomPadding: 16; leftPadding: 20; rightPadding: 20
        Column { id: inner; width: block.width - 40; spacing: 9 }
        Rectangle { width: block.width - 40; height: 0 }
    }

    component Heading: Text {
        color: "#91a4ae"; font.pixelSize: 10; font.bold: true; font.letterSpacing: 1.8
    }

    component Chip: Rectangle {
        id: chip
        property string text: ""
        property bool checked: false
        property bool small: false
        signal clicked()
        height: small ? 28 : 34; radius: 5
        opacity: enabled ? 1.0 : 0.4
        color: checked ? "#2a2217" : (chipArea.containsMouse ? "#1c2a33" : "#18232c")
        border.color: checked ? "#d6a873" : "#35444c"
        Text { anchors.centerIn: parent; text: chip.text; color: chip.checked ? "#f0be7e" : "#dce5e8"; font.pixelSize: chip.small ? 9 : 11; font.bold: chip.small; font.letterSpacing: chip.small ? 1 : 0 }
        MouseArea { id: chipArea; anchors.fill: parent; hoverEnabled: true; enabled: chip.enabled; onClicked: chip.clicked() }
    }

    component Stat: Rectangle {
        property var value: 0
        property string caption: ""
        height: 46; radius: 5; color: "#18232c"; border.color: "#2c3b44"
        Column {
            anchors.centerIn: parent; spacing: 2
            Text { anchors.horizontalCenter: parent.horizontalCenter; text: String(parent.parent.value); color: "#f0be7e"; font.pixelSize: 14; font.bold: true }
            Text { anchors.horizontalCenter: parent.horizontalCenter; text: parent.parent.caption; color: "#81949e"; font.pixelSize: 8 }
        }
    }

    component Fact: Item {
        property string name: ""
        property string value: ""
        width: parent ? parent.width : 260
        height: Math.max(16, valueText.implicitHeight)
        Text { text: parent.name; color: "#81949e"; font.pixelSize: 10; width: 90 }
        Text { id: valueText; x: 96; width: parent.width - 96; text: parent.value; color: "#dce5e8"; font.pixelSize: 10; wrapMode: Text.WrapAnywhere }
    }
}
