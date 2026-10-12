import QtQuick

// ArchieOS professional shell: DESIGN PROTOTYPE (phase B). Runs on plain Qt Quick so it can be
// rendered without Quickshell:   qml Main.qml     (Qt 6.6+)
// The Quickshell host layer (PanelWindow, layer-shell) is not part of this file and is untested.
//
//   Ctrl+K palette   C control center   A activity   G grid/list   M reduced motion
//   1..4 theme: frost / workspace / signature / synthesis
Item {
    id: root
    width: 1920; height: 1080
    property string overlay: ""
    focus: true

    Keys.onPressed: function (e) {
        if (e.key === Qt.Key_Escape) overlay = ""
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_K) overlay = overlay === "palette" ? "" : "palette"
        else if (e.key === Qt.Key_C) overlay = overlay === "control" ? "" : "control"
        else if (e.key === Qt.Key_A) overlay = overlay === "activity" ? "" : "activity"
        else if (e.key === Qt.Key_G) vault.gridView = !vault.gridView
        else if (e.key === Qt.Key_M) Theme.reducedMotion = !Theme.reducedMotion
        else if (e.key === Qt.Key_1) Theme.variant = "frost"
        else if (e.key === Qt.Key_2) Theme.variant = "workspace"
        else if (e.key === Qt.Key_3) Theme.variant = "signature"
        else if (e.key === Qt.Key_4) Theme.variant = "synthesis"
    }

    Rectangle { anchors.fill: parent
        gradient: Gradient { GradientStop { position: 0; color: Theme.wall0 } GradientStop { position: 1; color: Theme.wall1 } } }

    // Fed by quickshell/BridgeProcess.qml in the shell, by a recorded frame in tests.
    property alias bridge: bridgeModel
    // Plain values for tests that cannot reach the model object.
    readonly property string bridgeLink: bridgeModel.link
    readonly property int bridgeRejected: bridgeModel.rejected
    BridgeModel { id: bridgeModel }

    // Test/replay only: a context property `replayFile` (a file URL of JSON lines written by
    // archie-shell-bridge) is fed through the same ingest() path. Needs QML_XHR_ALLOW_FILE_READ=1.
    property string replayDebug: ""
    Component.onCompleted: {
        replayDebug = typeof replayFile === "undefined" ? "no-context-property" : "file=" + replayFile
        if (typeof replayFile === "undefined" || replayFile === "") return
        var xhr = new XMLHttpRequest()
        xhr.open("GET", replayFile, false)
        xhr.send()
        var lines = xhr.responseText.split(/\r?\n/)
        replayDebug += " status=" + xhr.status + " bytes=" + xhr.responseText.length + " lines=" + lines.length
        for (var i = 0; i < lines.length; i++) if (lines[i].trim() !== "") bridgeModel.ingest(lines[i])
    }

    TopBar { id: bar; width: parent.width; overlay: root.overlay; bridge: bridgeModel
        onToggle: function (n) { root.overlay = root.overlay === n ? "" : n } }

    VaultWindow { id: vault; x: Math.round(parent.width * 0.13); y: Math.round(parent.height * 0.083); width: Math.round(parent.width * 0.615); height: Math.round(parent.height * 0.65) }

    // A second, smaller window shows the Stage as a child window next to Vault (ADR-GUI-002).
    Panel { x: Math.round(parent.width * 0.76); y: Math.round(parent.height * 0.139); width: Math.round(parent.width * 0.208); height: Math.round(parent.height * 0.278)
        Rectangle { width: parent.width; height: 34; color: Theme.raised; radius: Theme.radius
            Text { anchors.centerIn: parent; text: "Stage — apps de Archie"; color: Theme.ink; font.pixelSize: Theme.fsBody } }
        Column { anchors.centerIn: parent; spacing: 8
            Chip { text: "Autoridad: Archie"; glyph: "◉"; tone: Theme.accent }
            Text { text: "Tú ganas el control al tocar el ratón"; color: Theme.soft; font.pixelSize: Theme.fsSmall }
        }
    }

    Dock { id: dock; anchors.horizontalCenter: parent.horizontalCenter; y: parent.height - height - 14 }

    Item { anchors.fill: parent; visible: root.overlay !== ""
        MouseArea { anchors.fill: parent; onClicked: root.overlay = "" }
        ControlCenter { bridge: bridgeModel; visible: root.overlay === "control"; x: parent.width - width - 14; y: Theme.barH + 8 }
        ActivityCenter { visible: root.overlay === "activity"; x: parent.width - width - 14; y: Theme.barH + 8 }
        CommandPalette { visible: root.overlay === "palette"; x: (parent.width - width) / 2; y: 150 }
    }

    DesignBanner { x: 14; y: Theme.barH + 8; visible: root.overlay === "" }
}
