import QtQuick

// Honest link state of the whole shell, derived only from BridgeModel.link.
Chip {
    id: c
    property var bridge
    readonly property var spec: ({
        waiting:            { t: "conectando…",                 g: "◌", c: Theme.unknown },
        live:               { t: "archied en vivo",             g: "●", c: Theme.ok },
        fixture:            { t: "GRABACIÓN · no es en vivo",   g: "▲", c: Theme.warn },
        daemon_unreachable: { t: "archied no responde",         g: "✕", c: Theme.bad },
        displayd_offline:   { t: "displayd sin respuesta",      g: "✕", c: Theme.bad },
        invalid:            { t: "respuesta no válida",         g: "!", c: Theme.warn },
        bridge_silent:      { t: "bridge sin datos",            g: "!", c: Theme.warn }
    })
    readonly property var s: spec[bridge.link]
    function ageText(ms) {
        var s = Math.round(ms / 1000)
        return s < 60 ? s + " s" : Math.round(s / 60) + " min"
    }
    text: s.t + (bridge.link === "daemon_unreachable" && bridge.system !== null && bridge.staleMs > 0 ? " · último estado hace " + ageText(bridge.staleMs) : "")
    glyph: s.g
    tone: s.c
}
