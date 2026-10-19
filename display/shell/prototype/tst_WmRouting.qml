import QtQuick
import QtTest
import "ActionRules.js" as Rules

Item {
    id: host
    width:1920; height:1080
    TestCase {
        name:"WmRouting"
        when:windowShown
        function init() { failOnWarning(/.*/); Wm.closeAll(); Demo.bare=true }
        function cleanup() { Wm.closeAll(); Demo.bare=true }
        function test_settings_fits_a_small_desktop() {
            var d=desktop(); d.width=1370; d.height=749
            verify(d.run("app.set"))
            var window=Wm.hosts.set
            verify(window!==undefined)
            verify(window.x>=0 && window.x+window.width<=d.width,"Settings remains horizontally reachable")
            verify(window.y>=Theme.barH && window.y+window.height<=d.height-90,"Settings stays above the dock")
        }
        function textIn(item) {
            var text=typeof item.text==="string" ? item.text : ""
            for(var i=0;i<item.children.length;i++) text+=" " + textIn(item.children[i])
            return text
        }
        function desktop() {
            var component=Qt.createComponent(Qt.resolvedUrl("Main.qml"))
            verify(component.status===Component.Ready,component.errorString())
            var view=createTemporaryObject(component,host)
            verify(view!==null)
            return view
        }
        function test_local_window_commands_are_allowed_without_backend_authority() {
            for (var id of ["dock.vault","dock.forge","dock.sentry","dock.stage","dock.set","window.stage","win.close","win.min","win.zoom","win.closeAll","win.minAll","win.showAll","win.desktop"])
                verify(Rules.allowed(id,{}),id)
            for (var bad of ["dock.unknown","window.arbitrary","win.execute","file:secret","go:3","vault.undo","overlay.lock"])
                verify(!Rules.allowed(bad,{}),bad)
            verify(!Rules.allowed("app.stage",{stageLauncher:false}))
        }
        function test_local_window_metadata_does_not_claim_private_domain() {
            Wm.open("vault"); Wm.open("sentry")
            compare(Wm.list().filter(function(w){return w.domain==="personal" || w.domain==="work"}).length,0)
        }
        function test_unknown_window_cannot_acquire_drag_state() {
            Wm.moveTo("unknown",20,30)
            compare(Wm.ox("unknown"),0)
        }
        function test_hidden_host_stops_pointer_input_during_fade() {
            var view=Qt.createQmlObject('import QtQuick; WinHost { wid:"vault"; Rectangle { width:100; height:100; opacity:1 } }',host)
            Wm.open("vault"); verify(view.enabled)
            Wm.minimize("vault"); compare(view.enabled,false)
            view.destroy()
        }
        function test_host_deregisters_after_its_view_is_destroyed() {
            var view=Qt.createQmlObject('import QtQuick; WinHost { wid:"vault" }',host)
            compare(Wm.hosts.vault,view)
            view.destroy(); wait(1)
            verify(Wm.hosts.vault===undefined)
        }
        function test_switcher_keeps_local_windows_without_inferring_domain() {
            Wm.open("vault"); Wm.open("sentry")
            var view=Qt.createQmlObject('import QtQuick; WindowSwitcher { st:"error" }',host)
            compare(view.flat.length,2)
            view.destroy()
        }
        function test_stage_local_view_never_claims_core_authority() {
            var view=Qt.createQmlObject('import QtQuick; StageWindow {}',host)
            verify(textIn(view).indexOf("Autoridad: Archie")<0)
            verify(textIn(view).indexOf("Tú ganas el control")<0)
            view.destroy()
        }
        function test_main_opens_managed_backend_views_through_both_routes() {
            var view=desktop()
            verify(view.run("app.forge")); compare(Wm.front(),"forge"); compare(view.overlay,"")
            verify(view.run("overlay.settings")); compare(Wm.front(),"set"); compare(view.overlay,"")
            verify(view.run("overlay.sentry")); compare(Wm.front(),"sentry"); compare(view.overlay,"")
        }
        function test_main_dock_minimizes_and_restores_local_view() {
            var view=desktop()
            verify(view.run("dock.vault")); compare(Wm.front(),"vault")
            verify(view.run("dock.vault")); verify(Wm.isMin("vault"))
            verify(view.run("dock.vault")); verify(Wm.isShown("vault")); compare(Wm.front(),"vault")
            verify(view.run("win.zoom")); verify(Wm.isMax("vault"))
            verify(view.run("win.close")); verify(!Wm.isOpen("vault"))
        }
        function test_stage_information_window_does_not_start_external_application() {
            var view=desktop()
            verify(view.run("window.stage")); compare(Wm.front(),"stage")
            verify(!view.run("app.stage")); compare(Wm.front(),"stage")
        }
        function test_switcher_keyboard_restores_previous_local_window() {
            var view=desktop()
            view.run("app.forge"); view.run("app.set")
            view.overlay="switcher"; view.forceActiveFocus()
            keyClick(Qt.Key_Return)
            compare(view.overlay,""); compare(Wm.front(),"forge")
        }
        function test_escape_and_rejected_authority_do_not_close_windows() {
            var view=desktop(); view.run("app.vault")
            var before=JSON.stringify(Wm.st)
            for(var id of ["overlay.lock","vault.undo","file:private","dock.unknown"])
                verify(!view.run(id),id)
            compare(JSON.stringify(Wm.st),before)
            view.overlay="inspector"; Demo.orbNav=true; view.forceActiveFocus()
            keyClick(Qt.Key_Escape)
            compare(view.overlay,""); verify(!Demo.orbNav); compare(Wm.front(),"vault")
        }
        function test_original_wm_state_machine_stays_intact() {
            var component=Qt.createComponent(Qt.resolvedUrl("WmTest.qml"))
            verify(component.status===Component.Ready,component.errorString())
            var result=createTemporaryObject(component,host)
            verify(result!==null)
            compare(result.failures,0,result.log.join(" | "))
            verify(result.passes>30)
        }
    }
}
