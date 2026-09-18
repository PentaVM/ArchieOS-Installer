// A sheet of paper for the agent to draw on.
//
// It has no controls and nothing to find: the pointer going down, along and up is the whole
// of what it understands, which is what a stroke is. It saves what has been drawn every few
// seconds, so the picture survives the run that made it.
//
//     QT_QUICK_BACKEND=software qml paint.qml
import QtQuick
import QtQuick.Window

Window {
    id: root
    width: 900
    height: 700
    visible: true
    color: "#ffffff"
    title: "Lienzo"

    property int strokes: 0
    /// Where the picture is written. Set with `qml -- paint.qml --out <path>` is not a
    /// thing, so it is an environment variable the demo scripts set.
    property string out: Qt.application.arguments.length > 1
                         ? Qt.application.arguments[Qt.application.arguments.length - 1]
                         : "/tmp/painting.png"

    function report() {
        console.log("paint: layout", root.width, root.height, "strokes", root.strokes)
    }
    Component.onCompleted: report()
    onWidthChanged: report()
    onHeightChanged: report()

    Canvas {
        id: paper
        anchors.fill: parent
        renderStrategy: Canvas.Immediate
        renderTarget: Canvas.Image
        property var line: []
        property bool papered: false

        onPaint: {
            const ctx = getContext("2d")
            if (!paper.papered) {
                ctx.fillStyle = "#ffffff"
                ctx.fillRect(0, 0, paper.width, paper.height)
                paper.papered = true
            }
            if (paper.line.length < 2)
                return
            ctx.strokeStyle = "#161616"
            ctx.lineWidth = 2
            ctx.lineCap = "round"
            ctx.lineJoin = "round"
            ctx.beginPath()
            ctx.moveTo(paper.line[0].x, paper.line[0].y)
            for (var i = 1; i < paper.line.length; ++i)
                ctx.lineTo(paper.line[i].x, paper.line[i].y)
            ctx.stroke()
            // The last point stays, so the next piece of the same line joins this one.
            paper.line = [paper.line[paper.line.length - 1]]
        }
    }

    MouseArea {
        anchors.fill: parent
        onPressed: (mouse) => {
            paper.line = [Qt.point(mouse.x, mouse.y)]
            paper.requestPaint()
        }
        onPositionChanged: (mouse) => {
            if (!pressed)
                return
            paper.line.push(Qt.point(mouse.x, mouse.y))
            paper.requestPaint()
        }
        onReleased: (mouse) => {
            paper.line.push(Qt.point(mouse.x, mouse.y))
            paper.requestPaint()
            root.strokes += 1
            if (root.strokes % 25 === 0)
                console.log("paint: strokes", root.strokes)
        }
    }

    Timer {
        interval: 4000
        running: true
        repeat: true
        onTriggered: {
            if (root.strokes > 0 && paper.save(root.out))
                console.log("paint: saved", root.strokes, "strokes to", root.out)
        }
    }
}
