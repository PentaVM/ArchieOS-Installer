import QtQuick

// Forge: the applications on this machine, opened in this session through the native app
// catalog (the same one the Observatory's Forge uses), and the bridges' observed jobs and module
// manifests. Opening an application is a launch in the person's own session; Forge never runs a
// tool, approves an operation or claims a permission.
AppWindow {
    id: fg
    title:"Forge"
    property var bridge:null
    property string st:Demo.state
    property int section:0
    property string note:""
    readonly property bool hasCatalog: typeof appCatalog !== "undefined" && appCatalog !== null
    readonly property var allApps: hasCatalog ? appCatalog.apps : []
    readonly property var apps: allApps.filter(function(a) { return a.section !== "suggested" })
    readonly property var suggested: allApps.filter(function(a) { return a.section === "suggested" })
    readonly property var records: bridge && bridge.jobs && bridge.jobs.jobs ? bridge.jobs.jobs : []
    readonly property var catalog: bridge && bridge.catalog ? bridge.catalog : null
    readonly property var modules: catalog && catalog.modules ? catalog.modules.filter(function(m) { return m.id.indexOf("forge")>=0 }) : []
    // The catalog counts running windows only while someone looks at it.
    Binding { when: fg.hasCatalog; target: fg.hasCatalog ? appCatalog : null; property: "active"; value: fg.shown && fg.section === 0 }
    Connections {
        target: fg.hasCatalog ? appCatalog : null
        function onLaunched(id, ok, message) {
            var app = fg.allApps.filter(function(a) { return a.id === id })[0]
            fg.note = (app ? app.name : id) + (ok ? ": abierta" : ": " + message)
            noteTimer.restart()
        }
    }
    Timer { id: noteTimer; interval: 4000; onTriggered: fg.note = "" }
    function launch(id) { return fg.hasCatalog && appCatalog.launch(id) }

    trailing:[SourceChip { bridge:fg.bridge; projection:fg.catalog }]
    sidebar:[Eyebrow { text:"Forge" },
        SideRow { objectName:"forge.section.apps"; icon:"windows"; text:"Aplicaciones"; on:fg.section===0; onClicked:fg.section=0 },
        SideRow { objectName:"forge.section.jobs"; icon:"plug"; text:"Trabajos observados"; on:fg.section===1; onClicked:fg.section=1 }]

    // ---- applications ----
    Column { visible:fg.section===0; width:parent.width; spacing:12
        Eyebrow { text:"Aplicaciones de este equipo" }
        NoteBox { visible:!fg.hasCatalog; icon:"unknown"; text:"Este host no tiene el catálogo nativo de aplicaciones." }
        NoteBox { visible:fg.hasCatalog && !appCatalog.canLaunch; icon:"unknown"; text:fg.hasCatalog ? appCatalog.launchNote : "" }
        NoteBox { visible:fg.note!==""; icon:"done"; text:fg.note }
        Flow { width:parent.width; spacing:10
            Repeater { model:fg.apps
                delegate:Rectangle { objectName:"forge.app." + modelData.id; width:112; height:118; radius:12
                    readonly property bool ready: !!modelData.available && fg.hasCatalog && appCatalog.canLaunch
                    color:ama.containsMouse && ready ? Theme.hover : "transparent"; opacity: modelData.available ? 1 : 0.5
                    Rectangle { id:tile; anchors.horizontalCenter:parent.horizontalCenter; y:8; width:60; height:60; radius:16
                        color: modelData.section === "archie" ? Theme.tint(Theme.acc, 0.16) : Theme.raised; border.width:1; border.color:Theme.hair
                        Image { id:img; anchors.centerIn:parent; width:40; height:40; source:modelData.icon || ""; sourceSize:Qt.size(80,80)
                            fillMode:Image.PreserveAspectFit; visible:status===Image.Ready; asynchronous:true }
                        Text { anchors.centerIn:parent; visible:img.status!==Image.Ready; text:modelData.glyph || "·"; color:Theme.t1; font.pixelSize:24; font.family:Theme.font } }
                    Rectangle { visible:modelData.running > 0; anchors.horizontalCenter:parent.horizontalCenter; anchors.top:tile.bottom; anchors.topMargin:4
                        width:5; height:5; radius:2.5; color:Theme.ok }
                    Text { anchors.horizontalCenter:parent.horizontalCenter; y:tile.y + tile.height + 12; width:parent.width - 8
                        horizontalAlignment:Text.AlignHCenter; text:modelData.name || ""; elide:Text.ElideRight; maximumLineCount:2; wrapMode:Text.Wrap
                        color:Theme.t1; font.family:Theme.font; font.pixelSize:12 }
                    MouseArea { id:ama; anchors.fill:parent; hoverEnabled:true; enabled:parent.ready; onClicked:fg.launch(modelData.id) } } } }
        Eyebrow { visible:fg.suggested.length>0; text:"Para instalar" }
        Repeater { model:fg.suggested
            delegate:ListRow { glyph:"plus"; title:modelData.name; sub:(modelData.comment || "") + " · " + (modelData.missing || "")
                Chip { small:true; text:"no instalada"; tone:"unk" } } }
    }

    // ---- jobs ----
    Column { visible:fg.section===1; width:parent.width; spacing:12
        Eyebrow { text:"Trabajos de los puentes existentes" }
        NoteBox { visible:!!(fg.bridge && fg.bridge.silent); icon:"stale"; text:"Puente sin tramas nuevas: estos trabajos son el último estado observado." }
        VoidBox { visible:fg.records.length===0; icon:"plug"; title:"Sin trabajos observados"
            text:"El bridge lee status.json de los workspaces configurados. Un listado vacío no demuestra disponibilidad de un worker." }
        Repeater { model:fg.records
            delegate:ListRow { glyph:"activity"; title:modelData.bridge + " · " + modelData.kind
                sub:modelData.workspace + " · " + modelData.id + " · " + modelData.done + (modelData.total>0 ? " / " + modelData.total : " iteraciones")
                Chip { small:true; text:String(modelData.state); tone:modelData.state==="failed" ? "bad" : "neutral" }
            }
        }
        Eyebrow { text:"Manifiesto de Forge" }
        Repeater { model:fg.modules
            delegate:ListRow { glyph:"plug"; title:modelData.id; sub:modelData.explanation
                Chip { small:true; text:modelData.state; tone:"neutral" }
            }
        }
        NoteBox { text:"Enviar, cancelar y aprobar trabajos requiere un canal de autoridad de Core. Este visor no ejecuta herramientas ni afirma permisos concedidos." }
    }
}
