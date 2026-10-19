import QtQuick
import QtQuick.Shapes

// A rounded rectangle outline with a dashed stroke. Used for every "unknown / no reading /
// design sample" mark: the handoff's rule is that what has no reading looks different.
Shape {
    id: d
    property color color: Theme.hair
    property real radius: 8
    property real strokeWidth: 1
    property var dash: [3, 3]
    preferredRendererType: Shape.CurveRenderer
    ShapePath {
        strokeColor: d.color; fillColor: "transparent"; strokeWidth: d.strokeWidth
        strokeStyle: ShapePath.DashLine; dashPattern: d.dash
        PathRectangle {
            x: d.strokeWidth / 2; y: d.strokeWidth / 2
            width: d.width - d.strokeWidth; height: d.height - d.strokeWidth
            radius: d.radius
        }
    }
}
