import QtQuick
import QtQuick3D
import QtQuick3D.Helpers
import Archie.Observatory

// The orb when the field source is "neural": Xavi's neural sphere, drawn by NeuralScene,
// finished by the bloom and the hologram pass, labelled by NeuralHud. The folder it draws is
// the controller's content root; the controller keeps the presets and the contract.
Item {
    id: viewport
    required property var controller
    property bool presented: viewport.visible
    property bool hudVisible: true
    property bool reducedMotion: false
    /// The original's deterministic frame, for captures and the SSIM reference.
    property bool still: false
    property alias field: neural
    property alias sphereScene: scene
    property alias view: view

    NeuralField {
        id: neural
        root: viewport.controller.contentRoot
        still: viewport.still
        limit: Number(viewport.controller.settings.contentLimit || 2500)
        filesPerFolder: Number(viewport.controller.settings.contentFiles || 30)
    }

    // Look presets drive only what they name; the sphere keeps the original's defaults.
    Connections {
        target: viewport.controller
        function onSettingsChanged() {
            const s = viewport.controller.settings
            if (s.background) neural.setBackground(String(s.background))
        }
    }

    View3D {
        id: view
        anchors.fill: parent
        renderMode: View3D.Offscreen
        camera: scene.camera
        environment: ExtendedSceneEnvironment {
            backgroundMode: SceneEnvironment.Color
            clearColor: neural.background
            // Thousands of one-pixel additive points: multisampling would change how much
            // light each adds, so it stays off, as the original rendered.
            antialiasingMode: SceneEnvironment.NoAA
            tonemapMode: SceneEnvironment.TonemapModeLinear
            // UnrealBloomPass(strength 0.6 x glow, radius 0.45, threshold 0.08)
            glowEnabled: Number(neural.settings.glow === undefined ? 1 : neural.settings.glow) > 0
            glowIntensity: 0.6 * Number(neural.settings.glow === undefined ? 1 : neural.settings.glow)
            glowBloom: 0.45
            glowStrength: 1.0
            glowHDRMinimumValue: 0.08
            glowHDRMaximumValue: 8.0
            glowHDRScale: 2.0
            glowQualityHigh: true
        }
        layer.enabled: true
        layer.effect: NeuralHolo { field: neural; pixelRatio: Screen.devicePixelRatio }

        NeuralScene {
            id: scene
            field: neural
            pixelRatio: Screen.devicePixelRatio
            running: viewport.presented
            reducedMotion: viewport.reducedMotion
        }
    }

    NeuralHud {
        anchors.fill: parent
        field: neural
        view: view
        labelsVisible: viewport.hudVisible
        rootLabel: neural.root
    }

    // The hands: drag turns, the wheel zooms, a click selects and opens a folded program,
    // a double click returns to HOME.
    MouseArea {
        anchors.fill: parent
        property point previous
        property bool moved: false
        onPressed: function(mouse) { previous = Qt.point(mouse.x, mouse.y); moved = false }
        onPositionChanged: function(mouse) {
            if (!pressed) return
            if (Math.abs(mouse.x - previous.x) + Math.abs(mouse.y - previous.y) > 0) moved = true
            scene.orbit(-(mouse.x - previous.x) * 0.4, (mouse.y - previous.y) * 0.4)
            previous = Qt.point(mouse.x, mouse.y)
        }
        onWheel: function(wheel) { scene.zoom(wheel.angleDelta.y > 0 ? 0.92 : 1 / 0.92); wheel.accepted = true }
        onDoubleClicked: scene.fit()
        onClicked: function(mouse) {
            if (moved) return
            const near = view.mapTo3DScene(Qt.vector3d(mouse.x, mouse.y, 0))
            const far = view.mapTo3DScene(Qt.vector3d(mouse.x, mouse.y, 1))
            const direction = Qt.vector3d(far.x - near.x, far.y - near.y, far.z - near.z).normalized()
            const id = neural.pick(scene.camera.position, direction, 3.0)
            if (id.length > 0) neural.activate(id)
        }
    }
}
