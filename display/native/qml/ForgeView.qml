import QtQuick
import QtQuick.Controls

// FORGE: the machine's applications as a desktop. Large icons, their names, and nothing
// else to operate: Archie opens and uses them; a person can still click one open.
// Archie's own come first, then everything installed, then, faintly, the well-known
// tools that are missing.
Item {
    id: forge

    property string notice: ""
    readonly property var apps: appCatalog.apps.filter(function (a) { return a.section !== "suggested" })
    readonly property var missing: appCatalog.apps.filter(function (a) { return a.section === "suggested" })

    Connections {
        target: appCatalog
        function onLaunched(id, ok, message) {
            forge.notice = ok ? "" : message
            noticeTimer.restart()
        }
    }
    Timer { id: noticeTimer; interval: 5000; onTriggered: forge.notice = "" }

    Flickable {
        id: desk
        anchors.fill: parent
        anchors.topMargin: 92
        anchors.leftMargin: 56
        anchors.rightMargin: 56
        contentHeight: column.implicitHeight + 60
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {
            policy: ScrollBar.AsNeeded
            contentItem: Rectangle { implicitWidth: 4; radius: 2; color: "#33ffffff" }
        }

        Column {
            id: column
            width: desk.width
            spacing: 34

            Grid {
                id: grid
                width: parent.width
                columns: Math.max(1, Math.floor(width / 132))
                columnSpacing: (width - columns * 112) / Math.max(1, columns - 1)
                rowSpacing: 26
                Repeater {
                    model: forge.apps
                    delegate: AppIcon { required property var modelData; app: modelData }
                }
            }

            Column {
                visible: forge.missing.length > 0
                width: parent.width
                spacing: 16
                Text { text: "Para instalar"; color: "#5d6574"; font.pixelSize: 12; font.weight: Font.Medium; font.letterSpacing: 0.4 }
                Grid {
                    width: parent.width
                    columns: grid.columns
                    columnSpacing: grid.columnSpacing
                    rowSpacing: 26
                    Repeater {
                        model: forge.missing
                        delegate: AppIcon { required property var modelData; app: modelData; faint: true }
                    }
                }
            }
        }
    }

    Text {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 22
        visible: forge.notice.length > 0
        text: forge.notice
        color: "#c9ced8"
        font.pixelSize: 12
    }

    component AppIcon: Item {
        id: icon
        property var app: ({})
        property bool faint: false
        readonly property bool ready: !!app.available && appCatalog.canLaunch
        width: 112
        height: 128
        opacity: faint ? 0.32 : (app.available ? 1.0 : 0.45)

        Rectangle {
            id: tile
            anchors.horizontalCenter: parent.horizontalCenter
            width: 76; height: 76; radius: 20
            scale: area.pressed ? 0.94 : (area.containsMouse ? 1.05 : 1.0)
            Behavior on scale { NumberAnimation { duration: 140; easing.type: Easing.OutCubic } }
            gradient: Gradient {
                GradientStop { position: 0.0; color: icon.app.section === "archie" ? "#2a2433" : "#1b2029" }
                GradientStop { position: 1.0; color: icon.app.section === "archie" ? "#171320" : "#12161d" }
            }
            border.color: area.containsMouse ? "#40ffffff" : "#14ffffff"
            Image {
                id: image
                anchors.centerIn: parent
                width: 50; height: 50
                source: icon.app.icon || ""
                sourceSize: Qt.size(96, 96)
                fillMode: Image.PreserveAspectFit
                visible: status === Image.Ready
                asynchronous: true
                smooth: true
            }
            Text {
                anchors.centerIn: parent
                visible: image.status !== Image.Ready
                text: icon.app.glyph || "·"
                color: icon.app.section === "archie" ? "#d9c3ff" : "#dfe4ec"
                font.pixelSize: 30
                font.weight: Font.Light
            }
        }
        // Running: a small light under the icon, as a dock shows it.
        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: tile.bottom
            anchors.topMargin: 6
            width: 5; height: 5; radius: 2.5
            visible: icon.app.running > 0
            color: "#9fe6c9"
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: tile.bottom
            anchors.topMargin: 16
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: icon.app.name || ""
            color: "#dfe4ec"
            font.pixelSize: 12
            elide: Text.ElideRight
            maximumLineCount: 2
            wrapMode: Text.Wrap
        }
        MouseArea {
            id: area
            anchors.fill: parent
            hoverEnabled: true
            onClicked: if (!icon.faint) appCatalog.launch(icon.app.id)
        }
        ToolTip.visible: area.containsMouse && (!!icon.app.comment || !!icon.app.missing)
        ToolTip.delay: 600
        ToolTip.text: icon.app.available ? (icon.app.comment || "") : (icon.app.missing || "")
    }
}
