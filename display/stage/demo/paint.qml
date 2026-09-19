// A sheet of paper for the agent to draw on.
//
// The paper understands the pointer going down, along and up, which is what a stroke is.
// Below it is a palette: clicking a swatch picks that colour up, which is the whole of what
// a colour change is from outside. It saves what has been drawn every few seconds, so the
// picture survives the run that made it.
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
    /// The colours this canvas can be drawn with, and which one is in hand.
    property var swatches: ["#141210", "#4a352a", "#8a6a46", "#c39c6d", "#e6d3b3", "#5d6b63"]
    property string ink: "#141210"
    /// How wide the pen is. A drawing of many lines wants a fine one, because lines closer
    /// together than the pen is wide leave no paper between them.
    property int nib: {
        const args = Qt.application.arguments
        const last = args.length > 1 ? parseInt(args[args.length - 1], 10) : NaN
        return isNaN(last) ? 2 : Math.max(1, last)
    }
    /// Where the picture is written. Set with `qml -- paint.qml --out <path>` is not a
    /// thing, so it is the last argument the demo scripts pass, before the pen's width if
    /// one is given.
    property string out: {
        const args = Qt.application.arguments
        if (args.length < 2)
            return "/tmp/painting.png"
        const back = isNaN(parseInt(args[args.length - 1], 10)) ? 1 : 2
        return args.length > back ? args[args.length - back] : "/tmp/painting.png"
    }

    /// Where the paper is and where the palette is, in the window's own pixels. An agent
    /// that is told this does not have to guess which part of the window is paper, and does
    /// not draw a line through the palette and change colour by accident.
    function report() {
        console.log("paint: layout", root.width, root.height, "strokes", root.strokes,
                    "pen", root.nib,
                    "paper", Math.round(paper.x), Math.round(paper.y),
                    Math.round(paper.width), Math.round(paper.height),
                    "palette", Math.round(tray.x + palette.x + 16), Math.round(tray.y + palette.y + 16), 40)
    }
    Component.onCompleted: report()
    onWidthChanged: report()
    onHeightChanged: report()

    Canvas {
        id: paper
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: tray.top
        renderStrategy: Canvas.Immediate
        renderTarget: Canvas.Image
        // Qt throws the canvas's picture away when the canvas changes size, so the paper
        // has to be laid again — otherwise the drawing is made on nothing, and every stroke
        // of it is there while the paper behind it is not. A picture saved from that is
        // transparent, which looks like paper in one viewer and like a black ground in the
        // next, and neither of them is what was drawn.
        onWidthChanged: { paper.papered = false; paper.requestPaint(); root.report() }
        onHeightChanged: { paper.papered = false; paper.requestPaint(); root.report() }

        /// The pieces waiting to be inked: each one a colour and the points drawn with it.
        /// A paint runs when the scene gets to it, which may be several strokes after the
        /// pointer moved, so nothing is thrown away until it has actually been drawn.
        property var pending: []
        property bool papered: false

        function begin(point) {
            paper.pending = paper.pending.concat([{ colour: root.ink, points: [point] }])
        }
        function extend(point) {
            if (paper.pending.length === 0) {
                paper.begin(point)
                return
            }
            paper.pending[paper.pending.length - 1].points.push(point)
        }

        onPaint: {
            const ctx = getContext("2d")
            if (!paper.papered && paper.width > 0 && paper.height > 0) {
                ctx.fillStyle = "#ffffff"
                ctx.fillRect(0, 0, paper.width, paper.height)
                paper.papered = true
            }
            ctx.lineWidth = root.nib
            ctx.lineCap = "round"
            ctx.lineJoin = "round"
            for (var p = 0; p < paper.pending.length; ++p) {
                const piece = paper.pending[p]
                if (piece.points.length < 2)
                    continue
                ctx.strokeStyle = piece.colour
                ctx.beginPath()
                ctx.moveTo(piece.points[0].x, piece.points[0].y)
                for (var i = 1; i < piece.points.length; ++i)
                    ctx.lineTo(piece.points[i].x, piece.points[i].y)
                ctx.stroke()
            }
            // The last point of the last piece stays, so the next move joins the line it
            // ends rather than starting a new one a frame further on.
            const last = paper.pending.length > 0 ? paper.pending[paper.pending.length - 1] : null
            paper.pending = (last && last.points.length > 0)
                ? [{ colour: last.colour, points: [last.points[last.points.length - 1]] }]
                : []
        }

        MouseArea {
            anchors.fill: parent
            onPressed: (mouse) => {
                paper.begin(Qt.point(mouse.x, mouse.y))
                paper.requestPaint()
            }
            onPositionChanged: (mouse) => {
                if (!pressed)
                    return
                paper.extend(Qt.point(mouse.x, mouse.y))
                paper.requestPaint()
            }
            onReleased: (mouse) => {
                paper.extend(Qt.point(mouse.x, mouse.y))
                paper.requestPaint()
                root.strokes += 1
                if (root.strokes % 25 === 0)
                    console.log("paint: strokes", root.strokes)
            }
        }
    }

    // The palette, on its own strip below the paper: six squares, each a colour to pick up.
    // It is not on the paper, so a line drawn near the bottom of the picture cannot change
    // the colour it is being drawn in.
    Rectangle {
        id: tray
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 56
        color: "#f2f0ec"
        border.width: 1
        border.color: "#d8d4cc"
        onYChanged: root.report()

        Row {
            id: palette
            x: 12
            y: 12
            spacing: 8
            Repeater {
                model: root.swatches
                Rectangle {
                    required property string modelData
                    required property int index
                    width: 32
                    height: 32
                    radius: 4
                    color: modelData
                    border.width: root.ink === modelData ? 3 : 1
                    border.color: root.ink === modelData ? "#1f6feb" : "#8a8a8a"
                    Accessible.role: Accessible.Button
                    Accessible.name: "color " + (index + 1)
                    MouseArea {
                        anchors.fill: parent
                        onPressed: {
                            root.ink = parent.modelData
                            console.log("paint: ink", root.ink)
                        }
                    }
                }
            }
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
