import QtQuick

// Self-contained tests of the window manager (Wm), the lights and the dock's dots (no QtTest
// dependency). Read `passes` / `failures` / `log`, or run: qml WmTest.qml
// It proves the state machine: what opens, hides, raises and closes, and what each click or key
// does. It does not prove the look or the animations.
Item {
    id: t
    width: 1920; height: 1080
    property int passes: 0
    property int failures: 0
    property var log: []

    function check(name, cond) {
        if (cond) { passes += 1 } else { failures += 1; log.push("FAIL " + name) }
    }
    function reset() {
        Wm.closeAll()
        Wm.st = { vault: { open: true, min: false, max: false, z: 2 }, forge: { open: false, min: false, max: false, z: 0 },
                  sentry: { open: false, min: false, max: false, z: 0 }, stage: { open: true, min: false, max: false, z: 1 },
                  set: { open: false, min: false, max: false, z: 0 } }
        Wm.off = ({}); Wm.top = 3; Demo.bare = true
    }

    Component.onCompleted: {
        // 1. start: the desktop shows, nothing is in front, Vault and Stage exist but are hidden
        reset()
        check("start: desktop", Demo.bare && Wm.front() === "" && Wm.shownCount() === 0)
        check("start: Vault and Stage exist", Wm.isOpen("vault") && Wm.isOpen("stage") && !Wm.isOpen("forge") && Wm.openCount() === 2)

        // 2. opening leaves the desktop and brings the window to the front
        Wm.open("forge")
        check("open: leaves desktop", !Demo.bare)
        check("open: Forge is in front", Wm.front() === "forge" && Wm.isShown("forge"))
        Wm.open("set")
        check("open: Ajustes is now in front", Wm.front() === "set")
        Wm.raise("vault")
        check("raise: Vault in front", Wm.front() === "vault")

        // 3. minimise hides it and passes the front to the next; restore brings it back on top
        Wm.minimize("vault")
        check("minimise: hidden but open", Wm.isMin("vault") && Wm.isOpen("vault") && !Wm.isShown("vault"))
        check("minimise: front passes on", Wm.front() !== "vault" && Wm.front() !== "")
        Wm.open("vault")
        check("restore: back in front", !Wm.isMin("vault") && Wm.front() === "vault")

        // 4. the dock: closed opens, front minimises, background raises, minimised restores
        reset(); Demo.bare = false
        Wm.dockClick("forge")
        check("dock: closed opens", Wm.isShown("forge") && Wm.front() === "forge")
        Wm.dockClick("forge")
        check("dock: front minimises", Wm.isMin("forge"))
        Wm.dockClick("forge")
        check("dock: minimised restores", Wm.isShown("forge") && Wm.front() === "forge")
        Wm.dockClick("vault")
        check("dock: a window behind the front one is raised", Wm.front() === "vault")
        Demo.bare = true
        Wm.dockClick("vault")
        check("dock: from the desktop it opens", !Demo.bare && Wm.front() === "vault")

        // 5. the key toggles: open, then close when pressed while in front
        reset(); Demo.bare = false
        Wm.toggle("sentry")
        check("toggle: opens", Wm.isShown("sentry"))
        Wm.toggle("sentry")
        check("toggle: closes when in front", !Wm.isOpen("sentry"))
        Wm.toggle("sentry"); Wm.open("set"); Wm.toggle("sentry")
        check("toggle: behind -> raises, does not close", Wm.isOpen("sentry") && Wm.front() === "sentry")

        // 6. close forgets position and zoom
        reset(); Demo.bare = false
        Wm.moveTo("vault", 50, 60)
        check("move: offset kept", Wm.ox("vault") === 50 && Wm.oy("vault") === 60)
        Wm.toggleMax("vault")
        check("zoom: on, offset reset", Wm.isMax("vault") && Wm.ox("vault") === 0)
        Wm.toggleMax("vault")
        check("zoom: off", !Wm.isMax("vault"))
        Wm.moveTo("vault", 10, 10)
        Wm.close("vault")
        check("close: gone, offset forgotten", !Wm.isOpen("vault") && Wm.ox("vault") === 0)

        // 7. all at once
        reset(); Demo.bare = false
        Wm.open("forge"); Wm.open("sentry")
        check("all: four shown", Wm.shownCount() === 4)
        Wm.minimizeAll()
        check("minimise all: none shown, all open", Wm.shownCount() === 0 && Wm.openCount() === 4)
        Wm.showAll()
        check("show all: four back", Wm.shownCount() === 4)
        Wm.closeAll()
        check("close all: none open", Wm.openCount() === 0 && Wm.front() === "")
        Wm.showAll()
        check("show all with nothing open: nothing appears", Wm.shownCount() === 0)

        // 8. front-window actions do nothing without a front window
        Wm.closeFront(); Wm.minimizeFront(); Wm.zoomFront()
        check("front actions with no window: no error", Wm.openCount() === 0)
        Wm.open("stage"); Wm.zoomFront()
        check("zoom front: Stage zoomed", Wm.isMax("stage"))
        Wm.minimizeFront()
        check("minimise front: Stage minimised", Wm.isMin("stage"))
        Demo.bare = false; Wm.open("stage"); Wm.closeFront()
        check("close front: Stage closed", !Wm.isOpen("stage"))

        // 9. the list the switcher and menu read: front first, minimised marked, cycle works
        reset(); Demo.bare = false
        Wm.open("forge"); Wm.open("vault"); Wm.minimize("stage")
        var l = Wm.list()
        check("list: all open windows", l.length === 3)
        check("list: front first and flagged", l[0].id === "vault" && l[0].front === true && l[1].front === false)
        check("list: minimised flagged", l.filter(function (w) { return w.id === "stage" })[0].min === true)
        check("next: the window behind the front one", Wm.next(1) === l[1].id)
        check("next: wraps backwards", Wm.next(-1) === l[l.length - 1].id)

        // 10. titles
        check("titles", Wm.title("set") === "Ajustes" && Wm.title("vault") === "Vault" && Wm.frontTitle() === "Vault")
        Demo.bare = true
        check("desktop title", Wm.frontTitle() === "Escritorio")

        // 11. dragging keeps the title bar reachable (host clamps)
        reset(); Demo.bare = false
        var host = Qt.createQmlObject("import QtQuick; WinHost { wid: \"forge\"; baseX: 460; baseY: 130; width: 1000; height: 560 }", t, "host")
        check("host: registered", Wm.hosts["forge"] === host)
        Wm.open("forge")
        Wm.moveTo("forge", 99999, 99999)
        check("clamp: right/bottom edge stays reachable", host.x + 120 <= t.width + 1 && host.y <= t.height - 80 + 1)
        Wm.moveTo("forge", -99999, -99999)
        check("clamp: top stays under the top bar", host.y >= 30 - 1 && host.x + host.width >= 120 - 1)
        host.destroy()

        // 12. the lights and the dock follow the state
        var lights = Qt.createQmlObject("import QtQuick; Lights { wid: \"vault\" }", t, "lights")
        Wm.close("vault"); Wm.open("vault")
        check("lights: lit while in front", lights.lit === true)
        Wm.open("forge")
        check("lights: grey when behind", lights.lit === false)
        lights.destroy()
        var dock = Qt.createQmlObject("import QtQuick; Dock { }", t, "dock")
        Wm.open("vault")
        check("dock: current is the front window", dock.current === "vault")
        Demo.bare = true
        check("dock: current is the Orb on the desktop", dock.current === "orb")
        dock.destroy()

        reset()
        console.log("WmTest: " + passes + " passed, " + failures + " failed")
        for (var j = 0; j < log.length; j++) console.log(log[j])
    }
}
