import QtQuick

// Self-contained tests for BridgeModel (no QtTest dependency). Run it with any Qt host and
// read `passes` / `failures` / `log`; with the stock tool:  qml BridgeModelTest.qml
// (results are printed with console.log). Frames are the real JSON lines the bridge prints.
Item {
    id: t
    property int passes: 0
    property int failures: 0
    property var log: []

    function check(name, cond) {
        if (cond) { passes += 1 } else { failures += 1; log.push("FAIL " + name) }
    }
    function frame(link, extra) {
        var f = { schema: 1, at_ms: 1, link: link, system: null, kernel: null }
        for (var k in extra) f[k] = extra[k]
        return JSON.stringify(f)
    }
    function model() { return Qt.createQmlObject('import QtQuick; BridgeModel {}', t, "m") }

    Component.onCompleted: {
        var m = model()
        check("starts waiting", m.link === "waiting" && m.received === false && m.system === null)

        check("accepts a live frame", m.ingest(frame({ state: "live" }, { system: { hostname: "n" } })))
        check("live link", m.link === "live" && m.system.hostname === "n")

        check("fixture is never live", m.ingest(frame({ state: "fixture" })) && m.link === "fixture")

        check("daemon down keeps its reason",
              m.ingest(frame({ state: "daemon_unreachable", stale_ms: 4000, reason: "no socket" })) &&
              m.link === "daemon_unreachable" && m.reason === "no socket" && m.staleMs === 4000)

        check("displayd offline", m.ingest(frame({ state: "displayd_offline", reason: "refused" })) && m.link === "displayd_offline")

        // Rejections must not clobber the last good frame.
        var before = m.link
        check("rejects non-JSON", m.ingest("not json") === false)
        check("rejects a JSON scalar", m.ingest("42") === false)
        check("rejects another schema", m.ingest(JSON.stringify({ schema: 2, link: { state: "live" } })) === false)
        check("rejects a frame without a link", m.ingest(JSON.stringify({ schema: 1 })) === false)
        check("rejections are counted", m.rejected === 4 && m.lastRejection === "no link state")
        check("rejections keep the last frame", m.link === before)

        // Silence: three polls (2 s each) without a frame means the bridge is gone.
        var s = model()
        s.nowMs = 1000
        s.ingest(frame({ state: "live" }))
        s.nowMs = 1000 + 5900
        check("not silent inside 3 polls", s.link === "live")
        s.nowMs = 1000 + 6100
        check("silent after 3 polls", s.link === "bridge_silent" && s.silent)
        s.ingest(frame({ state: "live" }))
        check("a new frame ends the silence", s.link === "live")

        console.log("BridgeModelTest: " + passes + " passed, " + failures + " failed")
        for (var i = 0; i < log.length; i++) console.log(log[i])
    }
}
