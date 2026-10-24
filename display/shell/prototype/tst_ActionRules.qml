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
    function test_vault_reads_but_never_mutates() {
        for (var id of ["vault.cancel","vault.undo","vault.redo","vault.delete","vault.move","file:private.txt","go:6","go:-1","go:x"])
            verify(!Rules.allowed(id,{}),id)
        for (var ok of ["vault.open","vault.up","vault.newTab","vault.sort","go:0","go:5"])
            verify(Rules.allowed(ok,{}),ok)
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
