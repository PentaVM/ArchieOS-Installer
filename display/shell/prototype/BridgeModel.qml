import QtQuick

// The shell's view of the node, fed by `archie-shell-bridge watch` (one JSON line per poll,
// schema 1; core/crates/archie-shell-bridge/src/protocol.rs). Pure QML: it neither reads
// files nor starts processes. A host feeds it lines with ingest(): in the shell that is
// quickshell/BridgeProcess.qml, in tests a recorded frame.
//
// Rule: the model never claims more than the last frame proved. A frame that is too old
// makes the link "bridge_silent"; a line it does not understand is counted and dropped,
// never half-applied.
QtObject {
    id: m
    readonly property int schema: 1
    // How often the bridge polls (seconds); silence is 3 polls without a frame.
    property int pollSeconds: 2
    // Wall clock in ms, advanced by `tick()`; injected so tests are deterministic.
    property real nowMs: Date.now()

    property var frame: null
    property real receivedAtMs: 0
    property int rejected: 0
    property string lastRejection: ""

    readonly property bool received: frame !== null
    readonly property real ageMs: received ? Math.max(0, nowMs - receivedAtMs) : 0
    readonly property bool silent: received && ageMs > pollSeconds * 3000

    // waiting | live | fixture | daemon_unreachable | displayd_offline | invalid | bridge_silent
    readonly property string link: !received ? "waiting" : (silent ? "bridge_silent" : frame.link.state)
    readonly property string reason: received && frame.link.reason ? frame.link.reason : ""
    readonly property real staleMs: received && frame.link.stale_ms !== undefined ? frame.link.stale_ms : 0

    readonly property var system: received ? frame.system : null
    readonly property var kernel: received ? frame.kernel : null
    // LAN inventory file read by the bridge. Frames from before this field existed count as not configured.
    readonly property var lan: received && frame.lan ? frame.lan : ({ state: "not_configured" })
    // Long bridge jobs, read-only. Same rule as lan for older frames.
    readonly property var jobs: received && frame.jobs ? frame.jobs : ({ state: "not_configured" })
    readonly property int runningJobs: jobs.state === "ok" ? jobs.jobs.filter(function (j) { return j.state === "running" || j.state === "starting" }).length : 0

    function tick() { nowMs = Date.now() }

    // Returns true when the line was a valid frame of a schema this model understands.
    function ingest(line) {
        var f
        try { f = JSON.parse(line) } catch (e) { return reject("not JSON") }
        if (!f || typeof f !== "object") return reject("not an object")
        if (f.schema !== schema) return reject("schema " + f.schema + " (expected " + schema + ")")
        if (!f.link || typeof f.link.state !== "string") return reject("no link state")
        frame = f
        receivedAtMs = nowMs
        return true
    }
    function reject(why) { rejected += 1; lastRejection = why; return false }
}
