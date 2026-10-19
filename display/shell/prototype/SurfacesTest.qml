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
        var must = ["AuthDialog"]
        for (var m = 0; m < must.length; m++) {
            Demo.state = "normal"
            var d = make(must[m])
            check(must[m] + " is labelled Diseño", texts(d).indexOf("Diseño") >= 0)
            d.destroy()
        }
        // 3. the authorization dialog grants nothing and says why
        var ad = make("AuthDialog")
        var at = texts(ad)
        check("auth: Rechazar offered", at.indexOf("Rechazar") >= 0)
        check("auth: Permitir disabled with its reason", at.indexOf("Permitir no está disponible") >= 0)
        ad.destroy()
        // Operational surfaces cannot manufacture rows by changing the old demo state.
        for (var k = 0; k < states.length; k++) {
            Demo.state = states[k]
            var fg = make("ForgeWindow")
            check("forge " + states[k] + ": no invented jobs", fg.records.length === 0 && fg.modules.length === 0)
            fg.destroy()
            var nc = make("NotificationCenter")
            check("notifications " + states[k] + ": no invented Policy or jobs", nc.records.length === 0 && texts(nc).indexOf("Policy concedió") < 0)
            nc.destroy()
            var settings = make("SettingsWindow")
            check("settings " + states[k] + ": no invented providers/models", settings.providers.length === 0 && settings.models.length === 0)
            settings.destroy()
            var ii = make("InspectorPanel")
            check("inspector " + states[k] + ": no invented trajectory or workflow", ii.trajectories.length === 0 && ii.observations.length === 0)
            ii.destroy()
        }
        var vault = make("VaultWindow", "width: 1060; height: 720")
        check("vault: default has no invented files or copy", vault.rows.length === 0 && !vault.fileOperationsAvailable && !vault.canUndo)
        vault.designSamples = true
        check("vault: explicit reference preview has sample label", vault.rows.length > 0 && texts(vault).indexOf("Diseño") >= 0 && !vault.canUndo)
        vault.destroy()
        var workspace = make("WorkspaceMenu")
        check("workspace: domain cannot be inferred from view", !workspace.domainSwitchAvailable && workspace.sel === "" && workspace.domainSwitchReason.length > 0)
        workspace.destroy()
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
