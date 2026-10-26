import QtQuick
import QtQuick3D
import Archie.Observatory

// Xavi's neural sphere (Lumenfield's "Esfera neuronal"). Rust (NeuralField) decides every
// position, colour and spark; this file says which material each buffer wears, where the
// camera sits and how it turns. Layers, as scene.ts adds them: link layers, the selection's
// hot lines, node dots, ghost particles and the globe (all in the field), then the
// projection base, its beam and the tilted orbit.
//
// The camera is OrbitControls': it starts at HOME = (0.55, 0.42, 3.55) R looking at
// (0, -0.08 R, 0), turns on its own at 0.3 x rotationSpeed (a minute a turn at 1) and the
// operator drags and zooms it.
Node {
    id: scene
    required property NeuralField field
    property real pixelRatio: 1
    property bool running: true
    property bool reducedMotion: false
    property int frameIntervalMs: 16
    property int particleLimit: 2000
    property int requestedPulses: 70
    property bool linksVisible: true
    property bool shellVisible: true
    property alias camera: camera

    readonly property real r: 100
    readonly property var cfg: field.settings
    readonly property real brightness: Number(cfg.brightness === undefined ? 1 : cfg.brightness)
    readonly property real linkOpacity: Number(cfg.linkOpacity === undefined ? 1 : cfg.linkOpacity)
    // Ghost geometry has 1600 fixed decorative points. Omit it when the governor cannot
    // afford the whole buffer; every spark has a trail of five points.
    readonly property bool drawGhosts: !!cfg.ghosts && particleLimit >= 1600
    readonly property int pulseLimit: Math.min(requestedPulses,70, Math.max(0, Math.floor((particleLimit - (drawGhosts ? 1600 : 0)) / 5)))
    readonly property vector3d cyan: Qt.vector3d(0.1144, 0.6584, 1.0)   // 0x5fd4ff in linear light

    // The orbit about the target: degrees and a distance. HOME in those terms.
    readonly property vector3d home: Qt.vector3d(r * 0.55, r * 0.42, r * 3.55)
    readonly property vector3d homeTarget: Qt.vector3d(0, -r * 0.08, 0)
    property vector3d target: homeTarget
    property real yaw: 0
    property real elevation: 0
    property real distance: 1

    function orbitOf(position, centre) {
        const d = Qt.vector3d(position.x - centre.x, position.y - centre.y, position.z - centre.z)
        const length = Math.sqrt(d.x * d.x + d.y * d.y + d.z * d.z)
        return { yaw: Math.atan2(d.x, d.z) * 180 / Math.PI, elevation: Math.asin(d.y / length) * 180 / Math.PI, distance: length }
    }
    function fit() {
        const o = orbitOf(home, homeTarget)
        target = homeTarget
        yaw = o.yaw
        elevation = o.elevation
        distance = o.distance
    }
    function orbit(dYaw, dElevation) {
        yaw += dYaw
        elevation = Math.max(-89, Math.min(89, elevation + dElevation))
    }
    function zoom(factor) {
        distance = Math.max(r * 0.4, Math.min(r * 12, distance * factor))
    }
    function place() {
        const e = elevation * Math.PI / 180
        const a = yaw * Math.PI / 180
        camera.position = Qt.vector3d(target.x + distance * Math.cos(e) * Math.sin(a),
                                      target.y + distance * Math.sin(e),
                                      target.z + distance * Math.cos(e) * Math.cos(a))
        camera.lookAt(target)
    }
    onYawChanged: place()
    onElevationChanged: place()
    onDistanceChanged: place()
    onTargetChanged: place()
    Component.onCompleted: { fit(); applyParticleBudget() }

    function link(kind) {
        const links = cfg.links
        return links && links[String(kind)] ? links[String(kind)] : { on: kind < 2, opacity: 1 }
    }

    function applyParticleBudget() {
        if (field) field.setNumber("pulses", pulseLimit)
    }
    onPulseLimitChanged: applyParticleBudget()
    Connections {
        target: scene.field
        function onSourceChanged() { scene.applyParticleBudget() }
    }

    // One clock for the model, shaders and camera, paced by the governor. Timer stops
    // while hidden/frozen; restarting never charges elapsed time spent off screen.
    Timer {
        id: frameTimer
        property double previous: 0
        interval: Math.max(16, scene.frameIntervalMs)
        repeat: true
        running: scene.running && scene.frameIntervalMs > 0 && !scene.field.still
        onRunningChanged: previous = Date.now()
        onTriggered: {
            const now = Date.now()
            const dt = scene.field.advance(Math.max(0, (now - previous) / 1000))
            previous = now
            if (!scene.reducedMotion)
                scene.yaw -= 6 * 0.3 * Number(scene.cfg.rotationSpeed === undefined ? 1 : scene.cfg.rotationSpeed) * dt
        }
    }

    PerspectiveCamera {
        id: camera
        fieldOfView: 40
        clipNear: 1
        clipFar: 3000
    }

    Node {
        id: fieldNode

        // --- Link layers: 0 skeleton and axons (their opacity is inside the colours), 2..6 the
        // meshes, bridges and the host's links, 7 the selection's hot lines.
        LineLayer { linkLayer: 0; visible: scene.linksVisible }
        LineLayer { linkLayer: 2; visible: scene.linksVisible && scene.link(2).on; alpha: scene.link(2).opacity * scene.linkOpacity }
        LineLayer { linkLayer: 3; visible: scene.linksVisible && scene.link(3).on; alpha: scene.link(3).opacity * scene.linkOpacity }
        LineLayer { linkLayer: 4; visible: scene.linksVisible && scene.link(4).on; alpha: scene.link(4).opacity * scene.linkOpacity }
        LineLayer { linkLayer: 5; visible: scene.linksVisible && scene.link(5).on; alpha: scene.link(5).opacity * scene.linkOpacity }
        LineLayer { linkLayer: 6; visible: scene.linksVisible && scene.link(6).on; alpha: scene.link(6).opacity * scene.linkOpacity }
        LineLayer { linkLayer: 7; visible: scene.linksVisible }

        // --- The nodes ---
        Model {
            geometry: NeuralGeometry { field: scene.field; kind: NeuralGeometry.Points }
            materials: CustomMaterial {
                shadingMode: CustomMaterial.Unshaded
                sourceBlend: CustomMaterial.SrcAlpha
                destinationBlend: CustomMaterial.One
                cullMode: Material.NoCulling
                depthDrawMode: Material.NeverDepthDraw
                property real time: scene.field.time
                property real pixelRatio: scene.pixelRatio
                property real nearDistance: scene.r * 2.4
                property real farDistance: scene.r * 4.8
                property real brightness: scene.brightness
                property real nodeSize: Number(scene.cfg.nodeSize === undefined ? 1 : scene.cfg.nodeSize)
                vertexShader: "qrc:/archie/shaders/neural_point.vert"
                fragmentShader: "qrc:/archie/shaders/neural_point.frag"
            }
        }

        // --- The signals travelling along the links ---
        Model {
            visible: scene.pulseLimit > 0 && Number(scene.cfg.pulses === undefined ? 70 : scene.cfg.pulses) > 0
            geometry: NeuralGeometry { field: scene.field; kind: NeuralGeometry.Sparks }
            materials: CustomMaterial {
                shadingMode: CustomMaterial.Unshaded
                sourceBlend: CustomMaterial.SrcAlpha
                destinationBlend: CustomMaterial.One
                cullMode: Material.NoCulling
                depthDrawMode: Material.NeverDepthDraw
                property real pixelRatio: scene.pixelRatio
                property real brightness: scene.brightness
                vertexShader: "qrc:/archie/shaders/neural_spark.vert"
                fragmentShader: "qrc:/archie/shaders/neural_spark.frag"
            }
        }

        // --- Ghost particles on the surface (decoration, not files) and the globe's rings ---
        LineLayer {
            kind: NeuralGeometry.Ghosts
            visible: scene.drawGhosts
            tint: scene.cyan; vertexColours: 0; gain: 1
            alpha: 0.22 * Number(scene.cfg.ghostOpacity === undefined ? 1 : scene.cfg.ghostOpacity)
            pointSize: 1.3 * scene.pixelRatio
        }
        LineLayer { kind: NeuralGeometry.Segments; linkLayer: 1; visible: scene.shellVisible; tint: scene.cyan; vertexColours: 0; gain: 1; alpha: 0.07 }
    }

    // --- The projection base, its beam and the tilted orbit ---
    Node {
        visible: !!scene.cfg.base
        Node {
            rotation: scene.field.baseRotation(scene.field.time)
            LineLayer { kind: NeuralGeometry.Segments; linkLayer: 2; tint: scene.cyan; vertexColours: 0; gain: 1; alpha: 0.12 }
        }
        Model {
            geometry: NeuralGeometry { kind: NeuralGeometry.Cone }
            materials: CustomMaterial {
                shadingMode: CustomMaterial.Unshaded
                sourceBlend: CustomMaterial.SrcAlpha
                destinationBlend: CustomMaterial.One
                cullMode: Material.NoCulling
                depthDrawMode: Material.NeverDepthDraw
                vertexShader: "qrc:/archie/shaders/neural_cone.vert"
                fragmentShader: "qrc:/archie/shaders/neural_cone.frag"
            }
        }
        Node {
            rotation: scene.field.orbitRotation(scene.field.time)
            LineLayer { kind: NeuralGeometry.Segments; linkLayer: 3; tint: scene.cyan; vertexColours: 0; gain: 1; alpha: 0.12 }
        }
    }

    // One buffer of segments, points or particles with the original's line material: colour
    // per vertex (or one tint), additive, and the fog that takes distant light back to the
    // background.
    component LineLayer: Model {
        id: layer
        property int linkLayer: 0
        property int kind: NeuralGeometry.Lines
        property real alpha: 1
        property vector3d tint: Qt.vector3d(1, 1, 1)
        property real vertexColours: 1
        property real gain: scene.brightness
        property real pointSize: 1
        geometry: NeuralGeometry { field: scene.field; kind: layer.kind; layer: layer.linkLayer }
        materials: CustomMaterial {
            shadingMode: CustomMaterial.Unshaded
            sourceBlend: CustomMaterial.SrcAlpha
            destinationBlend: CustomMaterial.One
            cullMode: Material.NoCulling
            depthDrawMode: Material.NeverDepthDraw
            property vector3d tint: layer.tint
            property real vertexColours: layer.vertexColours
            property real brightness: layer.gain
            property real alpha: layer.alpha
            property real pointSize: layer.pointSize
            property real fogNear: scene.r * 2
            property real fogFar: scene.r * 5
            property vector3d fogColour: scene.field.backgroundLinear
            vertexShader: "qrc:/archie/shaders/neural_line.vert"
            fragmentShader: "qrc:/archie/shaders/neural_line.frag"
        }
    }
}
