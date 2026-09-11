import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtMultimedia

ApplicationWindow {
    id: root
    visible: true; width: 1320; height: 900; minimumWidth: 1000; minimumHeight: 740
    title: "ArchieOS · Tu próximo comienzo"
    color: "#080d13"
    property color ink: "#edf4ef"
    property color dim: "#91a4ab"
    property color accent: "#b9f2bd"
    property int page: 0
    property var steps: ["Tu nodo", "Owner e identidad", "Ubicación", "Almacenamiento", "Mapas sin conexión", "Percepción", "Tu celular", "Revisar y preparar"]
    property string ownerName: ""
    property string nickname: ""
    property string addressAs: ""
    property string aiAlias: "ARCHIE"
    property string hostname: "archie-node"
    property string username: ""
    property bool useLocation: false
    property string latitude: ""
    property string longitude: ""
    property string locationSource: "operator"
    property bool useMap: false
    property string regionName: ""
    property string regionUrl: ""
    property string mapMirrors: ""
    property string mapPartition: ""
    property string destination: "/var/lib/archie/maps"
    property bool phoneProtection: false
    property bool cameraConsent: false
    property bool microphoneConsent: false
    property bool microphoneRunning: false
    Component.onCompleted: {
        const p=wizard.loadDraft()
        if(p.version!==1)return
        root.hostname=p.hostname || "archie-node";root.username=p.username || ""
        root.ownerName=p.owner.name || "";root.nickname=p.owner.nickname || ""
        root.addressAs=p.owner.address_as || "";root.aiAlias=p.owner.ai_alias || "ARCHIE"
        if(p.location){root.latitude=String(p.location.latitude);root.longitude=String(p.location.longitude);root.locationSource=p.location.source || "operator"}
        if(p.map){root.useMap=true;root.regionName=p.map.region || "";root.regionUrl=p.map.url || "";root.destination=p.map.destination || root.destination;root.mapPartition=p.map.partition || "";root.mapMirrors=(p.map.mirrors || []).join(", ")}
        root.phoneProtection=Boolean(p.perception && p.perception.require_phone_approval)
    }
    onPageChanged: { wizard.cancelApprovals(); camera.active=false; microphoneRunning=false; wizard.testMicrophone(false) }
    function profile() {
        return {version:1, hostname:hostname, username:username,
            owner:{name:ownerName,nickname:nickname,address_as:addressAs,ai_alias:aiAlias},
            location:useLocation ? {latitude:latitude.trim()==="" ? null : Number(latitude),longitude:longitude.trim()==="" ? null : Number(longitude),source:locationSource,accuracy_m:null,observed_at_ms:Date.now()} : null,
            perception:{camera:cameraConsent,microphone:microphoneConsent,face_enrolment:false,voice_enrolment:false,hand_enrolment:false,require_phone_approval:phoneProtection},
            map:useMap ? {region:regionName,url:regionUrl,destination:destination,partition:mapPartition || null,mirrors:mapMirrors.split(",").map(v=>v.trim()).filter(v=>v.length>0)} : null}
    }
    component Note: Text {
        color: root.dim; font.pixelSize: 14; wrapMode: Text.WordWrap; lineHeight: 1.3
        Layout.fillWidth: true
    }
    component Heading: Text {
        color: root.ink; font.pixelSize: 22; font.weight: Font.DemiBold; Layout.fillWidth: true
    }
    component Field: ColumnLayout {
        property string label
        property string hint: ""
        property alias value: input.text
        signal edited(string value)
        spacing: 9; Layout.fillWidth: true
        Text { text: label; color: root.dim; font.pixelSize: 12; font.letterSpacing: 0.4 }
        TextField {
            id: input; Layout.fillWidth: true; implicitHeight: 48; color: root.ink; font.pixelSize: 16
            Accessible.name: label
            placeholderText: hint; placeholderTextColor: "#576a74"; selectByMouse: true
            leftPadding: 15; onTextEdited: edited(text)
            background: Rectangle { color: "#0c151e"; radius: 9; border.color: input.activeFocus ? root.accent : "#2a3a43" }
        }
    }
    component Action: Button {
        id: control; implicitHeight: 44; leftPadding: 20; rightPadding: 20
        contentItem: Text { text: control.text; color: control.enabled ? root.ink : "#57636b"; font.pixelSize: 13; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
        background: Rectangle { radius: 8; color: control.down ? "#2b433d" : control.hovered ? "#24343c" : "#18252e"; border.color: "#34474f" }
    }
    component Card: Rectangle {
        color: "#101a23"; radius: 16; border.color: "#23343c"; Layout.fillWidth: true
        implicitHeight: body.implicitHeight+40
        default property alias contents: body.data
        ColumnLayout { id: body; anchors.fill: parent; anchors.margins: 20; spacing: 15 }
    }
    RowLayout {
        anchors.fill: parent; spacing: 0
        Rectangle {
            Layout.preferredWidth: 265; Layout.fillHeight: true; color: "#0d151d"
            ColumnLayout {
                anchors.fill: parent; anchors.margins: 25; spacing: 8
                Text { text: "A R C H I E O S"; color: root.ink; font.pixelSize: 19; font.weight: Font.DemiBold; Layout.topMargin: 12 }
                Text { text: "INSTALLATION STUDIO"; color: root.dim; font.pixelSize: 10; font.letterSpacing: 2; Layout.bottomMargin: 34 }
                Repeater {
                    model: root.steps
                    delegate: Rectangle {
                        required property int index; required property string modelData
                        Layout.fillWidth: true; implicitHeight: 51; radius: 9
                        color: root.page===index ? "#20362e" : "transparent"
                        RowLayout {
                            anchors.fill: parent; anchors.leftMargin: 13; spacing: 12
                            Text { text: String(index+1).padStart(2,"0"); color: root.page===index ? root.accent : "#536971"; font.pixelSize: 12; font.family: "monospace" }
                            Text { text: modelData; color: root.page===index ? root.ink : root.dim; font.pixelSize: 13; Layout.fillWidth: true }
                        }
                        MouseArea { anchors.fill: parent; onClicked: root.page=index; cursorShape: Qt.PointingHandCursor }
                    }
                }
                Item { Layout.fillHeight: true }
                Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: "#25333c" }
                Text { text: "TU MUNDO. TU EQUIPO."; color: root.accent; font.pixelSize: 10; font.letterSpacing: 1; Layout.topMargin: 16 }
                Note { text: "Local por diseño.
Sin suscripciones ni APIs de pago."; font.pixelSize: 12 }
                Text { text: wizard.live ? "●  LIVE USB" : "○  PREPARACIÓN LOCAL"; color: root.dim; font.pixelSize: 10; Layout.topMargin: 15 }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true; Layout.fillHeight: true; spacing: 0
            RowLayout {
                Layout.fillWidth: true; Layout.margins: 30
                Text { text: "CONFIGURA TU ESPACIO"; color: root.dim; font.pixelSize: 11; font.letterSpacing: 2; Layout.fillWidth: true }
                Text { text: "0"+(root.page+1)+" / 08"; color: root.accent; font.pixelSize: 13; font.family: "monospace" }
            }
            ScrollView {
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                contentWidth: availableWidth
                ColumnLayout {
                    width: parent.width; spacing: 20
                    ColumnLayout {
                        Layout.fillWidth: true; Layout.leftMargin: 38; Layout.rightMargin: 38; spacing: 22
                        Text { text: root.steps[root.page]; color: root.ink; font.pixelSize: 36; font.weight: Font.DemiBold; Layout.fillWidth: true }
                        ColumnLayout {
                            visible: root.page===0; spacing: 20; Layout.fillWidth: true
                            Card {
                                RowLayout {
                                    Layout.fillWidth: true; spacing: 30
                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        Text { text: "Un comienzo
hecho para ti."; color: root.ink; font.pixelSize: 38; font.weight: Font.DemiBold; lineHeight: 1.15 }
                                        Note { text: "Conecta tu nodo, elige dónde guardar tu mundo y decide cómo quieres que " + root.aiAlias + " te acompañe."; Layout.maximumWidth: 450; Layout.topMargin: 12 }
                                    }
                                    Canvas {
                                        Layout.preferredWidth: 210; Layout.preferredHeight: 210
                                        onPaint: {
                                            var c=getContext("2d");c.clearRect(0,0,width,height);c.strokeStyle="#82b896";c.lineWidth=1
                                            for(var n=0;n<7;n++){c.beginPath();c.ellipse(16+n*12,16,178-n*24,178);c.stroke()}
                                            for(var j=1;j<6;j++){c.beginPath();c.ellipse(16,16+j*12,178,178-j*24);c.stroke()}
                                            c.fillStyle="#b9f2bd";c.beginPath();c.arc(150,61,5,0,Math.PI*2);c.fill()
                                        }
                                    }
                                }
                            }
                            Card {
                                Heading { text: "Este equipo y su conexión" }
                                Field { label: "NOMBRE DEL NODO"; value: root.hostname; onEdited: value=>root.hostname=value }
                                Repeater { model: wizard.networks; delegate: Note { required property var modelData; text: (modelData.up ? "●  " : "○  ")+modelData.name+"   "+(modelData.address || "sin dirección IPv4") } }
                                RowLayout { Action { text: "Configurar red"; onClicked: wizard.openNetwork() } Action { text: "Volver a escanear"; onClicked: wizard.inspectMachine() } }
                            }
                        }
                        ColumnLayout {
                            visible: root.page===1; spacing: 20; Layout.fillWidth: true
                            Note { text: "Tu nombre, tu cuenta y tu forma de conversar son cosas distintas. Puedes personalizarlas por separado." }
                            Card {
                                GridLayout {
                                    columns: 2; columnSpacing: 22; rowSpacing: 22; Layout.fillWidth: true
                                    Field { label: "NOMBRE DEL OWNER"; hint: "Ej. Nelson"; value: root.ownerName; onEdited: value=>root.ownerName=value }
                                    Field { label: "NICKNAME"; hint: "Tu alias personal"; value: root.nickname; onEdited: value=>root.nickname=value }
                                    Field { label: "¿CÓMO QUIERES QUE TE LLAME?"; hint: "Ej. Nelson, Nel o señor Nelson"; value: root.addressAs; onEdited: value=>root.addressAs=value }
                                    Field { label: "ALIAS DE TU IA"; value: root.aiAlias; onEdited: value=>root.aiAlias=value }
                                    Field { label: "USUARIO DEL SISTEMA"; hint: "Ej. nelson (minúsculas)"; value: root.username; onEdited: value=>root.username=value }
                                }
                                Note { text: "La contraseña se introduce en el instalador seguro. No se guarda en este borrador." }
                            }
                            Card {
                                Text { text: "«Hola, "+(root.addressAs || "tu nombre")+". Soy "+root.aiAlias+".»"; color: root.accent; font.pixelSize: 25; wrapMode: Text.WordWrap; Layout.fillWidth: true }
                                Note { text: "Vista previa de tus preferencias. Un nombre o alias no otorga permisos de owner." }
                            }
                        }
                        ColumnLayout {
                            visible: root.page===2; spacing: 20; Layout.fillWidth: true
                            Note { text: "La posición pertenece al nodo. No asumimos dónde vive el owner ni usamos su IP para inventar una ubicación." }
                            Card {
                                Heading { text: "Ubicación con procedencia" }
                                Action { text: "Leer ubicación del nodo local"; onClicked: wizard.useNodeLocation() }
                                CheckBox { text: "Confirmo y quiero guardar estas coordenadas"; checked: root.useLocation; onToggled: root.useLocation=checked; palette.windowText: root.ink }
                                RowLayout {
                                    Field { label: "LATITUD"; hint: "−90 a 90"; value: root.latitude; onEdited: value=>{root.latitude=value;root.locationSource="operator"} }
                                    Field { label: "LONGITUD"; hint: "−180 a 180"; value: root.longitude; onEdited: value=>{root.longitude=value;root.locationSource="operator"} }
                                }
                                Note { text: "Fuente: "+root.locationSource+" · Precisión no declarada. También puedes continuar con ubicación desconocida." }
                            }
                        }
                        ColumnLayout {
                            visible: root.page===3; spacing: 20; Layout.fillWidth: true
                            Note { text: "Separa el sistema de tus mapas. Revisa los volúmenes reales y prepara tus particiones desde el entorno live." }
                            Repeater {
                                model: wizard.volumes
                                delegate: Card {
                                    required property var modelData
                                    Heading { text: modelData.path+"   "+modelData.name; font.pixelSize: 18 }
                                    ProgressBar { Layout.fillWidth: true; value: modelData.total>0 ? 1-modelData.free/modelData.total : 0 }
                                    Note { text: modelData.free.toFixed(1)+" GB disponibles de "+modelData.total.toFixed(1)+" GB"+(modelData.readOnly ? " · solo lectura" : "") }
                                }
                            }
                            RowLayout { Action { text: "Crear / eliminar particiones…"; onClicked: wizard.openPartitions() } Action { text: "Actualizar volúmenes"; onClicked: wizard.inspectMachine() } }
                            Note { text: "El editor GParted aplica sus cambios únicamente cuando los confirmas. El instalador actual usa un disco completo: no elijas para instalar un disco cuyas particiones quieras conservar." }
                            Card { Heading { text: "Arranque desde pendrive" } Note { text: "Arranca la ISO de ArchieOS desde un USB. Puedes instalar en otro disco o pendrive compatible; el medio desde el que arrancaste queda protegido. Este wizard no graba ni borra un USB automáticamente." } }
                        }
                        ColumnLayout {
                            visible: root.page===4; spacing: 20; Layout.fillWidth: true
                            Note { text: "El planeta NASA viene incluido. Añade una región vectorial de OpenStreetMap y conserva el archivo comprimido para futuras consultas y actualizaciones." }
                            Card {
                                Heading { text: "Tu biblioteca del mundo" }
                                Action { text: "Actualizar catálogo de regiones"; onClicked: wizard.refreshCatalog() }
                                CheckBox { text: "Descargar una región durante la instalación"; checked: root.useMap; onToggled: root.useMap=checked; palette.windowText: root.ink }
                                ComboBox {
                                    Layout.fillWidth: true; model: wizard.regions; textRole: "name"; currentIndex: -1
                                    palette.button: "#18252e"; palette.buttonText: root.ink; palette.base: "#101a23"; palette.text: root.ink; palette.highlight: "#315343"; palette.highlightedText: root.ink
                                    displayText: root.regionName || "Selecciona una región del catálogo"
                                    onActivated: {root.regionName=wizard.regions[currentIndex].name;root.regionUrl=wizard.regions[currentIndex].url}
                                }
                                Field { label: "DIRECTORIO EN EL SISTEMA INSTALADO"; value: root.destination; onEdited: value=>root.destination=value }
                                Field { label: "PARTICIÓN SEPARADA (OPCIONAL, YA MONTADA EN EL LIVE)"; hint: "Ej. /dev/sdb1 · vacía = disco del sistema"; value: root.mapPartition; onEdited: value=>root.mapPartition=value }
                                Field { label: "MIRRORS HTTPS OPCIONALES (SEPARADOS POR COMAS)"; hint: "URLs del mismo .osm.pbf, por ejemplo desde tu red"; value: root.mapMirrors; onEdited: value=>root.mapMirrors=value }
                                Note { text: root.regionUrl || "Fuente oficial: Geofabrik · Datos © OpenStreetMap contributors · ODbL"; font.pixelSize: 12 }
                            }
                            Card {
                                Heading { text: "Descarga una vez. Conserva el resultado." }
                                Note { text: "El descargador reutiliza paquetes verificados, reanuda archivos parciales y comprueba integridad antes de publicarlos. Guarda ruta, tamaño, fuente y SHA-256 en un manifiesto local. Los mirrors adicionales deben servir el mismo paquete verificado." }
                                Note { text: "El PBF es la fuente comprimida. Su conversión a un índice de teselas para calles y edificios sigue siendo una etapa independiente." }
                            }
                        }
                        ColumnLayout {
                            visible: root.page===5; spacing: 20; Layout.fillWidth: true
                            Note { text: "Primero probamos tus sensores. Tú decides cuándo se activan. No se guardan grabaciones durante estas pruebas." }
                            Card {
                                CheckBox { text: "Exigir aprobación del celular para activar los sensores"; checked: root.phoneProtection; onToggled: root.phoneProtection=checked; palette.windowText: root.ink }
                                Heading { text: "Cámara e iluminación" }
                                CheckBox { text: "Permito una vista previa local de la cámara"; checked: root.cameraConsent; onToggled: {root.cameraConsent=checked;if(!checked){camera.active=false;wizard.cancelApprovals()}} palette.windowText: root.ink }
                                Rectangle {
                                    Layout.fillWidth: true; implicitHeight: 210; color: "#080d13"; radius: 10; clip: true
                                    VideoOutput { id: video; anchors.fill: parent; fillMode: VideoOutput.PreserveAspectFit; Component.onCompleted: wizard.attachVideo(video.videoSink) }
                                    Text { anchors.centerIn: parent; visible: !camera.active; text: "La cámara está apagada"; color: root.dim }
                                }
                                RowLayout {
                                    Action { text: camera.active ? "Apagar cámara" : "Probar cámara"; enabled: root.cameraConsent && devices.videoInputs.length>0; onClicked: {if(camera.active)camera.active=false;else if(root.phoneProtection)wizard.requestApproval("camera:preview");else camera.active=true} }
                                    Note { text: devices.videoInputs.length===0 ? "No se detectó cámara" : wizard.light<0 ? "Esperando imagen" : wizard.light<0.18 ? "Poca luz: ilumina tu rostro de frente" : wizard.light>0.85 ? "Demasiada luz: evita reflejos" : "Iluminación media adecuada" }
                                }
                                Note { text: "La medición evalúa luz de la imagen, no identidad ni prueba de vida. Una foto de la mano no verifica una huella. El enrolamiento facial y de palma requiere un proveedor biométrico validado."; font.pixelSize: 12 }
                            }
                            Card {
                                Heading { text: "Prueba de voz" }
                                CheckBox { text: "Permito medir el micrófono localmente"; checked: root.microphoneConsent; onToggled: {root.microphoneConsent=checked;if(!checked){wizard.cancelApprovals();root.microphoneRunning=false;wizard.testMicrophone(false)}} palette.windowText: root.ink }
                                Note { text: "Di: «Hola "+root.aiAlias+", soy "+(root.ownerName || "tu nombre")+"»." }
                                ProgressBar { Layout.fillWidth: true; value: wizard.voice }
                                Action { text: root.microphoneRunning ? "Detener prueba" : "Probar micrófono"; enabled: root.microphoneConsent; onClicked: {if(!root.microphoneRunning && root.phoneProtection)wizard.requestApproval("microphone:test");else {root.microphoneRunning=!root.microphoneRunning;wizard.testMicrophone(root.microphoneRunning)}} }
                                Note { text: "Se mide volumen, no reconocimiento de voz. No se crea un perfil biométrico con esta prueba."; font.pixelSize: 12 }
                            }
                        }
                        ColumnLayout {
                            visible: root.page===6; spacing: 20; Layout.fillWidth: true
                            Card {
                                Heading { text: "Tu teléfono, tu aprobación" }
                                Note { text: "El servicio de passkeys vincula tu celular con este nodo mediante QR y confirmación en ambos dispositivos. Cada permiso muestra la acción y caduca tras un uso." }
                                Note { text: "La huella o el rostro se verifican dentro del teléfono. Archie recibe una prueba criptográfica, nunca la huella. El QR por sí solo no activa al owner ni concede permisos." }
                                RowLayout { Action { text: "Vincular Android / iPhone…"; onClicked: wizard.openPhone() } Action { text: "Copiar código de activación"; onClicked: wizard.copyActivation() } }
                                Note { text: "Abre el servicio local owner-companion, con PostgreSQL y la identidad del nodo configurados. Las aprobaciones pueden autorizar una prueba de sensores del wizard si activas esa protección. Su conexión con otros permisos del Core sigue pendiente. El QR seguro lo muestra el navegador, no esta pantalla." }
                            }
                        }
                        ColumnLayout {
                            visible: root.page===7; spacing: 20; Layout.fillWidth: true
                            Card {
                                Heading { text: "Todo empieza con una decisión tuya." }
                                Note { text: "Nodo: "+root.hostname+"\nOwner: "+(root.ownerName || "pendiente")+" · "+(root.username || "usuario pendiente")+"\nTe llamaré: "+(root.addressAs || "pendiente")+"\nTu IA: "+root.aiAlias+"\nUbicación: "+(root.useLocation ? root.latitude+", "+root.longitude : "desconocida")+"\nMapa: "+(root.useMap ? (root.regionName || "región pendiente")+" → "+root.destination : "NASA local, sin descarga") }
                                Note { text: "El siguiente paso abre el instalador de disco. Allí se valida este perfil, eliges el destino, defines tu contraseña y revisas el plan completo antes de confirmar el borrado." }
                                RowLayout {
                                    Action { text: "Guardar preparación"; onClicked: wizard.saveDraft(root.profile()) }
                                    Action { text: "Abrir instalador seguro"; enabled: wizard.live; onClicked: {if(wizard.saveDraft(root.profile()))wizard.launchInstall()} }
                                }
                                Note { text: wizard.draftPath; font.pixelSize: 11 }
                            }
                        }
                        Item { implicitHeight: 15 }
                    }
                }
            }
            Rectangle { Layout.fillWidth: true; height: 1; color: "#22313a" }
            ColumnLayout {
                Layout.fillWidth: true; Layout.margins: 24; spacing: 15
                Note { text: wizard.status; font.pixelSize: 12 }
                RowLayout {
                    Layout.fillWidth: true
                    Action { text: "← Atrás"; enabled: root.page>0; onClicked: root.page-- }
                    Item { Layout.fillWidth: true }
                    Text { text: "Cambios de disco: ninguno desde esta pantalla"; color: root.dim; font.pixelSize: 11 }
                    Action { text: root.page<7 ? "Continuar  →" : "Guardar preparación"; onClicked: {if(root.page<7)root.page++;else wizard.saveDraft(root.profile())} }
                }
            }
        }
    }
    MediaDevices { id: devices }
    CaptureSession { camera: Camera { id: camera; active: false; onErrorOccurred: (error,errorString)=>console.warn(errorString) } videoOutput: video }
    Connections {
        target: wizard
        function onPermissionGranted(resource) {
            if(root.page!==5)return
            if(resource==="camera:preview" && root.cameraConsent)camera.active=true
            if(resource==="microphone:test" && root.microphoneConsent){root.microphoneRunning=true;wizard.testMicrophone(true)}
        }
        function onLocationReady() {
            if(wizard.location.latitude!==undefined) {root.latitude=String(wizard.location.latitude);root.longitude=String(wizard.location.longitude);root.locationSource="node"}
        }
    }
    onClosing: { wizard.cancelApprovals();camera.active=false;wizard.testMicrophone(false) }
}
