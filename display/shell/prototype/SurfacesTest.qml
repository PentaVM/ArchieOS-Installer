import QtQuick

// Self-contained smoke and honesty tests for the design-sample surfaces and the motion tokens
// (no QtTest dependency). Read `passes` / `failures` / `log`, or run: qml SurfacesTest.qml
// It does not prove the look; it proves each surface builds in every state and keeps its promises:
// design samples say so, disabled controls say why, empty means empty, "sin lectura" is not "en vivo".
Item {
    id: t
    width: 1920; height: 1080
    property int passes: 0
    property int failures: 0
    property var log: []

    function check(name, cond) {
        if (cond) { passes += 1 } else { failures += 1; log.push("FAIL " + name) }
    }
    // Every visible text under `item`, joined.
    function texts(item) {
        var out = []
        function walk(it) {
            if (!it || it.visible === false) return
            if (typeof it.text === "string" && it.text !== "") out.push(it.text)
            if (typeof it.sub === "string" && it.sub !== "") out.push(it.sub)
            for (var i = 0; i < it.children.length; i++) walk(it.children[i])
        }
        walk(item)
        return out.join(" | ")
    }
    function make(type, props) {
        // The two windows get their size from the host (Main.qml), as here.
        var size = type === "ForgeWindow" ? "width: 1000; height: 560" : (type === "SettingsWindow" ? "width: 940; height: 520" : "")
        return Qt.createQmlObject("import QtQuick; " + type + " { " + size + " " + (props || "") + " }", t, type)
    }

    Component.onCompleted: {
        var states = Demo.states
        var surfaces = ["WindowSwitcher", "NotificationCenter", "ForgeWindow", "InspectorPanel", "SettingsWindow",
                        "WorkspaceMenu", "SearchPanel", "AuthDialog", "Toasts"]
        // 1. every surface builds in every state with a real size
        for (var s = 0; s < surfaces.length; s++) {
            for (var i = 0; i < states.length; i++) {
                Demo.state = states[i]
                var o = make(surfaces[s])
                check(surfaces[s] + " builds in " + states[i], o !== null && o.width > 0 && o.height > 0)
                o.destroy()
            }
        }
        // 2. design samples say they are samples
        var must = ["NotificationCenter", "ForgeWindow", "InspectorPanel", "SettingsWindow", "AuthDialog"]
        for (var m = 0; m < must.length; m++) {
            Demo.state = "normal"
            var d = make(must[m])
            check(must[m] + " is labelled Diseño", texts(d).indexOf("Diseño") >= 0)
            d.destroy()
        }
        // The switcher lists the real windows but says what is inside them is a sample.
        Demo.state = "normal"
        var sw0 = make("WindowSwitcher")
        check("WindowSwitcher says its content is a sample", texts(sw0).indexOf("contenido de ejemplo") >= 0)
        sw0.destroy()
        // 3. the authorization dialog grants nothing and says why
        var ad = make("AuthDialog")
        var at = texts(ad)
        check("auth: Rechazar offered", at.indexOf("Rechazar") >= 0)
        check("auth: Permitir disabled with its reason", at.indexOf("Permitir no está disponible") >= 0)
        ad.destroy()
        // 4. empty means empty: no phantom rows
        Demo.state = "vacio"
        var fg = make("ForgeWindow")
        check("forge empty: says so", texts(fg).indexOf("Ningún puente registrado") >= 0)
        check("forge empty: no bridge rows", texts(fg).indexOf("freecad-bridge") < 0)
        fg.destroy()
        var nc = make("NotificationCenter")
        check("notifications empty: no phantom job", texts(nc).indexOf("freecad") < 0)
        nc.destroy()
        // 5. unreadable is not alive: offline never says "en vivo"
        Demo.state = "offline"
        var fo = make("ForgeWindow")
        check("forge offline: no 'en vivo'", texts(fo).indexOf("en vivo") < 0)
        check("forge offline: 'sin lectura'", texts(fo).indexOf("sin lectura") >= 0)
        fo.destroy()
        // 6. errors show the reason
        Demo.state = "error"
        var si = make("InspectorPanel")
        check("inspector error: reason shown", texts(si).indexOf("No se pudieron leer las trayectorias") >= 0)
        check("inspector error: IRIS state still not drawn", texts(si).indexOf("No hay forma de leerlo") >= 0)
        si.destroy()
        // 7. IRIS internal state is never drawn, in any state
        for (var k = 0; k < states.length; k++) {
            Demo.state = states[k]
            var ii = make("InspectorPanel")
            check("inspector " + states[k] + ": IRIS unreadable", texts(ii).indexOf("sin lectura") >= 0)
            ii.destroy()
        }
        Demo.state = "normal"

        // 8. motion profiles
        Theme.reducedMotion = false
        Theme.motion = "full"
        check("full: open 220, pulse 600, spatial", Theme.durOpen === 220 && Theme.durPulse === 600 && Theme.spatial)
        Theme.motion = "balanced"
        check("balanced: open 160, close 120, pulse 400", Theme.durOpen === 160 && Theme.durClose === 120 && Theme.durPulse === 400 && Theme.spatial)
        Theme.motion = "quiet"
        check("quiet: opacity only, 100 ms, no space", Theme.durOpen === 100 && Theme.durPulse === 120 && !Theme.spatial)
        Theme.reducedMotion = true
        check("reduced: everything instant", Theme.durOpen === 0 && Theme.durClose === 0 && Theme.durPulse === 0 && Theme.durDock === 0 && !Theme.spatial)
        Theme.reducedMotion = false
        Theme.motion = "balanced"

        console.log("SurfacesTest: " + passes + " passed, " + failures + " failed")
        for (var j = 0; j < log.length; j++) console.log(log[j])
    }
}
