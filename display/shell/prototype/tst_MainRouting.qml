import QtQuick
import QtTest

Item {
    id:host; width:1920; height:1080
    Main { id:desktop; width:parent.width; height:parent.height }
    TestCase {
        name:"MainRouting"
        when:windowShown
        function test_escape_stops_navigation() {
            Demo.orbNav=true; desktop.overlay="inspector"
            verify(typeof desktop.handleEscape === "function","Escape has one shared handler")
            desktop.handleEscape()
            compare(Demo.orbNav,false)
            compare(desktop.overlay,"")
        }
        function test_file_effects_are_not_reported_done() {
            compare(desktop.run("vault.undo"),false)
            verify(desktop.actionNote.length>0)
        }
        function test_local_inspector_route_works_offline() {
            compare(desktop.run("overlay.inspector"),true)
            compare(desktop.overlay,"inspector")
        }
        function test_lock_sample_is_not_security_control() {
            compare(desktop.run("overlay.lock"),false)
            verify(desktop.overlay!=="lock")
        }
        function test_recording_cannot_claim_live() {
            verify(typeof desktop.ingestRecording === "function","recording has a provenance boundary")
            verify(desktop.ingestRecording(JSON.stringify({schema:1,at_ms:1,link:{state:"live"},system:null,kernel:null})))
            compare(desktop.bridge.link,"fixture")
        }
    }
}
