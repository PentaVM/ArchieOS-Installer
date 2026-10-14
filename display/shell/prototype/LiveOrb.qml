import QtQuick
import QtWebEngine

// Graph viewer on the desktop: the Archie Lumenfield 0449 sphere (its own WebGL engine, on the GPU)
// served by display/visor over the saved graph of the OS content. Only the sphere is shown: no
// program panels, just the floating card of the picked node. The camera is fixed on the desktop;
// Demo.orbNav (Ctrl+Space) frees it to navigate in 3D. The viewer writes its URL to /srv/visor/url.txt.
Item {
    id: lv
    property string base: ""
    readonly property bool ok: base !== "" && loaded
    property bool loaded: false
    readonly property var files: ["sphere", "orb", "neural"]
    function readUrl() {
        var x = new XMLHttpRequest()
        x.onreadystatechange = function () {
            if (x.readyState !== XMLHttpRequest.DONE) return
            var t = (x.responseText || "").trim()
            if (t.indexOf("http://127.0.0.1:") === 0) lv.base = t
        }
        x.open("GET", "file:///srv/visor/url.txt"); x.send()
    }
    Timer { interval: 1500; repeat: true; running: lv.base === ""; triggeredOnStart: true; onTriggered: lv.readUrl() }

    // Desktop: no program panels; only the floating card of the picked node and the hover hint stay.
    readonly property string deskCss: "#rail,.rail-tabs,.topbar,.minimap,#minimap,.dock-group,.brand,.sphere-legend,.toast,#toasts{display:none!important}" +
        ".workspace{position:fixed!important;inset:0!important;width:100vw!important;height:100vh!important;margin:0!important}" +
        "#inspector{position:fixed!important;right:16px!important;top:46px!important;width:340px!important;max-height:68vh!important;overflow:auto!important;z-index:30;background:rgba(4,12,18,.9)!important;border:1px solid rgba(95,212,255,.35)!important;border-radius:12px!important}" +
        "#inspector>*:not(#selected-section){display:none!important}#inspector:has(.selection-empty){display:none!important}"

    function desk() {
        view.runJavaScript("(function(){var s=document.getElementById('archie-desk');if(!s){s=document.createElement('style');s.id='archie-desk';s.textContent=" +
                           JSON.stringify(deskCss) + ";document.head.appendChild(s);}s.disabled=false;var c=window.__archie&&window.__archie.scene&&window.__archie.scene.controls;if(c){c.enableRotate=" + (Demo.orbNav ? "true" : "false") +
                           ";c.enableZoom=" + (Demo.orbNav ? "true" : "false") + ";c.enablePan=" + (Demo.orbNav ? "true" : "false") + ";}window.dispatchEvent(new Event('resize'));})()")
    }
    // Colour grade laid over the sphere's canvas by the shell (CSS only; the sphere itself is not touched).
    // The engine draws a lot of warm, pale lines that read as a grey, dusty veil on a dark desktop.
    //   cold hologram: hue turned toward cyan, as in the approved base look, with more saturation and contrast.
    //   vivid: same hues, more saturation and contrast.
    readonly property var grades: ["", "hue-rotate(168deg) saturate(1.7) contrast(1.15) brightness(1.05)", "saturate(1.8) contrast(1.15) brightness(1.04)"]
    function applyGrade() {
        var css = grades[Demo.orbGradeI] ? "canvas{filter:" + grades[Demo.orbGradeI] + "!important}" : ""
        view.runJavaScript("(function(){var s=document.getElementById('archie-grade');if(!s){s=document.createElement('style');s.id='archie-grade';document.head.appendChild(s);}s.textContent=" + JSON.stringify(css) + ";})()")
    }
    // Settings > Graph viewer. The engine's own controls are driven (the same events a click would send), once they exist:
    //   U  what the person chose here (always applied);
    //   K  the look that makes it generate and read well with many files: more nodes, no dust of light,
    //      exposure back to 1 (1.5 greys the black), a little more node and halo size; skipped on "Original";
    //   V  vivid system colours: black backdrop and less blue haze behind the sphere.
    // The engine's values are remembered in D the first time a control is touched, and put back when a choice is undone.
    function settings(extra) {
        var u = {}
        u.sphereBudget = [60, 220, 800, 2500, 5000][Demo.orbNodesI]
        u.sphereFiles = [6, 10, 20, 30, 40][Demo.orbNodesI]
        u.ambient = Demo.orbDust ? "true" : "false"
        u.sphereBrightness = [0.85, 1.15, 1.5][Demo.orbBrightI]
        u.pulses = Demo.orbActI > 0 ? "true" : "false"
        u.brainRate = [0, 1.5, 3, 6][Demo.orbActI]
        u.rotationSpeed = Demo.orbSpin ? 0.3 : 0
        var k = { exposure: 1.0, nodeSize: 1.5, sphereGlow: 1.2 }
        var v = { background: "#000000", sphereBackdropGlow: 0.3 }
        return "(function(){var D=window.__orbDef=window.__orbDef||{};function a(k,v){var e=document.querySelector('#controls [data-setting=\"'+k+'\"]');if(!e)return;" +
               "if(!(k in D))D[k]=(e.type==='checkbox')?String(e.checked):e.value;if(e.type==='checkbox'){e.checked=(String(v)==='true');}else e.value=String(v);" +
               "e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));}" +
               "var U=" + JSON.stringify(u) + ",K=" + JSON.stringify(k) + ",V=" + JSON.stringify(v) + ",look=" + (Demo.orbGradeI > 0 ? "true" : "false") + ",viv=" + (Theme.vivid ? "true" : "false") + ";" +
               "var tries=0;(function go(){if(!document.querySelector('#controls [data-setting=\"pulses\"]')){if(tries++<80)setTimeout(go,500);return;}" +
               (extra || "") + "setTimeout(function(){" +
               "for(var x in U)a(x,U[x]);" +
               "for(var x in K){if(!look&&!(x in D))continue;a(x,look?K[x]:D[x]);}" +
               "for(var x in V){if(!viv&&!(x in D))continue;a(x,viv?V[x]:D[x]);}},400);})();})()"
    }
    function applyProfile() {
        view.runJavaScript(settings("var b=document.querySelector('[data-dock-profile=\"" + files[Demo.orbProfile] + "\"]');if(b)b.click();"))
    }
    function applyLook() { view.runJavaScript(settings("")); applyGrade() }

    Connections { target: Theme
        function onVividChanged() { lv.applyLook() } }
    Connections { target: Demo
        function onOrbNavChanged() { lv.desk() }
        function onOrbProfileChanged() { lv.applyProfile() }
        function onOrbBrightIChanged() { lv.applyLook() }
        function onOrbActIChanged() { lv.applyLook() }
        function onOrbSpinChanged() { lv.applyLook() }
        function onOrbNodesIChanged() { lv.applyLook() }
        function onOrbDustChanged() { lv.applyLook() }
        function onOrbGradeIChanged() { lv.applyLook() }
    }

    // The page is a picture and a pointer target on the desktop: the keyboard belongs to the shell. The page
    // asks for focus on its own once it has loaded (and on a click), which would swallow every shortcut, so
    // focus is handed straight back to the item that holds the shell's keys.
    function giveBackFocus() { if (lv.parent) lv.parent.forceActiveFocus() }

    WebEngineView {
        id: view
        anchors.fill: parent
        url: lv.base
        backgroundColor: "#000000"
        visible: lv.loaded
        activeFocusOnPress: false
        onActiveFocusChanged: if (activeFocus) Qt.callLater(lv.giveBackFocus)
        onLoadingChanged: function (info) {
            if (info.status === WebEngineLoadingInfo.LoadSucceededStatus) { lv.desk(); lv.applyGrade(); lv.applyProfile(); lv.loaded = true }
        }
    }
}
