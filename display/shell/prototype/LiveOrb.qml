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
    // Settings > Graph viewer on top of the engine defaults: only what the person changes; the rest stays as the program draws it.
    function settings(extra) {
        var t = {}
        t.sphereBrightness = [0.75, 1, 1.4][Demo.orbBrightI]
        t.pulses = Demo.orbActI > 0 ? "true" : "false"
        t.brainRate = [0, 2, 5, 10][Demo.orbActI]
        t.rotationSpeed = Demo.orbSpin ? 0.3 : 0
        var d = Demo.orbBrightI === 1 && Demo.orbActI === 2 && Demo.orbSpin
        // Vivid system colours: black backdrop and less blue haze behind the sphere.
        var v = { background: "#000000", sphereBackdropGlow: 0.3 }
        return "(function(){var D=window.__orbDef=window.__orbDef||{};function a(k,v){var e=document.querySelector('#controls [data-setting=\"'+k+'\"]');if(!e)return;" +
               "if(!(k in D))D[k]=(e.type==='checkbox')?String(e.checked):e.value;if(e.type==='checkbox'){e.checked=(String(v)==='true');}else e.value=String(v);" +
               "e.dispatchEvent(new Event(e.tagName==='SELECT'?'change':'input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));}" +
               (extra || "") + "var T=" + JSON.stringify(t) + ";var V=" + JSON.stringify(v) + ";var def=" + (d ? "true" : "false") + ";var viv=" + (Theme.vivid ? "true" : "false") +
               ";setTimeout(function(){for(var k in T){if(def&&!(k in D))continue;a(k,def&&(k in D)?D[k]:T[k]);}" +
               "for(var k in V){if(!viv&&!(k in D))continue;a(k,viv?V[k]:D[k]);}},1500);})()"
    }
    function applyProfile() {
        view.runJavaScript(settings("var b=document.querySelector('[data-dock-profile=\"" + files[Demo.orbProfile] + "\"]');if(b)b.click();"))
    }
    function applyLook() { view.runJavaScript(settings("")) }

    Connections { target: Theme
        function onVividChanged() { lv.applyLook() } }
    Connections { target: Demo
        function onOrbNavChanged() { lv.desk() }
        function onOrbProfileChanged() { lv.applyProfile() }
        function onOrbBrightIChanged() { lv.applyLook() }
        function onOrbActIChanged() { lv.applyLook() }
        function onOrbSpinChanged() { lv.applyLook() }
    }

    WebEngineView {
        id: view
        anchors.fill: parent
        url: lv.base
        backgroundColor: "#000000"
        visible: lv.loaded
        onLoadingChanged: function (info) {
            if (info.status === WebEngineLoadingInfo.LoadSucceededStatus) { lv.desk(); lv.applyProfile(); lv.loaded = true }
        }
    }
}
