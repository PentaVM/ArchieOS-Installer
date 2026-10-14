import QtQuick
import QtQuick.Window
import "../../../display/shell/prototype" as Shell

// Normal-state render smoke. Uses design samples; does not attach services or input.
Window {
    id: win
    width: 1920; height: 1080; visible: true
    Shell.Main { id: desktop; width: win.width; height: win.height }
    property var views: ["", "control", "activity", "sentry", "palette", "switcher", "notif",
                         "avisos", "forge", "inspector", "settings", "workspace", "search", "lock"]
    property int step: 0
    Timer {
        interval: 200; running: true; repeat: true
        onTriggered: {
            if (win.step < win.views.length) {
                desktop.overlay = win.views[win.step++]
                return
            }
            stop()
            desktop.overlay = ""
            desktop.grabToImage(function(result) {
                console.log("REVIEW_RENDER " + result.saveToFile("review-desktop.png"))
                Qt.quit()
            })
        }
    }
    Timer {
        interval: 15000; running: true
        onTriggered: { console.error("REVIEW_RENDER_TIMEOUT"); Qt.quit() }
    }
}
