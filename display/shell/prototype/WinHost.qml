import QtQuick

// Holds one managed window (a child of this item): puts it at its home position plus the drag
// offset, at its place in the stacking order, and brings it to the front on any press inside it.
// The window itself animates in and out through its own `shown` (see Panel), driven from Main by
// Wm.isShown(wid).
Item {
    id: h
    property string wid
    property real baseX: 0
    property real baseY: 0
    default property alias content: holder.data
    readonly property bool maxed: Wm.isMax(wid)
    // Pixels of the title bar that must stay on screen.
    readonly property real keepX: 120
    readonly property real topEdge: 30            // under the top bar
    readonly property real homeX: Math.max(8,Math.min(baseX,(parent ? parent.width : 1920)-width-8))
    readonly property real homeY: Math.max(44,Math.min(baseY,(parent ? parent.height : 1080)-height-100))
    x: maxed ? 40 : homeX + Wm.ox(wid)
    y: maxed ? 44 : homeY + Wm.oy(wid)
    z: Wm.z(wid)
    width: holder.childrenRect.width
    height: holder.childrenRect.height
    // The window's own opacity, not its `visible` (that one already includes this item's, which would never come back).
    visible: holder.children.length > 0 && holder.children[0].opacity > 0
    enabled: Wm.isShown(wid)

    function clampOffset(ox, oy) {
        var maxX = (parent ? parent.width : 1920) - keepX - homeX
        var minX = keepX - width - homeX
        var maxY = (parent ? parent.height : 1080) - 80 - homeY
        var minY = topEdge - homeY
        return { x: Math.max(minX, Math.min(maxX, ox)), y: Math.max(minY, Math.min(maxY, oy)) }
    }
    Component.onCompleted: { var o = {}; for (var k in Wm.hosts) o[k] = Wm.hosts[k]; o[wid] = h; Wm.hosts = o }
    Component.onDestruction: {
        if (Wm.hosts[wid] !== h) return
        var remaining = {}; for (var key in Wm.hosts) if (key !== wid) remaining[key] = Wm.hosts[key]
        Wm.hosts = remaining
    }

    Item { id: holder }

    // Raise on press without taking the press from whatever is under it: a pointer handler only
    // watches (passive), so the window's own buttons and rows keep working.
    TapHandler { acceptedButtons: Qt.LeftButton | Qt.RightButton; gesturePolicy: TapHandler.DragThreshold
        grabPermissions: PointerHandler.ApprovesTakeOverByAnything
        onPressedChanged: if (pressed) Wm.raise(h.wid) }
}
