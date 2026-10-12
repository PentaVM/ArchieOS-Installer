import QtQuick

// Honest link state of the whole shell, derived only from BridgeModel.link. Seven states, each
// with its own icon, word and tone (icons and tones as in the Archie Mac components sheet).
Chip {
    id: c
    property var bridge
    readonly property var spec: ({
        waiting:            { t: "conectando…",               i: "waiting",    k: "info" },
        live:               { t: "archied en vivo",           i: "ok",         k: "ok" },
        fixture:            { t: "GRABACIÓN · no es en vivo", i: "tape",       k: "warn" },
        daemon_unreachable: { t: "archied no responde",       i: "bad",        k: "bad" },
        displayd_offline:   { t: "displayd sin respuesta",    i: "displayoff", k: "bad" },
        invalid:            { t: "respuesta no válida",       i: "warn",       k: "warn" },
        bridge_silent:      { t: "bridge sin datos",          i: "linkoff",    k: "warn" }
    })
    readonly property var s: spec[bridge.link]
    function ageText(ms) {
        var s = Math.round(ms / 1000)
        return s < 60 ? s + " s" : Math.round(s / 60) + " min"
    }
    text: s.t
    // "último estado hace N" only when a last state really exists (system != null).
    sub: bridge.link === "daemon_unreachable" && bridge.system !== null && bridge.staleMs > 0 ? "último estado hace " + ageText(bridge.staleMs) : ""
    icon: s.i
    tone: s.k
}
