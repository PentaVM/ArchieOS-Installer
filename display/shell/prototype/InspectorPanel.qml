import QtQuick
import QtQuick.Controls

// Core supplies minimized trajectory DTOs; the Orb supplies observed workflow signals.
Panel {
    id: ip
    kind: "panel"
    width: 540
    height: Math.min(900, parent ? Math.max(240,parent.height-y-24) : 900, content.implicitHeight + 40)
    property var bridge: null
    property string st: Demo.state
    readonly property var inspection: bridge && bridge.inspector ? bridge.inspector : null
    readonly property var flow: bridge && bridge.workflows ? bridge.workflows : null
    readonly property var trajectories: inspection && inspection.cortex_trajectories ? inspection.cortex_trajectories : []
    readonly property var observations: flow && flow.workflows ? flow.workflows : []
    Flickable {
        objectName:"projectionScroll"; anchors.fill:parent
        contentWidth:width; contentHeight:content.implicitHeight+40; clip:true
        boundsBehavior:Flickable.StopAtBounds
        ScrollBar.vertical:ScrollBar { }
    Column {
        id: content; x:20; y:20; width:parent.width-40; spacing:12
        Row { spacing:12
            Text { text:"IRIS · Cortex"; color:Theme.t1; font.family:Theme.disp; font.pixelSize:22 }
            SourceChip { bridge:ip.bridge; projection:ip.inspection }
        }
        NoteBox { text:"Proyección de lectura. No expone contenido de contexto, pensamiento interno ni credenciales." }
        Eyebrow { text:"Trayectorias de Cortex" }
        VoidBox { visible:ip.trajectories.length===0; icon:"trace"
            title:ip.inspection && ip.inspection.state==="ready" ? "Sin trayectorias registradas" : "Trayectorias sin lectura"
            text:ip.inspection && ip.inspection.reason ? ip.inspection.reason : "Sólo aparecen registros emitidos por Core." }
        Repeater { model:ip.trajectories
            delegate: ListRow { glyph:"trace"; mono:true; title:String(modelData.trajectory_id)
                sub:String(modelData.correction_kind) + " · modelo " + modelData.model_id
                    + " · contexto " + modelData.selected_context_count + ", omitidos " + modelData.omitted_candidate_count
                Chip { small:true; text:"minimizada"; tone:"neutral" }
            }
        }
        Row { spacing:12; Eyebrow { text:"Señales de IRIS, Cortex, voz y Stage" }
            SourceChip { bridge:ip.bridge; projection:ip.flow }
        }
        VoidBox { visible:ip.observations.length===0; icon:"unknown"; title:"Sin señales recibidas"
            text:"La ausencia de eventos no indica que un componente esté desactivado." }
        WorkflowList { records:ip.observations }
    }
    }
}
