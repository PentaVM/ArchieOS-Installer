import QtQuick

// Freshness/state chip of a design-sample surface, from Demo.state. Same words and icons in
// every surface: icon + word, never colour alone. `noun` fills "0 <noun>" for the empty state.
Chip {
    property string st: Demo.state
    property string noun: "elementos"
    readonly property var spec: ({
        normal:   { t: "actualizado",         s: "hace 4 s",                 i: "done",    k: "ok" },
        vacio:    { t: "0 " + noun,           s: "",                         i: "recents", k: "neutral" },
        error:    { t: "error de lectura",    s: "",                         i: "bad",     k: "bad" },
        offline:  { t: "archied no responde", s: "último estado hace 2 min", i: "bad",     k: "bad" },
        degraded: { t: "datos parciales",     s: "",                         i: "warn",    k: "warn" },
        stale:    { t: "desactualizado",      s: "hace 18 min",              i: "recents", k: "warn" }
    })
    text: spec[st].t; sub: spec[st].s; icon: spec[st].i; tone: spec[st].k
}
