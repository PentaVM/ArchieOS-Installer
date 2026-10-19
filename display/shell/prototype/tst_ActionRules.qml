import QtQuick
import QtTest
import "ActionRules.js" as Rules

TestCase {
    name:"ActionRules"
    function test_visual_panels_are_local() {
        verify(Rules.allowed("overlay.inspector",{}))
        verify(Rules.allowed("app.sentry",{}))
        verify(Rules.allowed("theme.motion",{}))
    }
    function test_missing_file_channel_never_mutates() {
        for (var id of ["vault.cancel","vault.undo","vault.redo","vault.open","file:private.txt","go:2"])
            verify(!Rules.allowed(id,{fileOperationsAvailable:false}),id)
    }
    function test_fake_lock_is_not_a_security_action() {
        verify(!Rules.allowed("overlay.lock",{}))
        verify(Rules.reason("overlay.lock",{}).length>0)
    }
    function test_navigation_needs_native_viewport() {
        verify(!Rules.allowed("orb.nav",{nativeReady:false}))
        verify(Rules.allowed("orb.nav",{nativeReady:true}))
    }
    function test_unknown_action_is_rejected() {
        verify(!Rules.allowed("execute:cmd.exe",{}))
        verify(!Rules.allowed("overlay.arbitrary",{}))
    }
}
