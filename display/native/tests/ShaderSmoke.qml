import QtQuick
import QtQuick3D
import "../qml" as Native

// Synthetic projection for shader execution only; no daemon or model is started.
Item {
    width: 800
    height: 600
    View3D {
        anchors.fill: parent
        environment: SceneEnvironment {
            clearColor: "#0d1415"
            backgroundMode: SceneEnvironment.Color
            antialiasingMode: SceneEnvironment.NoAA
        }
        camera: orb.camera
        Native.LumenScene {
            id: orb
            controller: shaderTestField
            reducedMotion: true
            presented: true
            viewportWidth: 800
            viewportHeight: 600
            distance: 260
            aspect: 4 / 3
        }
    }
}
