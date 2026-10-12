import QtQuick

// ArchieOS professional shell: Archie Mac design (phase B, restyled from the Claude Design
// handoff). Plain Qt Quick so it renders without Quickshell:   qml Main.qml     (Qt 6.6+)
// The Quickshell host layer (PanelWindow, layer-shell) is not part of this file and is untested.
//
//   Ctrl+K palette   C control center   A activity   S sentry   G grid/list
//   M reduced motion   O opaque surfaces   B compositor blur (glass)   Esc close
//
// Positions follow the design's 1920x1080 board: Vault 88/100 (list) or 88/92 (grid), Stage
// 110 px from the right edge, right panels 24 px from the right edge, Sentry and palette centred.
Item {
    id: root
    width: 1920; height: 1080
    property string overlay: ""
    focus: true

    property alias bridge: bridgeModel
    // Plain values for tests that cannot reach the model object.
    readonly property string bridgeLink: bridgeModel.link
    readonly property int bridgeRejected: bridgeModel.rejected
    readonly property bool opaque: Theme.opaque

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

    Keys.onPressed: function (e) {
        if (e.key === Qt.Key_Escape) overlay = ""
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_K) overlay = overlay === "palette" ? "" : "palette"
        else if (e.key === Qt.Key_C) overlay = overlay === "control" ? "" : "control"
        else if (e.key === Qt.Key_S) overlay = overlay === "sentry" ? "" : "sentry"
        else if (e.key === Qt.Key_A) overlay = overlay === "activity" ? "" : "activity"
        else if (e.key === Qt.Key_G) vault.gridView = !vault.gridView
        else if (e.key === Qt.Key_M) Theme.reducedMotion = !Theme.reducedMotion
        else if (e.key === Qt.Key_O) Theme.opaque = !Theme.opaque
        else if (e.key === Qt.Key_B) Theme.blur = !Theme.blur
    }

    Background { anchors.fill: parent }

    TopBar { id: bar; width: parent.width; overlay: root.overlay; bridge: bridgeModel
        app: root.overlay === "sentry" ? "Sentry" : "Vault"
        onToggle: function (n) { root.overlay = root.overlay === n ? "" : n } }

    DesignBanner { x: 14; y: 42 }

    // Vault: list 1060x720 at (88,100); grid 1060x840 at (88,92).
    VaultWindow { id: vault; x: 88; y: gridView ? 92 : 100; width: 1060; height: gridView ? 840 : 720 }

    // Stage: a window of its own program. Its inside (compositor, agent cursor, captions) is
    // outside this redesign.
    Panel { kind: "window"; x: root.width - 600 - 110; y: 172; width: 600; height: 400
        Item { width: parent.width; height: 52
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.hair }
            Row { x: 16; anchors.verticalCenter: parent.verticalCenter; spacing: 8
                Repeater { model: 3; Rectangle { width: 12; height: 12; radius: 6; color: Theme.tlOff } } }
            Row { anchors.centerIn: parent
                Text { text: "Stage"; color: Theme.t1; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.DemiBold; font.letterSpacing: -0.17 }
                Text { text: " — apps de Archie"; color: Theme.t2; font.family: Theme.disp; font.pixelSize: 17; font.weight: Font.Medium; font.letterSpacing: -0.17 } } }
        Column { x: 20; y: 72; width: parent.width - 40; spacing: 16
            Row { spacing: 14
                Chip { icon: "archie"; tone: "arch"; text: "Autoridad: Archie"; anchors.verticalCenter: parent.verticalCenter }
                Row { spacing: 6; anchors.verticalCenter: parent.verticalCenter
                    Icon { name: "pointer"; size: 14; color: Theme.t3; anchors.verticalCenter: parent.verticalCenter }
                    Text { text: "Tú ganas el control al tocar el ratón"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13; anchors.verticalCenter: parent.verticalCenter } } }
            Item { width: parent.width; height: 400 - 72 - 20 - 34
                DashedRect { anchors.fill: parent; radius: Theme.rCard; color: Theme.hair }
                Rectangle { anchors.fill: parent; radius: Theme.rCard; color: Qt.rgba(0, 0, 0, 0.22) }
                DashedRect { anchors.fill: parent; radius: Theme.rCard; color: Theme.hair }
                Column { anchors.centerIn: parent; spacing: 4
                    Text { anchors.horizontalCenter: parent.horizontalCenter; text: "Compositor de Stage"; color: Theme.t2; font.family: Theme.font; font.pixelSize: 13 }
                    Text { anchors.horizontalCenter: parent.horizontalCenter; text: "otro programa · fuera de este rediseño"; color: Theme.t3; font.family: Theme.font; font.pixelSize: 11 } } } } }

    Dock { id: dock; anchors.horizontalCenter: parent.horizontalCenter; y: parent.height - height - 14
        onLaunched: function (id) { if (id === "sentry") root.overlay = root.overlay === "sentry" ? "" : "sentry" } }

    Item { anchors.fill: parent; visible: root.overlay !== ""
        MouseArea { anchors.fill: parent; onClicked: root.overlay = "" }
        ControlCenter { bridge: bridgeModel; visible: root.overlay === "control"; x: root.width - width - 24; y: 44 }
        ActivityCenter { bridge: bridgeModel; visible: root.overlay === "activity"; x: root.width - width - 24; y: 44 }
        SentryPanel { bridge: bridgeModel; visible: root.overlay === "sentry"; x: (root.width - width) / 2; y: 110 }
        CommandPalette { visible: root.overlay === "palette"; x: (root.width - width) / 2; y: 140 }
    }
}
