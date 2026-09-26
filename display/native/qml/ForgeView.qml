import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// FORGE: the applications this machine has. Archie's own first, then everything
// installed, then the well-known tools that are missing and the package that would bring
// them. Each card says whether it can start, whether it is running and whether Archie may
// operate it through the Stage.
Rectangle {
    id: forge
    color: "#0b1117"

    readonly property color ink: "#dce5e8"
    readonly property color muted: "#81949e"
    readonly property color accent: "#f0be7e"
    readonly property color line: "#25323b"
    property string filter: "Todas"
    property string query: ""
    property string notice: ""

    readonly property var groups: {
        const seen = ["Todas"]
        const apps = appCatalog.apps
        for (let i = 0; i < apps.length; ++i)
            if (seen.indexOf(apps[i].group) < 0) seen.push(apps[i].group)
        return seen
    }
    function visibleIn(section) {
        const needle = forge.query.trim().toLowerCase()
        return appCatalog.apps.filter(function (app) {
            if (app.section !== section) return false
            if (forge.filter !== "Todas" && app.group !== forge.filter) return false
            if (needle.length === 0) return true
            return (app.name + " " + app.comment + " " + app.id).toLowerCase().indexOf(needle) >= 0
        })
    }
    readonly property int runningCount: appCatalog.apps.filter(function (a) { return a.running > 0 }).length
    readonly property int installedCount: appCatalog.apps.filter(function (a) { return a.section !== "suggested" }).length

    Connections {
        target: appCatalog
        function onLaunched(id, ok, message) {
            forge.notice = (ok ? "Abriendo " : "No se abrió " ) + id + " — " + message
            noticeTimer.restart()
        }
    }
    Timer { id: noticeTimer; interval: 6000; onTriggered: forge.notice = "" }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 22
        anchors.topMargin: 70
        spacing: 14

        RowLayout {
            Layout.fillWidth: true
            spacing: 16
            Column {
                spacing: 3
                Text { text: "FORGE · APLICACIONES"; color: forge.muted; font.pixelSize: 9; font.bold: true; font.letterSpacing: 1.8 }
                Text { text: forge.installedCount + " instaladas · " + forge.runningCount + " en ejecución"; color: forge.ink; font.pixelSize: 17; font.bold: true }
                Text { text: appCatalog.launchNote; color: forge.muted; font.pixelSize: 10 }
            }
            Item { Layout.fillWidth: true }
            Rectangle {
                Layout.preferredWidth: 280; Layout.preferredHeight: 36; radius: 5
                color: "#0c141b"; border.color: search.activeFocus ? "#d6a873" : "#34444c"
                TextField {
                    id: search
                    anchors.fill: parent; anchors.leftMargin: 8; anchors.rightMargin: 8
                    placeholderText: "Buscar aplicación"
                    placeholderTextColor: "#6c838e"
                    color: forge.ink; font.pixelSize: 11
                    background: Item {}
                    onTextChanged: forge.query = text
                }
            }
            Rectangle {
                Layout.preferredWidth: 36; Layout.preferredHeight: 36; radius: 5
                color: refreshArea.containsMouse ? "#1c2a33" : "#18232c"; border.color: "#35444c"
                Text { anchors.centerIn: parent; text: "⟳"; color: forge.ink; font.pixelSize: 14 }
                MouseArea { id: refreshArea; anchors.fill: parent; hoverEnabled: true; onClicked: appCatalog.refresh() }
            }
        }

        Flow {
            Layout.fillWidth: true
            spacing: 6
            Repeater {
                model: forge.groups
                delegate: Rectangle {
                    required property string modelData
                    width: chipText.implicitWidth + 22; height: 26; radius: 13
                    color: forge.filter === modelData ? "#2a2217" : "#18232c"
                    border.color: forge.filter === modelData ? "#d6a873" : "#2c3b44"
                    Text { id: chipText; anchors.centerIn: parent; text: modelData; color: forge.filter === modelData ? forge.accent : forge.ink; font.pixelSize: 10 }
                    MouseArea { anchors.fill: parent; onClicked: forge.filter = modelData }
                }
            }
        }

        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentWidth: availableWidth

            Column {
                width: forge.width - 44
                spacing: 18
                Shelf { title: "ARCHIE"; note: "diseñadas para Archie"; apps: forge.visibleIn("archie") }
                Shelf { title: "INSTALADAS"; note: "todo lo que el sistema declara"; apps: forge.visibleIn("installed") }
                Shelf { title: "SUGERIDAS"; note: "no instaladas · instalar requiere root"; apps: forge.visibleIn("suggested") }
            }
        }
    }

    Rectangle {
        visible: forge.notice.length > 0
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom; anchors.bottomMargin: 18
        width: Math.min(parent.width - 60, noticeText.implicitWidth + 28); height: 32; radius: 5
        color: "#e6101820"; border.color: "#d6a873"
        Text { id: noticeText; anchors.centerIn: parent; width: parent.width - 20; elide: Text.ElideRight; horizontalAlignment: Text.AlignHCenter; text: forge.notice; color: forge.ink; font.pixelSize: 10 }
    }

    component Shelf: Column {
        id: shelf
        property string title: ""
        property string note: ""
        property var apps: []
        visible: apps.length > 0
        width: parent ? parent.width : 800
        spacing: 10
        Row {
            spacing: 10
            Text { text: shelf.title; color: "#91a4ae"; font.pixelSize: 10; font.bold: true; font.letterSpacing: 1.8 }
            Text { text: shelf.apps.length + " · " + shelf.note; color: "#6c838e"; font.pixelSize: 9 }
        }
        Flow {
            width: shelf.width
            spacing: 10
            Repeater {
                model: shelf.apps
                delegate: AppCard { required property var modelData; app: modelData }
            }
        }
    }

    component AppCard: Rectangle {
        id: card
        property var app: ({})
        width: 262; height: 124; radius: 7
        color: cardArea.containsMouse ? "#16222b" : "#121c24"
        border.color: app.running > 0 ? "#3f7a5c" : (app.section === "archie" ? "#5a4631" : "#25323b")
        MouseArea { id: cardArea; anchors.fill: parent; hoverEnabled: true; onDoubleClicked: if (card.app.available) appCatalog.launch(card.app.id) }

        Rectangle {
            id: iconBox
            x: 14; y: 14; width: 46; height: 46; radius: 10
            color: card.app.section === "archie" ? "#2a2217" : "#18232c"
            border.color: card.app.section === "archie" ? "#d6a873" : "#35444c"
            Image {
                id: icon
                anchors.fill: parent; anchors.margins: 6
                source: card.app.icon || ""
                sourceSize: Qt.size(64, 64)
                fillMode: Image.PreserveAspectFit
                visible: status === Image.Ready
                asynchronous: true
            }
            Text {
                anchors.centerIn: parent
                visible: icon.status !== Image.Ready
                text: card.app.glyph || "?"
                color: card.app.section === "archie" ? "#f0be7e" : "#dce5e8"
                font.pixelSize: 20; font.bold: true
            }
        }
        Column {
            anchors.left: iconBox.right; anchors.leftMargin: 12
            anchors.right: parent.right; anchors.rightMargin: 12
            y: 14
            spacing: 3
            Text { width: parent.width; text: card.app.name; color: "#dce5e8"; font.pixelSize: 13; font.bold: true; elide: Text.ElideRight }
            Text { width: parent.width; text: card.app.comment || card.app.group; color: "#81949e"; font.pixelSize: 10; elide: Text.ElideRight; maximumLineCount: 2; wrapMode: Text.Wrap }
        }
        Row {
            x: 14; anchors.bottom: parent.bottom; anchors.bottomMargin: 12
            spacing: 5
            Badge { visible: card.app.running > 0; text: card.app.running > 1 ? "EN EJECUCIÓN ×" + card.app.running : "EN EJECUCIÓN"; tint: "#47d18b" }
            Badge { visible: card.app.operable; text: "ARCHIE LA OPERA"; tint: "#f0be7e" }
            Badge { visible: !card.app.available; text: card.app.section === "suggested" ? "NO INSTALADA" : "NO DISPONIBLE"; tint: "#81949e" }
        }
        Rectangle {
            id: openButton
            anchors.right: parent.right; anchors.rightMargin: 12
            anchors.bottom: parent.bottom; anchors.bottomMargin: 10
            width: 66; height: 26; radius: 5
            readonly property bool can: card.app.available && appCatalog.canLaunch
            opacity: can ? 1.0 : 0.4
            color: openArea.containsMouse && can ? "#3a2c1c" : "#2a2217"; border.color: "#d6a873"
            Text { anchors.centerIn: parent; text: "Abrir"; color: "#f0be7e"; font.pixelSize: 10; font.bold: true }
            MouseArea { id: openArea; anchors.fill: parent; hoverEnabled: true; onClicked: appCatalog.launch(card.app.id) }
            ToolTip.visible: openArea.containsMouse && !openButton.can
            ToolTip.text: card.app.missing || appCatalog.launchNote
        }
    }

    component Badge: Rectangle {
        id: badge
        property string text: ""
        property color tint: "white"
        width: badgeText.implicitWidth + 10; height: 16; radius: 3
        color: "transparent"; border.color: tint
        Text { id: badgeText; anchors.centerIn: parent; text: badge.text; color: badge.tint; font.pixelSize: 7; font.bold: true; font.letterSpacing: 0.8 }
    }
}
