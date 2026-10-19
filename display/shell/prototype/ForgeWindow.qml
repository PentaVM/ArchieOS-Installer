import QtQuick

// Existing bridge jobs and module manifests, never an invented pipeline executor.
AppWindow {
    id: fg
    title:"Forge"
    property var bridge:null
    property string st:Demo.state
    readonly property var records: bridge && bridge.jobs && bridge.jobs.jobs ? bridge.jobs.jobs : []
    readonly property var catalog: bridge && bridge.catalog ? bridge.catalog : null
    readonly property var modules: catalog && catalog.modules ? catalog.modules.filter(function(m) { return m.id.indexOf("forge")>=0 }) : []
    trailing:[SourceChip { bridge:fg.bridge; projection:fg.catalog }]
    sidebar:[Eyebrow { text:"Integraciones" }, SideRow { icon:"plug"; text:"Trabajos observados"; on:true }]
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
    NoteBox { text:"Enviar, cancelar y aprobar requiere un canal de autoridad de Core. Este visor no ejecuta herramientas ni afirma permisos concedidos." }
}
