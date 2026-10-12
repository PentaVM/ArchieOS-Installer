import QtQuick

// Machine pressure and temperature from archie-displayd's /api/kernel, via the bridge.
// RFC-006: unmeasured and idle never look the same. A source that cannot be read draws a
// dashed rule with the real reason; it never draws a bar at zero.
Column {
    id: g
    property var bridge
    spacing: 6
    readonly property var kernel: bridge.kernel
    readonly property var pressure: kernel && kernel.pressure_availability.state === "available" ? kernel.pressure : null
    readonly property string pressureWhy: !kernel ? "sin lectura: el bridge no recibió telemetría"
        : (kernel.pressure_availability.state === "unavailable" ? kernel.pressure_availability.reason : "")
    readonly property string thermalWhy: !kernel ? "sin lectura: el bridge no recibió telemetría"
        : (kernel.thermal_availability.state === "unavailable" ? kernel.thermal_availability.reason : "")

    component Dashed: Row {
        spacing: 4
        Repeater { model: 26; Rectangle { width: 8; height: 2; color: Theme.dim } }
    }
    component Meter: Column {
        id: meter
        property string label
        property real value: -1       // 0..1, or -1 when not measured
        property string why: ""
        property color tone: Theme.ok
        width: parent ? parent.width : 300
        spacing: 3
        Row { spacing: 6
            Text { text: meter.label; color: Theme.ink; font.pixelSize: Theme.fsSmall }
            Text { visible: meter.value < 0; text: "no medido"; color: Theme.dim; font.pixelSize: Theme.fsSmall } }
        Item { width: parent.width; height: 8
            Rectangle { visible: meter.value >= 0; anchors.fill: parent; radius: 4; color: Theme.raised
                Rectangle { width: parent.width * Math.min(1, meter.value); height: parent.height; radius: 4; color: meter.tone } }
            Dashed { visible: meter.value < 0; anchors.verticalCenter: parent.verticalCenter } }
        Text { visible: meter.value < 0 && meter.why !== ""; width: parent.width; wrapMode: Text.Wrap
            text: meter.why; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1 }
    }

    Row { spacing: 8
        Text { text: "RECURSOS"; color: Theme.dim; font.pixelSize: Theme.fsSmall - 1; font.letterSpacing: 1.2 }
        Chip { visible: kernel !== null; text: "presupuesto: " + (kernel ? kernel.display_budget : ""); tone: Theme.info } }

    Meter { label: "Presión CPU (PSI, 10 s)"; value: pressure ? pressure.cpu.some.avg10 / 100 : -1; why: pressureWhy }
    Meter { label: "Presión memoria (PSI, 10 s)"; value: pressure ? pressure.memory.some.avg10 / 100 : -1 }
    Meter { label: "Presión E/S (PSI, 10 s)"; value: pressure ? pressure.io.some.avg10 / 100 : -1 }

    Repeater { model: kernel && kernel.thermal_availability.state === "available" ? kernel.zones : []
        delegate: Meter { label: modelData.zone_type + " · " + modelData.celsius.toFixed(0) + " °C"
            // Per-zone scale: a zone with no declared critical trip has no scale and is not drawn as a bar.
            value: modelData.headroom_fraction === null || modelData.headroom_fraction === undefined ? -1 : modelData.headroom_fraction
            why: value < 0 ? "el firmware no declara un límite crítico: no hay escala" : ""
            tone: modelData.headroom_fraction > 0.85 ? Theme.bad : (modelData.headroom_fraction > 0.7 ? Theme.warn : Theme.ok) } }
    Meter { visible: !(kernel && kernel.thermal_availability.state === "available"); label: "Temperatura"; value: -1; why: thermalWhy }
}
