import QtQuick
import QtQuick3D
import Archie.Observatory

// The orb. Every buffer here is filled by `lumen` (LumenField); this file says which
// material each layer wears and how the camera moves. Layers, back to front: membrane,
// ghost particles, links, relations, satellites and their fibres, orbs, their glow,
// travelling signals, and the ring around the selection.
//
// The camera is cinematic, and it moves because something happened: a new field
// projects out of the centre while the camera pulls in from far away; when the system
// starts using something, the camera leans towards it; when it lets go, the camera
// settles back. Between events there is only a slow drift of the view.
Node {
    id: scene
    required property var controller
    property bool reducedMotion: false
    property bool presented: true

    property alias camera: lumenCamera
    property alias field: field
    property real aspect: 1.6
    property real flash: 0
    // The orbit: yaw and pitch about the target, and a distance.
    property real yaw: 4
    property real pitch: -6
    property real distance: 900
    property vector3d target: Qt.vector3d(0, 0, 0)
    property real home: 900
    readonly property var look: scene.controller.look
    readonly property var sceneInfo: scene.controller.scene

    function homeDistance() {
        const radius = Math.max(scene.controller.fieldRadius * 0.5, scene.controller.shellRadius || scene.controller.fieldRadius)
        const half = 20 * Math.PI / 180
        return radius * Math.max(3.5, 1.3 / (Math.tan(half) * Math.max(1.0, scene.aspect)))
    }
    function glide(distance, target, duration) {
        travel.stop()
        if (scene.reducedMotion || !scene.presented) {
            scene.distance = distance
            scene.target = target
            return
        }
        dolly.to = distance
        pan.to = target
        dolly.duration = duration
        pan.duration = duration
        travel.start()
    }
    function fit() {
        scene.home = scene.homeDistance()
        scene.glide(scene.home, Qt.vector3d(0, 0, 0), 1100)
    }
    // A window that is still being laid out reports no size; the framing follows it
    // once it has one, unless the operator has moved the camera since.
    onAspectChanged: {
        const next = scene.homeDistance()
        if (Math.abs(scene.distance - scene.home) < 1 && !travel.running)
            scene.distance = next
        else if (travel.running && Math.abs(dolly.to - scene.home) < 1)
            scene.glide(next, pan.to, 1200)
        scene.home = next
    }

    function approach(factor) {
        travel.stop()
        scene.distance = Math.max(20, Math.min(8000, scene.distance * factor))
    }

    ParallelAnimation {
        id: travel
        NumberAnimation { id: dolly; target: scene; property: "distance"; easing.type: Easing.InOutCubic }
        Vector3dAnimation { id: pan; target: scene; property: "target"; easing.type: Easing.InOutCubic }
    }
    onReducedMotionChanged: if (reducedMotion && travel.running) {
        travel.stop()
        scene.distance = dolly.to
        scene.target = pan.to
    }
    onPresentedChanged: if (!presented) travel.stop()

    Connections {
        target: scene.controller
        function onFitRequested() { scene.fit() }
        function onFocusRequested(position) {
            const world = field.mapPositionToScene(position)
            scene.glide(scene.distance, Qt.vector3d(world.x * 0.3, world.y * 0.3, world.z * 0.3), 900)
        }
        // Projection: from far away, the camera pulls in while the field unfolds.
        function onProjectionStarted() {
            scene.home = scene.homeDistance()
            travel.stop()
            scene.distance = scene.home * 2.4
            scene.target = Qt.vector3d(0, 0, 0)
            scene.glide(scene.home, Qt.vector3d(0, 0, 0), 2600)
        }
        // Attention: lean towards what the system is using, then settle back.
        function onAttentionMoved(position, engaged) {
            if (!!scene.controller.selection.id)
                return
            if (engaged) {
                const world = field.mapPositionToScene(position)
                scene.glide(scene.home * 0.78, Qt.vector3d(world.x * 0.35, world.y * 0.35, world.z * 0.35), 1600)
            } else {
                scene.glide(scene.home, Qt.vector3d(0, 0, 0), 2200)
            }
        }
    }

    Node {
        position: scene.target
        // A slow drift of the view between events: a few degrees, over about a minute.
        eulerRotation: Qt.vector3d(scene.pitch + (scene.reducedMotion ? 0 : Math.sin(scene.controller.time * 0.09) * 3.0),
                                   scene.yaw + (scene.reducedMotion ? 0 : Math.sin(scene.controller.time * 0.05) * 4.0), 0)
        PerspectiveCamera {
            id: lumenCamera
            position: Qt.vector3d(0, 0, scene.distance)
            fieldOfView: 40
            clipNear: 1
            clipFar: 20000
        }
    }

    Node {
        id: field
        eulerRotation: Qt.vector3d(0, scene.controller.spin, 0)

        // --- The membrane ---
        Model {
            visible: scene.look.shellOpacity > 0
            geometry: LumenGeometry { id: membraneMesh }
            Component.onCompleted: scene.controller.bindLayer("membrane", membraneMesh)
            materials: CustomMaterial {
                shadingMode: CustomMaterial.Unshaded
                sourceBlend: CustomMaterial.SrcAlpha
                destinationBlend: CustomMaterial.OneMinusSrcAlpha
                cullMode: Material.NoCulling
                depthDrawMode: Material.NeverDepthDraw
                property real time: scene.controller.time
                property real alpha: scene.look.shellOpacity
                property real legacy: scene.look.legacy
                property vector3d warm: scene.look.warm
                property vector3d cool: scene.look.cool
                property real listen: Number(scene.sceneInfo.listen || 0)
                property real think: Number(scene.sceneInfo.think || 0)
                property real voice: Number(scene.sceneInfo.speak || 0) * Number(scene.sceneInfo.audio || 0)
                property real flash: scene.flash
                vertexShader: "qrc:/archie/shaders/lumen_shell.vert"
                fragmentShader: "qrc:/archie/shaders/lumen_shell.frag"
            }
        }

        // --- Ghost particles: density only, never an entity ---
        Model {
            visible: scene.look.ghostOpacity > 0
            geometry: LumenGeometry { id: ghostQuad }
            instancing: LumenInstancing { id: ghostTable }
            Component.onCompleted: { scene.controller.bindLayer("quad", ghostQuad); scene.controller.bindLayer("ghosts", ghostTable) }
            materials: SpriteMaterial {
                grow: scene.look.ghostSize * 1.3
                alpha: scene.look.ghostOpacity
                sharpness: 1.7
            }
        }

        // --- Links and relations ---
        LineLayer { name: "backbone"; tint: Qt.vector3d(0.26, 0.38, 0.39); alpha: scene.look.linkOpacity * 4; visible: scene.look.links }
        LineLayer { name: "fibres"; tint: scene.look.cool; alpha: 0.14; visible: scene.look.links }
        LineLayer { name: "observed"; tint: scene.look.warm; alpha: 0.4; intensity: 1.8; visible: scene.look.links }
        LineLayer { name: "semantic"; tint: scene.look.cool; alpha: 0.22; intensity: 1.6; visible: scene.look.links }
        LineLayer { name: "guides"; tint: scene.look.warm; alpha: 0.15; visible: scene.look.guides }
        LineLayer { name: "highlight"; tint: scene.look.warm; alpha: scene.look.highlightOpacity; intensity: 1.8 }

        // --- Satellites: small beads around each module ---
        Model {
            geometry: LumenGeometry { id: satelliteMesh }
            instancing: LumenInstancing { id: satelliteTable }
            Component.onCompleted: { scene.controller.bindLayer("satelliteMesh", satelliteMesh); scene.controller.bindLayer("satellites", satelliteTable) }
            materials: OrbMaterial { facet: 0.2; shapeAmount: 0.12; detail: 0.0 }
        }

        // --- Orbs: modules are cut gems, entities polished beads; neither is a plain ball ---
        Model {
            geometry: LumenGeometry { id: orbMesh }
            instancing: LumenInstancing { id: moduleTable }
            Component.onCompleted: { scene.controller.bindLayer("orbMesh", orbMesh); scene.controller.bindLayer("modules", moduleTable) }
            materials: OrbMaterial { facet: 0.3; shapeAmount: 0.16; detail: 1.0 }
        }
        Model {
            geometry: LumenGeometry { id: entityMesh }
            instancing: LumenInstancing { id: entityTable }
            Component.onCompleted: { scene.controller.bindLayer("entityMesh", entityMesh); scene.controller.bindLayer("entities", entityTable) }
            materials: OrbMaterial { facet: 0.15; shapeAmount: 0.1; detail: 0.7 }
        }

        // --- Glow: a halo per orb, brighter with activity, focus and selection ---
        Model {
            geometry: LumenGeometry { id: moduleHaloQuad }
            instancing: moduleTable
            Component.onCompleted: scene.controller.bindLayer("quad", moduleHaloQuad)
            materials: SpriteMaterial { grow: 2.8; alpha: 0.3 + scene.look.bloomStrength * 0.16; sharpness: 1.8; haloGain: 0.9 }
        }
        Model {
            geometry: LumenGeometry { id: entityHaloQuad }
            instancing: entityTable
            Component.onCompleted: scene.controller.bindLayer("quad", entityHaloQuad)
            materials: SpriteMaterial { grow: 3.3; alpha: 0.28 + scene.look.bloomStrength * 0.16; sharpness: 1.7; haloGain: 1.1 }
        }

        // --- Signals travelling along real links ---
        Model {
            visible: scene.look.pulses
            geometry: LumenGeometry { id: pulseQuad }
            instancing: LumenInstancing { id: pulseTable }
            Component.onCompleted: { scene.controller.bindLayer("quad", pulseQuad); scene.controller.bindLayer("pulses", pulseTable) }
            materials: SpriteMaterial { grow: 1.0; alpha: 0.83; sharpness: 1.4 }
        }

        // --- The selection ---
        Model {
            visible: scene.controller.selectionVisible
            position: scene.controller.selectionPosition
            scale: Qt.vector3d(scene.controller.selectionScale, scene.controller.selectionScale, scene.controller.selectionScale)
            geometry: LumenGeometry { id: ringMesh }
            Component.onCompleted: scene.controller.bindLayer("ring", ringMesh)
            materials: CustomMaterial {
                shadingMode: CustomMaterial.Unshaded
                sourceBlend: CustomMaterial.SrcAlpha
                destinationBlend: CustomMaterial.One
                cullMode: Material.NoCulling
                depthDrawMode: Material.NeverDepthDraw
                property vector3d tint: Qt.vector3d(1.0, 0.72, 0.34)
                vertexShader: "qrc:/archie/shaders/lumen_ring.vert"
                fragmentShader: "qrc:/archie/shaders/lumen_ring.frag"
            }
        }
    }

    component LineLayer: Model {
        id: layer
        property string name: ""
        property vector3d tint: Qt.vector3d(1, 1, 1)
        property real alpha: 0.2
        property real intensity: 1.0
        geometry: LumenGeometry { id: lines }
        Component.onCompleted: scene.controller.bindLayer(name, lines)
        materials: CustomMaterial {
            shadingMode: CustomMaterial.Unshaded
            sourceBlend: CustomMaterial.SrcAlpha
            destinationBlend: CustomMaterial.One
            cullMode: Material.NoCulling
            depthDrawMode: Material.NeverDepthDraw
            property vector3d tint: layer.tint
            property real alpha: layer.alpha
            property real intensity: layer.intensity
            vertexShader: "qrc:/archie/shaders/lumen_line.vert"
            fragmentShader: "qrc:/archie/shaders/lumen_line.frag"
        }
    }

    component SpriteMaterial: CustomMaterial {
        property real grow: 1.0
        property real alpha: 0.5
        property real sharpness: 2.0
        property real haloGain: 0.0
        property vector3d tintOverride: Qt.vector3d(1, 1, 1)
        property real tintMix: 0.0
        shadingMode: CustomMaterial.Unshaded
        sourceBlend: CustomMaterial.SrcAlpha
        destinationBlend: CustomMaterial.One
        cullMode: Material.NoCulling
        depthDrawMode: Material.NeverDepthDraw
        vertexShader: "qrc:/archie/shaders/lumen_sprite.vert"
        fragmentShader: "qrc:/archie/shaders/lumen_sprite.frag"
    }

    component OrbMaterial: CustomMaterial {
        property real facet: 0.5
        property real shapeAmount: 0.35
        property real detail: 1.0
        property real time: scene.controller.time
        property vector3d warm: scene.controller.look.warm
        property vector3d cool: scene.controller.look.cool
        property vector3d warmLight: scene.controller.look.warmLight
        property vector3d coolLight: scene.controller.look.coolLight
        property vector3d ambientLight: scene.controller.look.ambientLight
        property vector3d warmPosition: scene.controller.look.warmPosition
        property vector3d coolPosition: scene.controller.look.coolPosition
        property real metalness: scene.controller.look.metalness
        property real roughness: scene.controller.look.roughness
        property real emissive: scene.controller.look.emissive
        property real textureContrast: scene.controller.look.textureContrast
        property real textureDepth: scene.controller.look.textureDepth
        shadingMode: CustomMaterial.Unshaded
        cullMode: Material.BackFaceCulling
        vertexShader: "qrc:/archie/shaders/lumen_orb.vert"
        fragmentShader: "qrc:/archie/shaders/lumen_orb.frag"
    }
}
