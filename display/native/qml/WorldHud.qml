import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "qrc:/archie/shell/prototype" as Shell
import "Exploration.js" as Exploration

// Xavi (ByXavi270 / XaviCottier): shell design; PentaVM: native implementation.
// A reader of daemon projections. There are deliberately no job mutation controls.
Shell.Panel {
    id: hud
    required property var controller
    property bool worldActive: false
    property var seenJobs: []
    property string cameraOwner: "Vista libre"
    property string cameraJob: ""
    property bool cameraAutomatic: false
    property var elevationPlans: []
    property bool inspectorOpen: false
    readonly property var projection: controller.exploration || ({source:"unavailable"})
    readonly property var jobs: projection.jobs || []
    readonly property var providers: projection.providers || []
    readonly property var snapshot: controller.snapshot || ({})
    readonly property var status: snapshot.status || ({})
    readonly property var kernel: controller.kernel || ({})
    readonly property var metricRows: Exploration.metrics(snapshot)
    readonly property var inference: Exploration.inference(snapshot)
    function cedeCamera() { cameraAutomatic=false;cameraOwner="Cámara manual" }
    function applyCamera(job) {
        if(!Exploration.validCamera(job.camera)) return false
        var elevation=Exploration.cameraElevation(job)
        // Zero here is an explicit provisional reference datum, never a measured elevation.
        if(!controller.frameExploration(job.camera.latitude,job.camera.longitude,job.camera.radius_m,
                                        elevation===null ? 0 : elevation,Shell.Theme.spatial)) return false
        if(elevation!==null && elevationPlans.indexOf(job.id)<0) elevationPlans.push(job.id)
        cameraAutomatic=true;cameraOwner=elevation===null ? "AOI · datum provisional" : "AOI · elevación de la fuente";cameraJob=job.id
        return true
    }
    function observeJobs() {
        if(!worldActive || projection.source==="unavailable") return
        var fresh=Exploration.newJobs(jobs,seenJobs)
        if(fresh.length>0) { applyCamera(fresh[fresh.length-1]);return }
        if(!cameraAutomatic || elevationPlans.indexOf(cameraJob)>=0) return
        // One terrain-ready correction is allowed only while this job still owns the camera.
        var job=jobs.find(function(entry) { return entry.id===hud.cameraJob })
        if(job && Exploration.cameraElevation(job)!==null) applyCamera(job)
    }
    Connections { target:hud.controller; function onExplorationChanged() { hud.observeJobs() } }
    // Visibility/focus never restart a consumed plan or change downloads; a first measured elevation may refine automatic framing.
    onWorldActiveChanged: if(worldActive) observeJobs()
    Component.onCompleted: observeJobs()
    ScrollView {
        anchors.fill:parent; anchors.margins:16
        clip:true; contentWidth:availableWidth
        ColumnLayout {
            width:parent.width; spacing:12
            RowLayout {
                Layout.fillWidth:true
                Text { text:"World"; font.family:Shell.Theme.disp; font.pixelSize:20; font.weight:Font.DemiBold; color:Shell.Theme.t1; Layout.fillWidth:true }
                Shell.SourceChip { projection:hud.projection; small:true }
            }
            Label { text:"Exploración · GeoFabric / Aleph"; color:Shell.Theme.t2; font.family:Shell.Theme.font }
            Label { Layout.fillWidth:true; wrapMode:Text.Wrap; text:"La voz y la CLI inician trabajos. Mover la cámara solo cambia esta vista."; color:Shell.Theme.t3; font.family:Shell.Theme.font; font.pixelSize:11 }
            Info { label:"Cámara"; value:hud.cameraOwner }
            Info { label:"Última proyección"; value:Exploration.known(hud.projection.generated_at_ms) ? new Date(hud.projection.generated_at_ms).toLocaleTimeString() : "Sin lectura" }
            Label { visible:!!hud.projection.error; text:hud.projection.error || ""; color:Shell.Theme.bad; font.family:Shell.Theme.font; wrapMode:Text.Wrap; Layout.fillWidth:true }
            Label { visible:hud.jobs.length===0; text:hud.projection.source==="unavailable" ? "Servicio de exploración sin lectura" : "Sin exploraciones publicadas"; color:Shell.Theme.t3; font.family:Shell.Theme.font; wrapMode:Text.Wrap; Layout.fillWidth:true }
            Repeater {
                model:hud.jobs
                delegate:Rectangle {
                    id:jobCard
                    required property var modelData
                    Layout.fillWidth:true; implicitHeight:jobColumn.implicitHeight+20
                    radius:Shell.Theme.rCard; color:Shell.Theme.raised; border.color:Shell.Theme.hair
                    ColumnLayout {
                        id:jobColumn; anchors.fill:parent; anchors.margins:10; spacing:7
                        RowLayout {
                            Layout.fillWidth:true
                            Shell.Chip { text:jobCard.modelData.domain==="work" ? "Trabajo" : jobCard.modelData.domain==="personal" ? "Personal" : "Sin dominio"; domain:jobCard.modelData.domain || ""; small:true }
                            Shell.Chip { text:Exploration.state(jobCard.modelData.state); tone:Exploration.tone(jobCard.modelData.state); small:true }
                        }
                        Text { Layout.fillWidth:true; text:jobCard.modelData.id || "Sin ID"; elide:Text.ElideMiddle; font.family:Shell.Theme.font; font.pixelSize:11; color:Shell.Theme.t2 }
                        Text { Layout.fillWidth:true; text:Exploration.validCamera(jobCard.modelData.camera) ? Number(jobCard.modelData.camera.latitude).toFixed(5)+", "+Number(jobCard.modelData.camera.longitude).toFixed(5)+" · "+jobCard.modelData.camera.radius_m+" m" : "AOI sin lectura"; font.family:Shell.Theme.font; font.pixelSize:11; color:Shell.Theme.t3; wrapMode:Text.Wrap }
                        Info { label:"Teselas"; value:Exploration.count(jobCard.modelData.tiles_done)+" / "+Exploration.count(jobCard.modelData.tiles_total) }
                        Info { label:"Recibido / caché"; value:Exploration.bytes(jobCard.modelData.bytes_received)+" / "+Exploration.count(jobCard.modelData.cache_hits) }
                        Info { label:"Capas solicitadas"; value:jobCard.modelData.request && jobCard.modelData.request.layers ? jobCard.modelData.request.layers.join(", ") : "Sin lectura" }
                        Info { label:"Elevación de cámara"; value:Exploration.cameraElevation(jobCard.modelData)===null ? "Sin lectura · datum provisional" : Exploration.cameraElevation(jobCard.modelData)+" m · fuente" }
                        Info { label:"Muestras de terreno"; value:Exploration.elevationRange(jobCard.modelData) }
                        Info { label:"Artefactos en esta vista"; value:String((jobCard.modelData.artifacts || []).length) }
                        Label { objectName:"projectionTruncationNotice"; visible:jobCard.modelData.projection_truncated===true; text:"Proyección resumida: el plan y los artefactos están limitados en esta vista; los contadores del trabajo conservan sus totales."; color:Shell.Theme.warn; Layout.fillWidth:true; wrapMode:Text.Wrap; font.family:Shell.Theme.font; font.pixelSize:10 }
                        Label { objectName:"omittedBuildingsNotice"; visible:!!jobCard.modelData.preview && Exploration.known(jobCard.modelData.preview.omitted_buildings) && jobCard.modelData.preview.omitted_buildings>0; text:jobCard.modelData.preview ? "Edificios omitidos de la vista previa: "+Exploration.count(jobCard.modelData.preview.omitted_buildings) : ""; color:Shell.Theme.warn; Layout.fillWidth:true; wrapMode:Text.Wrap; font.family:Shell.Theme.font; font.pixelSize:10 }
                        Text { Layout.fillWidth:true; text:!jobCard.modelData.preview ? "Vista de datos pendiente" : "Terreno: muestras reales; edificios: contorno si falta altura. Datum base: terreno cuando existe; referencia del mar si falta. En vista local el terreno reemplaza el mapa global y el océano, conservando elevaciones negativas. Sin textura satelital descargada."; color:Shell.Theme.t3; font.family:Shell.Theme.font; font.pixelSize:10; wrapMode:Text.Wrap }
                        Text { visible:!!jobCard.modelData.error; Layout.fillWidth:true; text:jobCard.modelData.error || ""; font.family:Shell.Theme.font; font.pixelSize:11; color:Shell.Theme.bad; wrapMode:Text.Wrap }
                    }
                }
            }
            Text { text:"Proveedores y cuotas"; color:Shell.Theme.t1; font.family:Shell.Theme.disp; font.pixelSize:14 }
            Repeater {
                model:hud.providers
                delegate:ColumnLayout {
                    id:providerBlock
                    required property var modelData
                    Layout.fillWidth:true; spacing:5
                    Info { label:providerBlock.modelData.id || "Sin proveedor"; value:providerBlock.modelData.state || "Sin lectura" }
                    Info { label:"Solicitudes / recibido"; value:Exploration.count(providerBlock.modelData.requests_sent)+" / "+Exploration.bytes(providerBlock.modelData.bytes_received) }
                    Info { label:"Límite por minuto"; value:Exploration.count(providerBlock.modelData.requests_per_minute) }
                    Info { label:"En vuelo / máximo"; value:Exploration.count(providerBlock.modelData.in_flight)+" / "+Exploration.count(providerBlock.modelData.max_in_flight) }
                    Info { label:"Cuota restante"; value:Exploration.count(providerBlock.modelData.remaining_requests)+" · "+(providerBlock.modelData.quota_source || "unknown") }
                    Info { label:"Reintento"; value:Exploration.known(providerBlock.modelData.retry_at_ms) ? new Date(providerBlock.modelData.retry_at_ms).toLocaleTimeString() : "Sin espera publicada" }
                }
            }
            Button { text:hud.inspectorOpen ? "Cerrar inspector del sistema" : "Inspector del sistema"; Layout.fillWidth:true; font.family:Shell.Theme.font; onClicked:hud.inspectorOpen=!hud.inspectorOpen }
            ColumnLayout {
                visible:hud.inspectorOpen; Layout.fillWidth:true; spacing:7
                Shell.SourceChip { projection:hud.snapshot; small:true }
                Info { label:"Core"; value:hud.snapshot.daemon_reachable===true ? "Accesible" : "Sin respuesta" }
                Info { label:"CPU"; value:hud.status.cpu_model || "Sin lectura" }
                Info { label:"Aceleradores"; value:hud.status.accelerators ? hud.status.accelerators.join(", ") || "Ninguno publicado" : "Sin lectura" }
                Info { label:"Uso de GPU"; value:"Sin fuente medida publicada" }
                Info { label:"RAM disponible"; value:Exploration.bytes(hud.status.memory_available_bytes) }
                Info { label:"RAM total"; value:Exploration.bytes(hud.status.memory_total_bytes) }
                Info { label:"Presión"; value:hud.kernel.pressure_availability && hud.kernel.pressure_availability.state==="available" ? (hud.kernel.verdict || "Sin lectura") : "Sin lectura" }
                Info { label:"Térmico"; value:hud.kernel.thermal_availability && hud.kernel.thermal_availability.state==="available" ? (hud.kernel.thermal_state || "Sin lectura") : "Sin lectura" }
                Info { label:"Nodos / modelos"; value:String((hud.snapshot.nodes || []).length)+" / "+String((hud.snapshot.models || []).length) }
                Repeater {
                    model:hud.snapshot.models || []
                    delegate:Info { id:modelRow; required property var modelData; label:modelRow.modelData.id || "Sin ID"; value:(modelRow.modelData.provider || "Sin proveedor")+" · "+(modelRow.modelData.availability || "Sin lectura") }
                }
                Repeater {
                    model:hud.snapshot.nodes || []
                    delegate:Info { id:nodeRow; required property var modelData; label:nodeRow.modelData.hostname || nodeRow.modelData.node_id || "Sin nodo"; value:(nodeRow.modelData.health || "Sin lectura")+" · "+(nodeRow.modelData.reachable===true ? "Accesible" : "Sin respuesta") }
                }
                Info { label:"TPS efectivos · e2e"; value:hud.inference.tps }
                Info { label:"Última inferencia"; value:hud.inference.observed }
                Info { label:"Tokens de salida"; value:hud.inference.tokens }
                Info { label:"Duración de solicitud"; value:hud.inference.elapsed }
                Text { Layout.fillWidth:true; text:"E2e incluye toda la solicitud; no mide solo la generación del modelo."; font.family:Shell.Theme.font; color:Shell.Theme.t3; font.pixelSize:10; wrapMode:Text.Wrap }
                Repeater { model:hud.metricRows; delegate:Info { id:metricRow; required property var modelData; label:metricRow.modelData.name+" ("+metricRow.modelData.kind+")"; value:metricRow.modelData.value } }
            }
            Text { Layout.fillWidth:true; text:"Diseño: Xavi · Implementación nativa: PentaVM"; color:Shell.Theme.t3; font.family:Shell.Theme.font; font.pixelSize:10; wrapMode:Text.Wrap }
        }
    }
    component Info: RowLayout {
        id:infoRow
        property string label
        property string value
        Layout.fillWidth:true; spacing:8
        Text { text:infoRow.label; Layout.fillWidth:true; Layout.preferredWidth:1; wrapMode:Text.Wrap; color:Shell.Theme.t3; font.family:Shell.Theme.font; font.pixelSize:11 }
        Text { text:infoRow.value; Layout.fillWidth:true; Layout.preferredWidth:1; horizontalAlignment:Text.AlignRight; wrapMode:Text.Wrap; color:Shell.Theme.t1; font.family:Shell.Theme.font; font.pixelSize:11 }
    }
}
