pragma Singleton
import QtQuick

// Which of the six states the DESIGN-SAMPLE surfaces (Alt-Tab, notifications, Forge, inspector,
// settings, lock, workspace menu, search) show. These surfaces have no data behind them yet, so
// the state is chosen by hand here (key V cycles it); it is never read from a service.
QtObject {
    readonly property var states: ["normal", "vacio", "error", "offline", "degraded", "stale"]
    property string state: "normal"
    // Draws the visible-focus ring on the first focusable item (state "07 · Foco visible").
    property bool focusRing: false
    // Hides Vault and Stage so a surface sits on an empty desktop, as in the design boards.
    property bool bare: true
    // Graph viewer (the desktop sphere): Settings > Visor gráfico.
    property bool orbShow: true
    property int orbProfile: 1          // 0 Sphere, 1 Orb, 2 Field
    property int orbBrightI: 1
    property int orbActI: 2
    property bool orbSpin: true
    property bool orbNav: false         // "navigate the sphere in 3D" (free camera); otherwise the camera is fixed
    function next() { state = states[(states.indexOf(state) + 1) % states.length] }
}
