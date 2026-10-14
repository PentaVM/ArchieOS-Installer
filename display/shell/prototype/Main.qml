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
    readonly property var wmState: Wm.st
    readonly property string wmFront: Wm.front()
    readonly property var wmOffsets: Wm.off

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

    property string menuName: ""
    onOverlayChanged: if (overlay !== "palette" && overlay !== "search") root.forceActiveFocus()

    // Everything the menus, palette, search, dock, switcher and keys can do. Vault data stays
    // sample data; opening a file only says there is no backend.
    function run(id) {
        if (id.indexOf("go:") === 0) { Wm.open("vault"); vault.goPlace(parseInt(id.slice(3))); return }
        if (id.indexOf("file:") === 0) { Wm.open("vault"); vault.reveal(id.slice(5)); return }
        if (id.indexOf("dock.") === 0) { overlay = ""; Wm.dockClick(id.slice(5)); return }
        if (id.indexOf("overlay.") === 0) { var o = id.slice(8); overlay = ""; if (o === "lock") lock.installer = false; overlay = o; return }
        switch (id) {
        case "app.orb": overlay = ""; Demo.bare = !Demo.bare; break
        case "app.vault": case "app.forge": case "app.sentry": case "app.stage": case "app.set": overlay = ""; Wm.open(id.slice(4)); break
        case "win.close": Wm.closeFront(); break
        case "win.min": Wm.minimizeFront(); break
        case "win.zoom": Wm.zoomFront(); break
        case "win.closeAll": Wm.closeAll(); break
        case "win.minAll": Wm.minimizeAll(); break
        case "win.showAll": overlay = ""; Wm.showAll(); break
        case "win.desktop": overlay = ""; Demo.bare = !Demo.bare; break
        case "vault.newTab": Wm.open("vault"); vault.newTab(); break
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
        else if (e.key === Qt.Key_S) Wm.toggle("sentry")
        else if (e.key === Qt.Key_A) overlay = overlay === "activity" ? "" : "activity"
        else if (e.key === Qt.Key_Escape && Demo.orbNav) Demo.orbNav = false
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_Space) run("orb.nav")
        else if (e.key === Qt.Key_G && Wm.isOpen("vault")) vault.gridView = !vault.gridView
        else if (overlay === "switcher" && e.key === Qt.Key_Tab) switcher.step(1)
        else if (overlay === "switcher" && e.key === Qt.Key_Backtab) switcher.step(-1)
        else if (overlay === "switcher" && (e.key === Qt.Key_Return || e.key === Qt.Key_Enter)) switcher.choose()
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_T) run("vault.newTab")
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_W) run((e.modifiers & Qt.ShiftModifier) ? "win.closeAll" : "win.close")
        else if ((e.modifiers & Qt.ControlModifier) && e.key === Qt.Key_M) run((e.modifiers & Qt.ShiftModifier) ? "win.minAll" : "win.min")
        else if (e.key === Qt.Key_D) run("win.desktop")
        else if (overlay === "" && Wm.front() === "vault" && e.key === Qt.Key_Down) vault.move(vault.gridView ? 4 : 1)
        else if (overlay === "" && Wm.front() === "vault" && e.key === Qt.Key_Up) vault.move(vault.gridView ? -4 : -1)
        else if (overlay === "" && Wm.front() === "vault" && e.key === Qt.Key_Right && vault.gridView) vault.move(1)
        else if (overlay === "" && Wm.front() === "vault" && e.key === Qt.Key_Left && vault.gridView) vault.move(-1)
        else if (overlay === "" && Wm.front() === "vault" && (e.key === Qt.Key_Return || e.key === Qt.Key_Enter)) vault.openSelected()
        else if (overlay === "" && Wm.front() === "vault" && e.key === Qt.Key_Backspace) vault.up()
        else if (e.key === Qt.Key_W) toggle("switcher")
        else if (e.key === Qt.Key_N) toggle("notif")
        else if (e.key === Qt.Key_T) toggle("avisos")
        else if (e.key === Qt.Key_F) Wm.toggle("forge")
        else if (e.key === Qt.Key_I) toggle("inspector")
        else if (e.key === Qt.Key_Comma) Wm.toggle("set")
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

    TopBar { id: bar; z: 600; width: parent.width; overlay: root.overlay; bridge: bridgeModel; menu: root.overlay === "menu" ? root.menuName : ""
        onMenuClicked: function (n, x) { if (root.overlay === "menu" && root.menuName === n) root.overlay = ""; else { root.menuName = n; menuDrop.x = x; root.overlay = "menu" } }
        app: Wm.frontTitle()
        onToggle: function (n) { root.overlay = root.overlay === n ? "" : n } }

    DesignBanner { x: 14; y: 42; z: 550 }

    // Managed windows (see Wm): each is wrapped in a WinHost that places it, stacks it and raises it on
    // a press. `shown` comes from the window manager, so minimising or closing fades it out.
    // Vault: list 1060x720 at (88,100); grid 1060x840 at (88,92); zoomed fills the desktop area.
    WinHost { wid: "vault"; baseX: 88; baseY: vault.gridView ? 92 : 100
        VaultWindow { id: vault; shown: Wm.isShown("vault"); enterDy: Wm.isMin("vault") ? 160 : 0; enterScale: Wm.isMin("vault") ? 0.8 : 0.96
            width: Wm.isMax("vault") ? root.width - 80 : 1060; height: Wm.isMax("vault") ? root.height - 44 - 110 : (gridView ? 840 : 720)
            onCloseRequested: Wm.close("vault") } }

    // Stage: a window of its own program (see StageWindow).
    WinHost { wid: "stage"; baseX: root.width - 600 - 110; baseY: 172
        StageWindow { shown: Wm.isShown("stage"); enterDy: Wm.isMin("stage") ? 160 : 0; enterScale: Wm.isMin("stage") ? 0.8 : 0.96
            width: Wm.isMax("stage") ? root.width - 80 : 600; height: Wm.isMax("stage") ? root.height - 44 - 110 : 400 } }

    WinHost { wid: "forge"; baseX: 460; baseY: 130
        ForgeWindow { wid: "forge"; shown: Wm.isShown("forge"); enterDy: Wm.isMin("forge") ? 160 : 0; enterScale: Wm.isMin("forge") ? 0.8 : 0.96
            width: Wm.isMax("forge") ? root.width - 80 : 1000; height: Wm.isMax("forge") ? root.height - 44 - 110 : 560 } }

    WinHost { wid: "set"; baseX: 490; baseY: 140
        SettingsWindow { wid: "set"; shown: Wm.isShown("set"); enterDy: Wm.isMin("set") ? 160 : 0; enterScale: Wm.isMin("set") ? 0.8 : 0.96
            width: Wm.isMax("set") ? root.width - 80 : 940; height: Wm.isMax("set") ? root.height - 44 - 110 : 520 } }

    WinHost { wid: "sentry"; baseX: (root.width - 780) / 2; baseY: 110
        SentryPanel { bridge: bridgeModel; shown: Wm.isShown("sentry"); enterDy: Wm.isMin("sentry") ? 160 : 0; enterScale: Wm.isMin("sentry") ? 0.8 : 0.96 } }

    Rectangle { visible: Demo.orbNav; z: 700; anchors.horizontalCenter: parent.horizontalCenter; y: 40; width: navTxt.implicitWidth + 28; height: 30; radius: 15
        MouseArea { anchors.fill: parent; onClicked: Demo.orbNav = false }
        color: Qt.rgba(2 / 255, 2 / 255, 4 / 255, 0.9); border.width: 1; border.color: Theme.hair
        Text { id: navTxt; anchors.centerIn: parent; text: "Navegando la esfera en 3D · Esc o clic aquí para fijar la cámara"
            color: Theme.t2; font.family: Theme.font; font.pixelSize: 12 } }

    Dock { id: dock; z: 500; anchors.horizontalCenter: parent.horizontalCenter; y: parent.height - height - 14
        onLaunched: function (id) { root.run(id === "orb" ? "app.orb" : "dock." + id) }
        onMenuRequested: function (id, cx) { dockMenu.target = id; dockMenu.x = dock.x + cx - dockMenu.width / 2; root.overlay = "dockmenu" } }

    // Overlays. Each panel reveals itself with `shown` (opacity plus a short slide or scale; see
    // Theme motion tokens), so closing fades out instead of vanishing.
    Item { anchors.fill: parent; z: 1000
        MouseArea { anchors.fill: parent; enabled: root.overlay !== "" && root.overlay !== "lock"; onClicked: root.overlay = "" }
        ControlCenter { bridge: bridgeModel; shown: root.overlay === "control"; enterDy: -8; x: root.width - width - 24; y: 44 }
        ActivityCenter { bridge: bridgeModel; shown: root.overlay === "activity"; enterDy: -8; x: root.width - width - 24; y: 44 }
        CommandPalette { onActivated: function (act) { root.overlay = ""; root.run(act) }; shown: root.overlay === "palette"; enterDy: -8; durIn: Theme.durPalette; durOut: Theme.durPaletteClose; x: (root.width - width) / 2; y: 140 }

        // Surfaces added by the second handoff. All design samples (see Demo.qml).
        WindowSwitcher { id: switcher; onChosen: function (k) { root.overlay = ""; if (k !== "") root.run("app." + k) }; shown: root.overlay === "switcher"; enterDy: -8; durIn: Theme.durPalette; durOut: Theme.durPaletteClose; x: (root.width - width) / 2; y: 250 }
        NotificationCenter { shown: root.overlay === "notif"; enterDy: -8; x: root.width - width - 24; y: 44 }
        Toasts { shown: root.overlay === "avisos" && !Demo.silent; x: root.width - width - 14; y: 44 }
        AuthDialog { shown: root.overlay === "avisos"; x: 620; y: 340; onRejected: root.overlay = "" }
        DockMenu { id: dockMenu; shown: root.overlay === "dockmenu"; y: dock.y - height - 8; onActivated: root.overlay = "" }
        InspectorPanel { shown: root.overlay === "inspector"; enterDy: -8; x: root.width - width - 24; y: 44 }
        OrbHole { visible: root.overlay === "workspace" && Demo.bare; x: 700; y: 250 }
        WorkspaceMenu { onChosen: function (w) { root.overlay = ""; Demo.bare = (w === "orb") }; shown: root.overlay === "workspace"; x: 1338; y: 42 }
        MenuDropdown { id: menuDrop; vault: vault; name: root.menuName; shown: root.overlay === "menu"; y: 34; enterDy: -6
            onActivated: function (id) { root.overlay = ""; root.run(id) } }
        SearchPanel { onActivated: function (act) { root.overlay = ""; root.run(act) }; shown: root.overlay === "search"; x: (root.width - width) / 2; y: 140 }
    }
    LockScreen { id: lock; z: 2000; visible: root.overlay === "lock"; onLeave: root.overlay = "" }
}
