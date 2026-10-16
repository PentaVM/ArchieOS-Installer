import QtQuick

// The native host provides the existing controller and budget bridge. Plain QML
// previews keep their desktop background without importing native types.
Item {
    id: lv
    readonly property var nativeController: typeof lumen === "undefined" ? null : lumen
    readonly property var nativeBridge: typeof archieBridge === "undefined" ? null : archieBridge
    readonly property bool nativeAvailable: nativeController !== null && nativeBridge !== null
    readonly property bool loaded: viewport.status === Loader.Ready
    readonly property bool ok: nativeAvailable && loaded
    readonly property string diagnostics: JSON.stringify({native:nativeAvailable,loaded:loaded,
        viewportPresented:viewport.item ? viewport.item.presented : false,
        interval:viewport.item ? viewport.item.frameIntervalMs : -1,
        owned:viewport.item ? viewport.item.ownsController : false,
        controllerActive:nativeController ? nativeController.active : false,
        hasScene:nativeController ? nativeController.loaded : false})
    property bool navigationEnabled: Demo.orbNav
    property bool hudVisible: typeof Demo.orbHud === "boolean" ? Demo.orbHud : true
    property bool reducedMotion: Theme.reducedMotion || !Theme.spatial
    property color backgroundColor: Theme.desktop
    property var originalGrade: ({})

    function loadNative() {
        if (!nativeAvailable) return
        viewport.setSource("qrc:/qt/qml/Archie/Observatory/qml/OrbViewport.qml", {
            controller: nativeController, bridge: nativeBridge
        })
    }
    function applyProfile() {
        if (!nativeAvailable) return
        const profiles = Demo.orbProfiles
        nativeController.applyProfile(profiles[Math.max(0, Math.min(profiles.length - 1, Demo.orbProfile))])
        originalGrade={}
        applyLook()
    }
    function applyLook() {
        if (!nativeAvailable) return
        nativeController.setSetting("exposure", [1.6, 2.2, 2.8][Demo.orbBrightI])
        nativeController.setSetting("pulses", Demo.orbActI > 0)
        nativeController.setSetting("pulseSpeed", [0, 0.2, 0.33, 0.7][Demo.orbActI])
        nativeController.setSetting("rotate", Demo.orbSpin)
        nativeController.setSetting("visibleLimit", [60,120,220,360,420][Demo.orbNodesI])
        nativeController.setSetting("ambient", Demo.orbDust)
        if (!Object.keys(originalGrade).length) {
            var settings=nativeController.settings, saved={}
            for (var key in settings)
                if (key.indexOf("color")===0 || key==="accentWarm" || key==="accentCool") saved[key]=settings[key]
            originalGrade=saved
        }
        var cold=["#40bfe5","#71aeee","#41c9b8","#83bfd6","#9da8ef","#58c8e9","#78b9e7","#73bbc5"]
        var vivid=["#00b5e8","#e0a500","#00c58f","#9bd225","#b68be8","#f07764","#6499ee","#86c5c0"], i=0
        for (var key in originalGrade) {
            var value=originalGrade[key]
            if (Demo.orbGradeI===1) value=key==="accentWarm" ? "#71c5ef" : key==="accentCool" ? "#42cfcf" : cold[(i++)%cold.length]
            else if (Demo.orbGradeI===2) value=key==="accentWarm" ? "#ffba40" : key==="accentCool" ? "#00c9e8" : vivid[(i++)%vivid.length]
            nativeController.setSetting(key,value)
        }
    }
    Component.onCompleted: { loadNative(); applyProfile() }
    onNativeAvailableChanged: loadNative()

    Loader {
        id: viewport
        anchors.fill: parent
        active: lv.nativeAvailable && lv.visible
    }
    Text {
        visible:lv.loaded && lv.nativeController && !lv.nativeController.loaded
        anchors.horizontalCenter:parent.horizontalCenter; anchors.bottom:parent.bottom; anchors.bottomMargin:110
        text:lv.nativeController && lv.nativeController.error ? "Orbe sin lectura del observador" : "Orbe sin señales TFOG recibidas"
        color:Theme.t3; font.family:Theme.font; font.pixelSize:12
    }
    Binding { target: viewport.item; property: "navigationEnabled"; value: lv.navigationEnabled; when: lv.loaded }
    Binding { target: viewport.item; property: "hudVisible"; value: lv.hudVisible; when: lv.loaded }
    Binding { target: viewport.item; property: "reducedMotion"; value: lv.reducedMotion; when: lv.loaded }
    Binding { target: viewport.item; property: "backgroundColor"; value: lv.backgroundColor; when: lv.loaded }
    Binding { target: viewport.item; property: "textColor"; value: Theme.t1; when: lv.loaded }
    Binding { target: viewport.item; property: "secondaryColor"; value: Theme.t2; when: lv.loaded }
    Binding { target: viewport.item; property: "mutedColor"; value: Theme.t3; when: lv.loaded }
    Binding { target: viewport.item; property: "errorColor"; value: Theme.bad; when: lv.loaded }
    Connections {
        target: Demo
        function onOrbProfileChanged() { lv.applyProfile() }
        function onOrbBrightIChanged() { lv.applyLook() }
        function onOrbActIChanged() { lv.applyLook() }
        function onOrbSpinChanged() { lv.applyLook() }
        function onOrbNodesIChanged() { lv.applyLook() }
        function onOrbDustChanged() { lv.applyLook() }
        function onOrbGradeIChanged() { lv.applyLook() }
    }
}
