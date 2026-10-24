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
        SideRow { objectName:"settings.section.0"; icon:"key"; text:"Proveedores"; on:sw.section===0; onClicked:sw.section=0 },
        SideRow { objectName:"settings.section.1"; icon:"cpu"; text:"Modelos"; on:sw.section===1; onClicked:sw.section=1 },
        SideRow { objectName:"settings.section.2"; icon:"lock"; text:"Autoridad"; on:sw.section===2; onClicked:sw.section=2 },
        SideRow { objectName:"settings.section.3"; icon:"activity"; text:"Diagnóstico"; on:sw.section===3; onClicked:sw.section=3 },
        SideRow { objectName:"settings.section.4"; icon:"dkOrb"; text:"Orbe nativo"; on:sw.section===4; onClicked:sw.section=4 },
        SideRow { objectName:"settings.section.5"; icon:"control"; text:"Apariencia"; on:sw.section===5; onClicked:sw.section=5 }
    ]
    Column { visible:sw.section===0; width:parent.width; spacing:8
        Eyebrow { text:"Proveedores declarados por Core" }
        VoidBox { visible:sw.providers.length===0; icon:"key"; title:"Sin proveedores en la proyección"
            text:sw.inspection && (sw.inspection.providers_reason || sw.inspection.reason) ? (sw.inspection.providers_reason || sw.inspection.reason) : "El visor no solicita ni muestra valores de claves." }
        Repeater { model:sw.providers
            delegate:ListRow { glyph:"key"; title:modelData.title || modelData.id; sub:modelData.id + " · " + modelData.capability
                Chip { small:true; text:modelData.status; tone:modelData.status==="available" ? "neutral" : "unk" }
            }
        }
    }
    Column { id:localModels; visible:sw.section===1; width:parent.width; spacing:8
        readonly property bool host: typeof shellRuntime !== "undefined" && shellRuntime !== null
        readonly property var list: host ? shellRuntime.localModels : []
        onVisibleChanged: if (visible && host) shellRuntime.localModelsRefresh()
        Eyebrow { text:"Modelo local que responde" }
        VoidBox { visible:localModels.list.length===0; icon:"cpu"; title:"Sin modelos locales en la lista"
            text:!localModels.host ? "Este host no puede hablar con archie-local." : (shellRuntime.localModelNote || "archie-local todavía no respondió.") }
        Repeater { model:localModels.list
            delegate:ListRow { objectName:"settings.localModel." + modelData.name; glyph:"cpu"; title:modelData.label
                sub:modelData.answering ? "Responde ahora en 127.0.0.1:" + modelData.port
                    : modelData.here ? (modelData.chosen ? "Elegido; su servidor no está en marcha" : "En este equipo")
                    : "No está en este equipo: archie-local setup " + modelData.name
                Btn { objectName:"settings.useModel." + modelData.name
                    text:modelData.answering ? "En uso" : shellRuntime.localModelBusy ? "Cargando…" : "Usar"
                    dis:!modelData.here || modelData.answering || shellRuntime.localModelBusy
                    onClicked:shellRuntime.useLocalModel(modelData.name) } }
        }
        NoteBox { visible:localModels.list.length>0; icon:"cpu"
            text:(localModels.host && shellRuntime.localModelNote ? shellRuntime.localModelNote + " " : "")
                + "Uno responde a la vez: la GPU tiene lugar para uno. El 27B responde mejor y tarda unos minutos en cargar; el router de Core pregunta al que esté en marcha, sin reiniciar nada." }
        Eyebrow { text:"Modelos registrados" }
        VoidBox { visible:sw.models.length===0; icon:"cpu"; title:"Sin modelos en la proyección"; text:"No se deduce si hay un modelo cargado a partir de un catálogo vacío." }
        Repeater { model:sw.models
            delegate:ListRow { glyph:"cpu"; title:modelData.id
                sub:(modelData.locality || "localidad no declarada") + " · privacidad " + (modelData.max_privacy_grade || "no declarada")
                Chip { small:true; text:modelData.availability || "sin lectura"; tone:"neutral" }
            }
        }
    }
    Column { id:authority; visible:sw.section===2; width:parent.width; spacing:8
        readonly property bool host: typeof shellRuntime !== "undefined" && shellRuntime !== null
        readonly property string state: host ? shellRuntime.aiPolicy : "unknown"
        onVisibleChanged: if (visible && host) shellRuntime.aiPolicyRefresh()
        Eyebrow { text:"Pausa de emergencia de Core" }
        ListRow { glyph:"pause"; title:authority.state === "paused" ? "La IA está en pausa" : authority.state === "running" ? "La IA está en marcha" : "Estado desconocido"
            sub:!authority.host ? "Este host no puede hablar con archied." : (shellRuntime.aiPolicyNote || "Pausar rechaza toda inferencia y acción hasta reanudar; solo el dueño del nodo puede hacerlo.")
            Btn { objectName:"settings.emergency"; text:authority.state === "paused" ? "Reanudar" : authority.state === "running" ? "Pausar" : "Reintentar"
                dis:!authority.host || shellRuntime.aiPolicyBusy
                onClicked:authority.state === "paused" ? shellRuntime.aiResume() : authority.state === "running" ? shellRuntime.aiPause() : shellRuntime.aiPolicyRefresh() } }
        NoteBox { icon:"lock"
            text:"Core y Policy conservan la autoridad. La pausa es el único control de autoridad de esta pantalla; aprobar o revocar permisos sigue en archie (CLI) y en la voz." }
    }
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
        OrbOpt { label:"Representación"; options:Demo.orbProfiles; index:typeof lumen!=="undefined" ? Demo.orbProfiles.indexOf(lumen.profile) : Demo.orbProfile; onPicked:function(i) { Demo.orbProfile=i } }
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
