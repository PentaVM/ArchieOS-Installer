import QtQuick

AppWindow {
    id:sw; title:"Ajustes"
    property var bridge:null
    property string st:Demo.state
    property int section:0
    readonly property var catalog:bridge && bridge.catalog ? bridge.catalog : null
    readonly property var inspection:bridge && bridge.inspector ? bridge.inspector : null
    readonly property var providers:inspection && inspection.providers ? inspection.providers : []
    readonly property var models:catalog && catalog.models ? catalog.models : []
    readonly property var modules:catalog && catalog.modules ? catalog.modules : []
    trailing:[SourceChip { bridge:sw.bridge; projection:sw.section===0 ? sw.inspection : sw.catalog }]
    sidebar:[
        SideRow { icon:"key"; text:"Proveedores"; on:sw.section===0; onClicked:sw.section=0 },
        SideRow { icon:"cpu"; text:"Modelos"; on:sw.section===1; onClicked:sw.section=1 },
        SideRow { icon:"lock"; text:"Autoridad"; on:sw.section===2; onClicked:sw.section=2 },
        SideRow { icon:"activity"; text:"Diagnóstico"; on:sw.section===3; onClicked:sw.section=3 },
        SideRow { icon:"dkOrb"; text:"Orbe nativo"; on:sw.section===4; onClicked:sw.section=4 },
        SideRow { icon:"control"; text:"Apariencia"; on:sw.section===5; onClicked:sw.section=5 }
    ]
    Column { visible:sw.section===0; width:parent.width; spacing:8
        Eyebrow { text:"Proveedores declarados por Core" }
        VoidBox { visible:sw.providers.length===0; icon:"key"; title:"Sin proveedores en la proyección"
            text:sw.inspection && sw.inspection.reason ? sw.inspection.reason : "El visor no solicita ni muestra valores de claves." }
        Repeater { model:sw.providers
            delegate:ListRow { glyph:"key"; title:modelData.title || modelData.id; sub:modelData.id + " · " + modelData.capability
                Chip { small:true; text:modelData.status; tone:modelData.status==="available" ? "neutral" : "unk" }
            }
        }
    }
    Column { visible:sw.section===1; width:parent.width; spacing:8
        Eyebrow { text:"Modelos registrados" }
        VoidBox { visible:sw.models.length===0; icon:"cpu"; title:"Sin modelos en la proyección"; text:"No se deduce si hay un modelo cargado a partir de un catálogo vacío." }
        Repeater { model:sw.models
            delegate:ListRow { glyph:"cpu"; title:modelData.id
                sub:(modelData.locality || "localidad no declarada") + " · privacidad " + (modelData.max_privacy_grade || "no declarada")
                Chip { small:true; text:modelData.availability || "sin lectura"; tone:"neutral" }
            }
        }
    }
    NoteBox { visible:sw.section===2; icon:"lock"
        text:"Core y Policy conservan la autoridad. Esta proyección no expone leases ni ofrece aprobar operaciones: el canal gráfico de revisión todavía no está conectado." }
    Column { visible:sw.section===3; width:parent.width; spacing:8
        Eyebrow { text:"Módulos y diagnóstico real" }
        Repeater { model:sw.modules
            delegate:ListRow { glyph:"activity"; title:modelData.id; sub:modelData.explanation
                Chip { small:true; text:modelData.state; tone:"neutral" }
            }
        }
        ResourceGauges { bridge:sw.bridge; width:parent.width }
    }
    Column { visible:sw.section===4; width:parent.width; spacing:8
        OrbOpt { label:"Mostrar en escritorio"; options:["Sí","No"]; index:Demo.orbShow ? 0 : 1; onPicked:function(i) { Demo.orbShow=(i===0) } }
        OrbOpt { label:"Representación"; options:["Esfera","Orbe","Campo","Neural","Crown","Atlas","Dense"]; index:typeof lumen!=="undefined" ? ["sphere","orb","field","neural","crown","atlas","dense"].indexOf(lumen.profile) : Demo.orbProfile; onPicked:function(i) { Demo.orbProfile=i } }
        OrbOpt { label:"Nodos visibles (máximo nativo)"; options:["60","120","220","360","420"]; index:Demo.orbNodesI; onPicked:function(i) { Demo.orbNodesI=i } }
        OrbOpt { label:"Color de la esfera"; options:["Original","Holograma frío","Vivo"]; index:Demo.orbGradeI; onPicked:function(i) { Demo.orbGradeI=i } }
        OrbOpt { label:"Polvo de luz"; options:["No","Sí"]; index:Demo.orbDust ? 1 : 0; onPicked:function(i) { Demo.orbDust=i===1 } }
        OrbOpt { label:"Brillo"; options:["Bajo","Normal","Alto"]; index:Demo.orbBrightI; onPicked:function(i) { Demo.orbBrightI=i } }
        OrbOpt { label:"Actividad visual"; options:["Apagada","Baja","Media","Alta"]; index:Demo.orbActI; onPicked:function(i) { Demo.orbActI=i } }
        OrbOpt { label:"Giro automático"; options:["Sí","No"]; index:Demo.orbSpin ? 0 : 1; onPicked:function(i) { Demo.orbSpin=(i===0) } }
        NoteBox { text:"Renderer nativo Lumenfield, geometría y cinemáticas existentes. El presupuesto del OS y movimiento reducido limitan efectos; una opción visual no cambia modelos, inferencia ni autoridad." }
    }
    Column { visible:sw.section===5; width:parent.width; spacing:8
        OrbOpt { objectName:"themeMode"; label:"Tema"; options:["Oscuro","Claro · beta"]; index:Theme.light ? 1 : 0; onPicked:function(i) { Theme.colorMode=i===1 ? "light" : "dark" } }
        OrbOpt { label:"Colores"; options:["Vívidos","Originales"]; index:Theme.vivid ? 0 : 1; onPicked:function(i) { Theme.vivid=(i===0) } }
        OrbOpt { label:"Perfil de movimiento"; options:["Completo","Equilibrado","Tranquilo"]; index:["full","balanced","quiet"].indexOf(Theme.motion); onPicked:function(i) { Theme.motion=["full","balanced","quiet"][i] } }
        OrbOpt { label:"Superficies opacas"; options:["No","Sí"]; index:Theme.opaque ? 1 : 0; onPicked:function(i) { Theme.opaque=i===1 } }
        OrbOpt { label:"Movimiento reducido"; options:["Sí","No"]; index:Theme.reducedMotion ? 0 : 1; onPicked:function(i) { Theme.reducedMotion=(i===0) } }
        NoteBox { text:"Preferencias locales de presentación. No modifican la configuración de Core." }
    }
}
