import QtQuick
import QtQuick3D
import QtQuick3D.Helpers
import Archie.Observatory

// Shared by the shell and native renderer: one controller, one active scene.
Item {
    id: viewport
    required property var controller
    required property var bridge
    property bool navigationEnabled: true
    property bool hudVisible: true
    property bool reducedMotion: false
    property color backgroundColor: controller.look.background
    property color textColor: "#e9edf3"
    property color secondaryColor: "#c9ced8"
    property color mutedColor: "#8b93a1"
    property color errorColor: "#c98a8a"
    property int frameIntervalMs: Number(bridge.frame.frameIntervalMs === undefined ? 16 : bridge.frame.frameIntervalMs)
    readonly property bool presented: viewport.visible && bridge.presented && viewport.width > 0 && viewport.height > 0
    property bool ownsController: false

    Component.onCompleted: ownsController = controller.acquireView(viewport)
    Component.onDestruction: controller.releaseView(viewport)
    Binding { target: controller; property: "active"; value: viewport.presented; when: viewport.ownsController }
    Binding { target: controller; property: "renderBudget"; value: bridge.frame; when: viewport.ownsController }
    // renderBudget carries frameIntervalMs; the controller takes it from there alone.
    Binding { target: controller; property: "reducedMotion"; value: viewport.reducedMotion; when: viewport.ownsController }

    View3D {
        id: view
        anchors.fill: parent
        visible: viewport.ownsController && viewport.presented && viewport.frameIntervalMs > 0
        renderMode: View3D.Offscreen
        camera: scene.camera
        environment: ExtendedSceneEnvironment {
            backgroundMode: SceneEnvironment.Color
            clearColor: viewport.backgroundColor
            antialiasingMode: Number(viewport.bridge.frame.msaaSamples || 0) > 0
                && viewport.controller.scene.profile !== "low" ? SceneEnvironment.MSAA : SceneEnvironment.NoAA
            antialiasingQuality: Number(viewport.bridge.frame.msaaSamples || 0) >= 4
                ? SceneEnvironment.VeryHigh : SceneEnvironment.High
            tonemapMode: SceneEnvironment.TonemapModeFilmic
            exposure: viewport.controller.look.exposure
            glowEnabled: !!viewport.bridge.frame.postProcessing && viewport.controller.look.bloom
            glowStrength: 0.9 + viewport.controller.look.bloomRadius * 0.6
            glowIntensity: viewport.controller.look.bloomStrength * 1.3
            glowBloom: viewport.controller.look.bloomRadius * 0.5
            glowHDRMinimumValue: viewport.controller.look.bloomThreshold
            glowHDRMaximumValue: 8.0
            glowHDRScale: 2.0
            glowQualityHigh: Number(viewport.bridge.frame.msaaSamples || 0) >= 4
        }
        LumenScene {
            id: scene
            controller: viewport.controller
            aspect: view.width / Math.max(1, view.height)
            viewportWidth: view.width
            viewportHeight: view.height
            reducedMotion: viewport.reducedMotion || !viewport.controller.renderBudget.ambientMotion
            presented: viewport.presented
        }
    }
    OrbHud {
        anchors.fill: parent
        controller: viewport.controller
        view: view
        orbScene: scene
        navigationEnabled: viewport.navigationEnabled
        labelsVisible: viewport.hudVisible
        textColor:viewport.textColor
        secondaryColor:viewport.secondaryColor
        mutedColor:viewport.mutedColor
        errorColor:viewport.errorColor
        visible: viewport.ownsController && view.visible
    }
}
