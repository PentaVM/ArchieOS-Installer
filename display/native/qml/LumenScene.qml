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
    property real viewportWidth: 1280
    property real viewportHeight: 800
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
            clipNear: Number(scene.look.linkStyleId || 0) > 0 ? 0.11 : 1
            clipFar: 20000
        }
    }

    // --- The hologram's frame (Xavi's desktop sphere): it stays put while the field turns ---
    Node {
        id: holoFrame
        visible: !!scene.look.holoFrame
        LineLayer { name: "holoRings"; tint: scene.look.cool; alpha: Number(scene.look.holoOpacity || 0); intensity: 1.4 }
        LineLayer { name: "holoBeam"; tint: scene.look.cool; alpha: Number(scene.look.holoOpacity || 0) * 0.12; intensity: 1.2 }
        LineLayer { name: "holoCore"; tint: scene.look.warm; alpha: Math.min(1, Number(scene.look.holoOpacity || 0) * 2.2); intensity: 2.6 }
    }

    Node {
        id: field
        eulerRotation: Qt.vector3d(0, scene.controller.spin, 0)

        // --- The core: one bright star at the centre, and a faint haze behind the sphere ---
        GlowBall {
            readonly property real r: scene.controller.fieldRadius * 0.05 * Number(scene.look.coreGlow || 0)
            scale: Qt.vector3d(r / 50, r / 50, r / 50)
            tint: Qt.vector3d(1.0, 0.93, 0.82); gain: 4.0; alpha: 1.0; sharpness: 9.0
        }
        GlowBall {
            readonly property real r: scene.controller.fieldRadius * 0.95
            scale: Qt.vector3d(r / 50, r / 50, r / 50)
            tint: scene.look.warm; gain: 0.1 * Number(scene.look.coreGlow || 0); alpha: 1.0; sharpness: 2.2
        }

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
                property int shellKind: Number(scene.look.shellStyleId === undefined ? -1 : scene.look.shellStyleId)
                property real density: Number(scene.look.shellDensity || 1)
                property real flow: Number(scene.look.shellFlow || 0)
                property vector3d shellTint: scene.look.shellTint || scene.look.cool
                vertexShader: "qrc:/archie/shaders/lumen_shell.vert"
                fragmentShader: "qrc:/archie/shaders/lumen_shell.frag"
            }
        }

        LineLayer { name: "membraneEdges"; tint: scene.look.cool; alpha: scene.look.shellOpacity * 0.18; visible: Number(scene.look.shellStyleId) === 0 }

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
        // The content sphere's links are its filaments: warm and lit (cyan under the hologram's grade).
        LineLayer { name: "backbone"; tint: scene.look.content ? scene.look.warm : Qt.vector3d(0.26, 0.38, 0.39)
                    alpha: Math.min(1, scene.look.linkOpacity * (scene.look.content ? 3.0 : 4)); intensity: scene.look.content ? 2.0 : 1.0
                    visible: scene.look.links }
        LineLayer { name: "fibres"; tint: scene.look.cool; alpha: 0.14; visible: scene.look.links }
        LineLayer { name: "observed"; tint: scene.look.warm; alpha: 0.4; intensity: 1.8; visible: scene.look.links }
        LineLayer { name: "semantic"; tint: scene.look.cool; alpha: 0.22; intensity: 1.6; visible: scene.look.links }
        LineLayer { name: "guides"; tint: scene.look.warm; alpha: 0.15; visible: scene.look.guides }
        LineLayer { name: "highlight"; tint: scene.look.warm; alpha: scene.look.highlightOpacity; intensity: 1.8 }
        LineLayer { name: "pulseRibbons"; tint: scene.look.warm; alpha: 0.8; visible: Number(scene.look.linkStyleId || 0)>0 && scene.look.pulses && Number(scene.look.pulseStyleId || 0)>0 }

        // --- Satellites: small beads around each module ---
        Model {
            geometry: LumenGeometry { id: satelliteMesh }
            instancing: LumenInstancing { id: satelliteTable }
            Component.onCompleted: { scene.controller.bindLayer("satelliteMesh", satelliteMesh); scene.controller.bindLayer("satellites", satelliteTable) }
            materials: OrbMaterial { facet: 0.2; shapeAmount: 0.12; detail: 0.0 }
        }

        // --- Orbs: modules are cut gems, entities polished beads; neither is a plain ball ---
        Model {
            geometry: scene.look.folderStyleId > 0 && scene.look.folderStyleId < 8 ? folderDesignMesh : orbMesh
            LumenGeometry { id: orbMesh }
            XaviGeometry { id: folderDesignMesh; style: Number(scene.look.folderStyleId || 0); folder: true }
            instancing: LumenInstancing { id: moduleTable }
            Component.onCompleted: { scene.controller.bindLayer("orbMesh", orbMesh); scene.controller.bindLayer("modules", moduleTable) }
            materials: scene.look.folderStyleId > 0 ? folderDesignMaterial : originalFolderMaterial
            OrbMaterial { id: originalFolderMaterial; facet: 0.3; shapeAmount: 0.16; detail: 1.0 }
            XaviMaterial { id: folderDesignMaterial; designStyle: Number(scene.look.folderStyleId || 0) }
        }
        Model {
            geometry: scene.look.nodeStyleId > 0 && scene.look.nodeStyleId < 8 ? entityDesignMesh : entityMesh
            LumenGeometry { id: entityMesh }
            XaviGeometry { id: entityDesignMesh; style: Number(scene.look.nodeStyleId || 0) }
            instancing: LumenInstancing { id: entityTable }
            Component.onCompleted: { scene.controller.bindLayer("entityMesh", entityMesh); scene.controller.bindLayer("entities", entityTable) }
            materials: scene.look.nodeStyleId > 0 ? entityDesignMaterial : originalEntityMaterial
            OrbMaterial { id: originalEntityMaterial; facet: 0.15; shapeAmount: 0.1; detail: 0.7 }
            XaviMaterial { id: entityDesignMaterial; designStyle: Number(scene.look.nodeStyleId || 0) }
        }

        // --- Glow: a halo per orb, brighter with activity, focus and selection ---
        Model {
            geometry: LumenGeometry { id: moduleHaloQuad }
            instancing: moduleTable
            Component.onCompleted: scene.controller.bindLayer("quad", moduleHaloQuad)
            // On the content sphere the halos are the beads of light along its filaments.
            materials: SpriteMaterial { grow: scene.look.content ? 2.4 : 2.8; alpha: scene.look.content ? 0.95 : 0.3 + scene.look.bloomStrength * 0.16; sharpness: 1.8; haloGain: 0.9 }
        }
        Model {
            geometry: LumenGeometry { id: entityHaloQuad }
            instancing: entityTable
            Component.onCompleted: scene.controller.bindLayer("quad", entityHaloQuad)
            materials: SpriteMaterial { grow: scene.look.content ? 2.6 : 3.3; alpha: scene.look.content ? 0.95 : 0.28 + scene.look.bloomStrength * 0.16; sharpness: 1.7; haloGain: 1.1 }
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
        geometry: LumenGeometry { id: lines; ribbons: Number(scene.look.linkStyleId || 0) > 0 && layer.name !== "guides" && !layer.name.startsWith("holo") }
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
            property int linkStyle: Number(scene.look.linkStyleId || 0)
            property int pulseStyle: layer.name === "pulseRibbons" && scene.look.pulses ? Number(scene.look.pulseStyleId || 0) : 0
            property int bloomMode: Number(scene.look.bloomModeId || 0)
            property real time: scene.controller.time
            readonly property bool relationFlow: !!scene.controller.selection.id || !!scene.controller.hovered.id
            property real pulseSpeed: Number(scene.controller.settings.pulseSpeed || 0) * Number((relationFlow ? scene.controller.settings.relationFlowSpeed : scene.controller.settings.ambientFlowSpeed) || 0)
            property real pulseIntensity: Number(scene.look.energyIntensity || 0) * Number((relationFlow ? scene.controller.settings.relationFlowIntensity : scene.controller.settings.ambientFlowIntensity) || 0)
            property bool activeLink: layer.name === "highlight" || layer.name === "pulseRibbons" || layer.name === "observed" || layer.name === "semantic"
            property real focusWeight: layer.name === "highlight" ? 1 : 0
            property real lineWidth: Number(scene.look.lineWidthPx || 1)
            property vector2d viewportSize: Qt.vector2d(Math.max(1,scene.viewportWidth),Math.max(1,scene.viewportHeight))
            property vector3d warm: scene.look.warm
            property vector3d cool: scene.look.cool
            property vector3d warmPosition: scene.look.warmPosition
            property vector3d coolPosition: scene.look.coolPosition
            property real fieldRadius: scene.controller.fieldRadius
            vertexShader: lines.ribbons ? "qrc:/archie/shaders/xavi_ribbon.vert" : "qrc:/archie/shaders/lumen_line.vert"
            fragmentShader: lines.ribbons ? "qrc:/archie/shaders/xavi_ribbon.frag" : "qrc:/archie/shaders/lumen_line.frag"
        }
    }

    component GlowBall: Model {
        property vector3d tint: Qt.vector3d(1, 1, 1)
        property real gain: 1.0
        property real alpha: 1.0
        property real sharpness: 4.0
        id: ball
        visible: Number(scene.look.coreGlow || 0) > 0
        source: "#Sphere"
        materials: CustomMaterial {
            shadingMode: CustomMaterial.Unshaded
            sourceBlend: CustomMaterial.SrcAlpha
            destinationBlend: CustomMaterial.One
            cullMode: Material.BackFaceCulling
            depthDrawMode: Material.NeverDepthDraw
            property vector3d tint: ball.tint
            property real gain: ball.gain
            property real alpha: ball.alpha
            property real sharpness: ball.sharpness
            vertexShader: "qrc:/archie/shaders/lumen_glow.vert"
            fragmentShader: "qrc:/archie/shaders/lumen_glow.frag"
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

    // The Qt/GPU boundary is intentional. Source material policy, packed maps and
    // meshes are Rust; these properties only upload them to the existing renderer.
    component XaviMaterial: CustomMaterial {
        property int designStyle: 0
        property real time: scene.controller.time
        property vector3d warm: scene.look.warm
        property vector3d cool: scene.look.cool
        property vector3d warmPosition: scene.look.warmPosition
        property vector3d coolPosition: scene.look.coolPosition
        property real fieldRadius: scene.controller.fieldRadius
        property real metalness: scene.look.metalness
        property real roughness: scene.look.roughness
        property real emissive: scene.look.emissive
        property real textureDepth: scene.look.textureDepth
        property real textureContrast: scene.look.textureContrast
        property real poleMix: Number(scene.look.poleMix || 0)
        property real glowMaskIntensity: Number(scene.look.glowMaskIntensity || 0)
        property real detailFrequency: Number(scene.look.detailFrequency || 1)
        property int bloomMode: Number(scene.look.bloomModeId || 0)
        property bool selectionActive: !!scene.controller.selection.id
        property real focusDim: Number(scene.controller.settings.focusDim || 0)
        property real viewportHeight: scene.viewportHeight
        property real hybridReady: designStyle > 0 && hybridStructureData.resolution > 0 ? 1 : 0
        property vector4d hybridParameters: sourceSurface.hybridParameters(designStyle,textureDepth,emissive,textureContrast)
        readonly property real projectedMaximum: scene.viewportHeight * 2.75 * Number(scene.look.maximumNodeRadius || 4) / Math.max(0.2,scene.distance-scene.controller.fieldRadius)
        readonly property int hybridResolution: hybridStructureData.chooseResolution(
            scene.controller.settings.visualTextureTier === "procedural" ? 0 : Number(scene.look.textureResolution || 0),
            Number(scene.look.qualityId || 0),projectedMaximum)
        property TextureInput albedoMap: TextureInput {
            texture: Texture {
                textureData: XaviSurfaceData { style: designStyle; frequency: detailFrequency; resolution: Math.min(1024, Math.max(32, Number(scene.look.textureResolution || 512))) }
                generateMipmaps: true
                minFilter: Texture.Linear
                mipFilter: Texture.Linear
                tilingModeHorizontal: Texture.Repeat
                tilingModeVertical: Texture.Repeat
            }
        }
        property TextureInput surfaceMap: TextureInput {
            texture: Texture {
                textureData: XaviSurfaceData { id: sourceSurface; style: designStyle; frequency: detailFrequency; surface: true; resolution: Math.min(1024, Math.max(32, Number(scene.look.textureResolution || 512))) }
                generateMipmaps: true
                minFilter: Texture.Linear
                mipFilter: Texture.Linear
                tilingModeHorizontal: Texture.Repeat
                tilingModeVertical: Texture.Repeat
            }
        }
        property TextureInput hybridStructure: TextureInput {
            texture: Texture {
                textureData: XaviHybridData { id: hybridStructureData; enabled: designStyle > 0; resolution: hybridResolution }
                generateMipmaps: true; minFilter: Texture.Linear; mipFilter: Texture.Linear
                tilingModeHorizontal: Texture.Repeat; tilingModeVertical: Texture.Repeat
            }
        }
        property TextureInput hybridSurface: TextureInput {
            texture: Texture {
                textureData: XaviHybridData { enabled: designStyle > 0; surface: true; resolution: hybridResolution }
                generateMipmaps: true; minFilter: Texture.Linear; mipFilter: Texture.Linear
                tilingModeHorizontal: Texture.Repeat; tilingModeVertical: Texture.Repeat
            }
        }
        shadingMode: CustomMaterial.Unshaded
        cullMode: Material.NoCulling
        vertexShader: "qrc:/archie/shaders/xavi_surface.vert"
        fragmentShader: "qrc:/archie/shaders/xavi_surface.frag"
    }
}
