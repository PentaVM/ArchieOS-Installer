import QtQuick

// Provenance of a particular projection, not a verdict about another service.
Chip {
    property var bridge: null
    property var projection: null
    readonly property string source: projection && projection.source ? projection.source : "unavailable"
    readonly property bool old: !!(bridge && bridge.silent) || !!(projection &&
        (projection.state === "stale" || projection.daemon_reachable === false))
    readonly property bool missing: !projection || source === "unavailable" || projection.state === "unavailable"
    text: source === "fixture" ? "Grabación" : missing ? "Sin lectura" : old ? "Último estado" : "Observado"
    tone: source === "fixture" || old ? "warn" : missing ? "unk" : "neutral"
    icon: source === "fixture" ? "sample" : old ? "stale" : missing ? "unknown" : "done"
    sub: old && bridge ? Math.floor(bridge.ageMs / 1000) + " s sin trama" : ""
}
