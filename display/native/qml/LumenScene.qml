import QtQuick
import QtQuick3D
import Archie.Observatory

// The orb, drawn the Lumenfield way. Every buffer here is filled by `lumen` (LumenField):
// this file only says which material each layer wears. The layers, back to front:
// membrane, ghost particles, links, relations, satellites and their fibres, orbs, their
// glow, travelling signals, and the ring around the selection.
Node {
    id: scene

    property alias camera: lumenCamera
    property alias field: field
    property real aspect: 1.6
    property real flash: 0
    // The operator's orbit: yaw and pitch about the target, and a distance.
    property real yaw: 4
    property real pitch: -2.3
    property real distance: 900
    property vector3d target: Qt.vector3d(0, 0, 0)
    readonly property var look: lumen.look
    readonly property var sceneInfo: lumen.scene

    Behavior on distance { NumberAnimation { duration: 520; easing.type: Easing.OutCubic } }
    Behavior on target { Vector3dAnimation { duration: 520; easing.type: Easing.OutCubic } }

    function fit() {
        const radius = Math.max(lumen.fieldRadius * 0.5, lumen.shellRadius || lumen.fieldRadius)
        const half = 22 * Math.PI / 180
        scene.distance = radius * Math.max(3.2, 1.2 / (Math.tan(half) * Math.max(0.3, scene.aspect)))
        scene.target = Qt.vector3d(0, 0, 0)
        scene.yaw = 4
        scene.pitch = -2.3
    }

    Connections {
        target: lumen
        function onFitRequested() { scene.fit() }
        // "Centrar": the camera turns partway towards the orb, the design's gentle focus.
        function onFocusRequested(position) {
            const world = field.mapPositionToScene(position)
            scene.target = Qt.vector3d(world.x * 0.22, world.y * 0.22, world.z * 0.22)
        }
        function onSceneChanged() { if (!scene.fitted && lumen.loaded) { scene.fitted = true; scene.fit() } }
    }
    property bool fitted: false

    Node {
        position: scene.target
        eulerRotation: Qt.vector3d(scene.pitch, scene.yaw, 0)
        PerspectiveCamera {
            id: lumenCamera
            position: Qt.vector3d(0, 0, scene.distance)
            fieldOfView: 44
            clipNear: 1
            clipFar: 20000
        }
    }

    Node {
        id: field
        eulerRotation: Qt.vector3d(0, lumen.spin, 0)

        // --- The membrane ---
        Model {
            visible: scene.look.shellOpacity > 0
            geometry: LumenGeometry { id: membraneMesh }
            Component.onCompleted: lumen.bindLayer("membrane", membraneMesh)
            materials: CustomMaterial {
                shadingMode: CustomMaterial.Unshaded
                sourceBlend: CustomMaterial.SrcAlpha
                destinationBlend: CustomMaterial.OneMinusSrcAlpha
                cullMode: Material.NoCulling
                depthDrawMode: Material.NeverDepthDraw
                property real time: lumen.time
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
            Component.onCompleted: { lumen.bindLayer("quad", ghostQuad); lumen.bindLayer("ghosts", ghostTable) }
            materials: SpriteMaterial {
                grow: scene.look.ghostSize * 1.3
                alpha: scene.look.ghostOpacity
                sharpness: 1.7
            }
        }

        // --- Links and relations ---
        LineLayer { name: "backbone"; tint: Qt.vector3d(0.26, 0.38, 0.39); alpha: scene.look.linkOpacity * 4; visible: scene.look.links }
        LineLayer { name: "fibres"; tint: scene.look.cool; alpha: 0.074; visible: scene.look.links }
        LineLayer { name: "observed"; tint: scene.look.warm; alpha: 0.25; intensity: 1.4; visible: scene.look.links }
        LineLayer { name: "semantic"; tint: scene.look.cool; alpha: 0.12; intensity: 1.3; visible: scene.look.links }
        LineLayer { name: "guides"; tint: scene.look.warm; alpha: 0.15; visible: scene.look.guides }
        LineLayer { name: "highlight"; tint: scene.look.warm; alpha: scene.look.highlightOpacity; intensity: 1.8 }

        // --- Satellites: small beads around each module ---
        Model {
            geometry: LumenGeometry { id: satelliteMesh }
            instancing: LumenInstancing { id: satelliteTable }
            Component.onCompleted: { lumen.bindLayer("satelliteMesh", satelliteMesh); lumen.bindLayer("satellites", satelliteTable) }
            materials: OrbMaterial { facet: 0.35; shapeAmount: 0.3; detail: 0.0 }
        }

        // --- Orbs: modules are cut gems, entities polished beads; neither is a plain ball ---
        Model {
            geometry: LumenGeometry { id: orbMesh }
            instancing: LumenInstancing { id: moduleTable }
            Component.onCompleted: { lumen.bindLayer("orbMesh", orbMesh); lumen.bindLayer("modules", moduleTable) }
            materials: OrbMaterial { facet: 0.55; shapeAmount: 0.42; detail: 1.0 }
        }
        Model {
            geometry: LumenGeometry { id: entityMesh }
            instancing: LumenInstancing { id: entityTable }
            Component.onCompleted: { lumen.bindLayer("entityMesh", entityMesh); lumen.bindLayer("entities", entityTable) }
            materials: OrbMaterial { facet: 0.3; shapeAmount: 0.3; detail: 0.7 }
        }

        // --- Glow: a halo per orb, brighter with activity, focus and selection ---
        Model {
            geometry: LumenGeometry { id: moduleHaloQuad }
            instancing: moduleTable
            Component.onCompleted: lumen.bindLayer("quad", moduleHaloQuad)
            materials: SpriteMaterial { grow: 2.7; alpha: 0.2 + scene.look.bloomStrength * 0.14; sharpness: 2.0; haloGain: 0.9 }
        }
        Model {
            geometry: LumenGeometry { id: entityHaloQuad }
            instancing: entityTable
            Component.onCompleted: lumen.bindLayer("quad", entityHaloQuad)
            materials: SpriteMaterial { grow: 3.2; alpha: 0.18 + scene.look.bloomStrength * 0.14; sharpness: 1.9; haloGain: 1.1 }
        }

        // --- Signals travelling along real links ---
        Model {
            visible: scene.look.pulses
            geometry: LumenGeometry { id: pulseQuad }
            instancing: LumenInstancing { id: pulseTable }
            Component.onCompleted: { lumen.bindLayer("quad", pulseQuad); lumen.bindLayer("pulses", pulseTable) }
            materials: SpriteMaterial { grow: 1.0; alpha: 0.83; sharpness: 1.4 }
        }

        // --- The selection ---
        Model {
            visible: lumen.selectionVisible
            position: lumen.selectionPosition
            scale: Qt.vector3d(lumen.selectionScale, lumen.selectionScale, lumen.selectionScale)
            geometry: LumenGeometry { id: ringMesh }
            Component.onCompleted: lumen.bindLayer("ring", ringMesh)
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
        Component.onCompleted: lumen.bindLayer(name, lines)
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
        property real time: lumen.time
        property vector3d warm: lumen.look.warm
        property vector3d warmLight: lumen.look.warmLight
        property vector3d coolLight: lumen.look.coolLight
        property vector3d ambientLight: lumen.look.ambientLight
        property vector3d warmPosition: lumen.look.warmPosition
        property vector3d coolPosition: lumen.look.coolPosition
        property real metalness: lumen.look.metalness
        property real roughness: lumen.look.roughness
        property real emissive: lumen.look.emissive
        property real textureContrast: lumen.look.textureContrast
        property real textureDepth: lumen.look.textureDepth
        shadingMode: CustomMaterial.Unshaded
        cullMode: Material.BackFaceCulling
        vertexShader: "qrc:/archie/shaders/lumen_orb.vert"
        fragmentShader: "qrc:/archie/shaders/lumen_orb.frag"
    }
}
