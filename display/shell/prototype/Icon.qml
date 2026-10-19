import QtQuick
import QtQuick.Shapes

// One Archie Mac line icon. 24x24 viewBox, round caps and joins, stroke in `color`.
// Stroke widths per size follow the handoff (12:3 · 14:2.6 · 16:2.25 · 18:2 · 20:1.8 ·
// 24:1.5 · 28:1.4 · 36:1.4, in viewBox units) so every size draws ~1.5 px.
Item {
    id: ic
    property string name
    property int size: 16
    property color color: Theme.t2
    property real strokeOverride: 0
    readonly property var entry: Icons.paths[name]
    readonly property real sw: strokeOverride > 0 ? strokeOverride
        : (size <= 12 ? 3 : size <= 14 ? 2.6 : size <= 16 ? 2.25 : size <= 18 ? 2 : size <= 20 ? 1.8 : size <= 24 ? 1.5 : 1.4)
    width: size; height: size
    implicitWidth: size; implicitHeight: size

    Shape {
        width: 24; height: 24
        scale: ic.size / 24
        transformOrigin: Item.TopLeft
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: ic.color; fillColor: "transparent"
            strokeWidth: ic.sw
            capStyle: ShapePath.RoundCap; joinStyle: ShapePath.RoundJoin
            PathSvg { path: ic.entry ? ic.entry.d : "" }
        }
        ShapePath {
            strokeColor: ic.entry && ic.entry.dots ? ic.color : "transparent"; fillColor: "transparent"
            strokeWidth: ic.sw * 2.2
            capStyle: ShapePath.RoundCap; joinStyle: ShapePath.RoundJoin
            PathSvg { path: ic.entry && ic.entry.dots ? ic.entry.dots : "M0 0" }
        }
    }
}
