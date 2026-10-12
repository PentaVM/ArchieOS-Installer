// UNVERIFIED: written against Quickshell's documented Process + SplitParser API (v0.3.x) and
// NOT run: there is no Quickshell on the machine this was written on. Treat it as a
// specification of the host layer until it is run on Linux (docs/gui/TEST_REPORT.md).
//
// Starts `archie-shell-bridge watch` with a FIXED argument list (nothing here is built from
// user input) and feeds each JSON line to a BridgeModel. If the bridge exits it is restarted
// after 3 s; while it is down the model goes `bridge_silent` by itself because no frames
// arrive, so the shell never keeps showing a "connected" picture.
import QtQuick
import Quickshell.Io

Process {
    id: bridge
    required property var model          // a BridgeModel (display/shell/prototype/BridgeModel.qml)

    // Paths are the deployed ones (modules/sentry/lan.toml, docs/WSL-ARCHITECTURE.md), fixed here:
    // the bridge reads them, never writes them, and a missing file is reported as missing.
    command: ["archie-shell-bridge", "watch",
              "--lan-file", "/var/lib/archie/sentry/lan.json",
              "--workspaces-root", "/archie/workspaces"]
    running: true

    stdout: SplitParser {
        onRead: line => bridge.model.ingest(line)
    }

    onExited: restart.start()

    property Timer restartTimer: Timer {
        id: restart
        interval: 3000
        onTriggered: bridge.running = true
    }
    // Advances the model's clock so that silence can be detected.
    property Timer clock: Timer {
        interval: 1000
        running: true
        repeat: true
        onTriggered: bridge.model.tick()
    }
}
