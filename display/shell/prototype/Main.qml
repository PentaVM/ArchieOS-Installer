import QtQuick

// ArchieOS professional shell: Archie Mac design (phase B, restyled from the Claude Design
// handoff). Plain Qt Quick so it renders without Quickshell:   qml Main.qml     (Qt 6.6+)
// The Quickshell host layer (PanelWindow, layer-shell) is not part of this file and is untested.
//
//   Ctrl+K palette   C control center   A activity   S sentry   G grid/list
//   W windows (Alt-Tab)   N notifications   T toasts + authorization   F forge   I inspector
//   , settings   L lock   Shift+L installer   X workspace menu   / search
//   V cycle the state shown by the design-sample surfaces   . visible-focus ring   H bare desktop
//   M reduced motion   P motion profile (full / balanced / quiet)   O opaque   B glass   Esc close
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

    function toggle(name) { overlay = overlay === name ? "" : name }

    property bool vaultOpen: true
    property bool zoomed: false
    property string menuName: ""
    onOverlayChanged: if (overlay !== "palette" && overlay !== "search") root.forceActiveFocus()

    // Everything the menus, palette, search, dock, switcher and keys can do. Vault data stays
    // sample data; opening a file only says there is no backend.
    function run(id) {
        if (id.indexOf("go:") === 0) { vaultOpen = true; Demo.bare = false; vault.goPlace(parseInt(id.slice(3))); return }
        if (id.indexOf("file:") === 0) { vaultOpen = true; Demo.bare = false; vault.reveal(id.slice(5)); return }
        if (id.indexOf("overlay.") === 0) { var o = id.slice(8); overlay = ""; if (o === "lock") lock.installer = false; overlay = o; return }
        switch (id) {
        case "app.orb": overlay = ""; Demo.bare = !Demo.bare; break
        case "app.vault": overlay = ""; Demo.bare = false; vaultOpen = true; break
        case "app.stage": overlay = ""; Demo.bare = false; break
        case "app.forge": toggle("forge"); break
        case "app.sentry": toggle("sentry"); break
        case "app.set": toggle("settings"); break
        case "vault.newTab": vaultOpen = true; vault.newTab(); break
        case "vault.closeTab": vault.closeTab(vault.tabIndex); break
        case "vault.open": vault.openSelected(); break
        case "vault.up": vault.up(); break
        case "vault.cancel": vault.cancelCopy(); break
        case "vault.undo": vault.undoCopy(); break
        case "vault.redo": vault.repeatCopy(); break
        case "vault.sort": vault.toggleSort(); break
        case "vault.list": vault.gridView = false; break
        case "vault.grid": vault.gridView = true; break
        case "theme.motion": Theme.reducedMotion = !Theme.reducedMotion; break
        case "theme.opaque": Theme.opaque = !Theme.opaque; break
        case "theme.blur": Theme.blur = !Theme.blur; break
        case "desk.toggle": Demo.bare = !Demo.bare; break
        case "orb.nav": overlay = ""; Demo.bare = true; Demo.orbNav = !Demo.orbNav; break
        }
    }

    Keys.onPressed: function (e) {
        if (e.key === Qt.Key_Escape) overlay = ""
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_K) overlay = overlay === "palette" ? "" : "palette"
        else if (e.key === Qt.Key_C) overlay = overlay === "control" ? "" : "control"
        else if (e.key === Qt.Key_S) overlay = overlay === "sentry" ? "" : "sentry"
        else if (e.key === Qt.Key_A) overlay = overlay === "activity" ? "" : "activity"
        else if (e.key === Qt.Key_Escape && Demo.orbNav) Demo.orbNav = false
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_Space) run("orb.nav")
        else if (e.key === Qt.Key_G) vault.gridView = !vault.gridView
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_T) run("vault.newTab")
        else if (overlay === "" && e.key === Qt.Key_Down) vault.move(vault.gridView ? 4 : 1)
        else if (overlay === "" && e.key === Qt.Key_Up) vault.move(vault.gridView ? -4 : -1)
        else if (overlay === "" && e.key === Qt.Key_Right && vault.gridView) vault.move(1)
        else if (overlay === "" && e.key === Qt.Key_Left && vault.gridView) vault.move(-1)
        else if (overlay === "" && (e.key === Qt.Key_Return || e.key === Qt.Key_Enter)) vault.openSelected()
        else if (overlay === "" && e.key === Qt.Key_Backspace) vault.up()
        else if (e.key === Qt.Key_W) toggle("switcher")
        else if (e.key === Qt.Key_N) toggle("notif")
        else if (e.key === Qt.Key_T) toggle("avisos")
        else if (e.key === Qt.Key_F) toggle("forge")
        else if (e.key === Qt.Key_I) toggle("inspector")
        else if (e.key === Qt.Key_Comma) toggle("settings")
        else if (e.key === Qt.Key_L) { lock.installer = !!(e.modifiers & Qt.ShiftModifier); toggle("lock") }
        else if (e.key === Qt.Key_X) toggle("workspace")
        else if (e.key === Qt.Key_Slash) toggle("search")
        else if (e.key === Qt.Key_V) Demo.next()
        else if (e.key === Qt.Key_Period) Demo.focusRing = !Demo.focusRing
        else if (e.key === Qt.Key_H) Demo.bare = !Demo.bare
        else if (e.key === Qt.Key_P) Theme.motion = Theme.motion === "full" ? "balanced" : (Theme.motion === "balanced" ? "quiet" : "full")
        else if (e.key === Qt.Key_M) Theme.reducedMotion = !Theme.reducedMotion
        else if (e.key === Qt.Key_O) Theme.opaque = !Theme.opaque
        else if (e.key === Qt.Key_B) Theme.blur = !Theme.blur
    }

    // The plain backdrop shows until the graph viewer is up (or when it is turned off in Settings).
    Background { anchors.fill: parent; nodes: !Demo.orbShow || !live.ok }

    // Graph viewer: the Lumenfield 0449 sphere over the saved graph of the OS content
    // (display/visor), part of the desktop, always behind the windows.
    LiveOrb { id: live; anchors.fill: parent; visible: Demo.orbShow || Demo.orbNav }

    TopBar { id: bar; width: parent.width; overlay: root.overlay; bridge: bridgeModel; menu: root.overlay === "menu" ? root.menuName : ""
        onMenuClicked: function (n, x) { if (root.overlay === "menu" && root.menuName === n) root.overlay = ""; else { root.menuName = n; menuDrop.x = x; root.overlay = "menu" } }
        app: root.overlay === "sentry" ? "Sentry" : root.overlay === "forge" ? "Forge" : root.overlay === "settings" ? "Ajustes" : "Vault"
        onToggle: function (n) { root.overlay = root.overlay === n ? "" : n } }

    DesignBanner { x: 14; y: 42 }

    // Vault: list 1060x720 at (88,100); grid 1060x840 at (88,92).
    VaultWindow { id: vault; visible: !Demo.bare && root.vaultOpen
        x: (root.zoomed ? 40 : 88) + dx; y: (root.zoomed ? 44 : (gridView ? 92 : 100)) + dy
        width: root.zoomed ? root.width - 80 : 1060; height: root.zoomed ? root.height - 44 - 110 : (gridView ? 840 : 720)
        onCloseRequested: root.vaultOpen = false
        onZoomRequested: { root.zoomed = !root.zoomed; vault.dx = 0; vault.dy = 0 } }

    // Stage: a window of its own program. Its inside (compositor, agent cursor, captions) is
    // outside this redesign.
    Panel { kind: "window"; visible: !Demo.bare; x: root.width - 600 - 110; y: 172; width: 600; height: 400
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

    Rectangle { visible: Demo.orbNav; z: 50; anchors.horizontalCenter: parent.horizontalCenter; y: 40; width: navTxt.implicitWidth + 28; height: 30; radius: 15
        MouseArea { anchors.fill: parent; onClicked: Demo.orbNav = false }
        color: Qt.rgba(2 / 255, 2 / 255, 4 / 255, 0.9); border.width: 1; border.color: Theme.hair
        Text { id: navTxt; anchors.centerIn: parent; text: "Navegando la esfera en 3D · Esc o clic aquí para fijar la cámara"
            color: Theme.t2; font.family: Theme.font; font.pixelSize: 12 } }

    Dock { id: dock; anchors.horizontalCenter: parent.horizontalCenter; y: parent.height - height - 14
        vaultOpen: root.vaultOpen
        current: root.overlay === "sentry" ? "sentry" : root.overlay === "forge" ? "forge" : root.overlay === "settings" ? "set" : (Demo.bare ? "orb" : "vault")
        onLaunched: function (id) { root.run("app." + id) } }

    // Overlays. Each panel reveals itself with `shown` (opacity plus a short slide or scale; see
    // Theme motion tokens), so closing fades out instead of vanishing.
    Item { anchors.fill: parent
        MouseArea { anchors.fill: parent; enabled: root.overlay !== "" && root.overlay !== "lock"; onClicked: root.overlay = "" }
        ControlCenter { bridge: bridgeModel; shown: root.overlay === "control"; enterDy: -8; x: root.width - width - 24; y: 44 }
        ActivityCenter { bridge: bridgeModel; shown: root.overlay === "activity"; enterDy: -8; x: root.width - width - 24; y: 44 }
        SentryPanel { bridge: bridgeModel; shown: root.overlay === "sentry"; enterDy: -8; x: (root.width - width) / 2; y: 110 }
        CommandPalette { onActivated: function (act) { root.overlay = ""; root.run(act) }; shown: root.overlay === "palette"; enterDy: -8; durIn: Theme.durPalette; durOut: Theme.durPaletteClose; x: (root.width - width) / 2; y: 140 }

        // Surfaces added by the second handoff. All design samples (see Demo.qml).
        WindowSwitcher { onChosen: function (k) { root.overlay = ""; if (k !== "") root.run("app." + k) }; shown: root.overlay === "switcher"; enterDy: -8; durIn: Theme.durPalette; durOut: Theme.durPaletteClose; x: (root.width - width) / 2; y: 250 }
        NotificationCenter { shown: root.overlay === "notif"; enterDy: -8; x: root.width - width - 24; y: 44 }
        Toasts { shown: root.overlay === "avisos"; x: root.width - width - 14; y: 44 }
        AuthDialog { shown: root.overlay === "avisos"; x: 620; y: 340; onRejected: root.overlay = "" }
        ForgeWindow { shown: root.overlay === "forge"; x: 460; y: 130; width: 1000; height: 560 }
        InspectorPanel { shown: root.overlay === "inspector"; enterDy: -8; x: root.width - width - 24; y: 44 }
        SettingsWindow { shown: root.overlay === "settings"; x: 490; y: 140; width: 940; height: 520 }
        OrbHole { visible: root.overlay === "workspace" && Demo.bare; x: 700; y: 250 }
        WorkspaceMenu { onChosen: function (w) { root.overlay = ""; Demo.bare = (w === "orb") }; shown: root.overlay === "workspace"; x: 1338; y: 42 }
        MenuDropdown { id: menuDrop; vault: vault; name: root.menuName; shown: root.overlay === "menu"; y: 34; enterDy: -6
            onActivated: function (id) { root.overlay = ""; root.run(id) } }
        SearchPanel { onActivated: function (act) { root.overlay = ""; root.run(act) }; shown: root.overlay === "search"; x: (root.width - width) / 2; y: 140 }
    }
    LockScreen { id: lock; visible: root.overlay === "lock" }
}
