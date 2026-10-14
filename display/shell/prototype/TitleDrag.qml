import QtQuick

// Put inside a window's title bar: dragging this strip moves the window (through the window
// manager, which keeps the title bar on screen). Pressing also brings the window to the front.
// The strip leaves out the lights on the left (and `reserveRight` px on the right for controls):
// a drag handler over a button would take its click away.
Item {
    id: d
    property string wid
    property real reserveLeft: 72
    property real reserveRight: 0
    x: reserveLeft
    width: parent.width - reserveLeft - reserveRight
    height: parent.height

    DragHandler {
        property real startX: 0
        property real startY: 0
        target: null
        onActiveChanged: if (active) { startX = Wm.ox(d.wid); startY = Wm.oy(d.wid); Wm.raise(d.wid) }
        onTranslationChanged: if (active) Wm.moveTo(d.wid, startX + translation.x, startY + translation.y)
    }
}
