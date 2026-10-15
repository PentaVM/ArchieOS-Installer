import QtQuick
import QtTest

Item {
    Component { id:shell; ShellWindow { visible:false } }
    TestCase {
        name:"ProfileRouting"; when:windowShown
        function test_ipc_preset_keeps_the_next_user_selection_effective() {
            var window=createTemporaryObject(shell,null)
            verify(window!==null)
            Demo.orbProfile=0
            verify(typeof window.applyVisualProfile==="function","IPC and settings share profile state")
            verify(window.applyVisualProfile("neural"))
            compare(Demo.orbProfile,3)
            Demo.orbProfile=0
            compare(Demo.orbProfile,0)
            verify(!window.applyVisualProfile("execute"))
            compare(Demo.orbProfile,0)
        }
    }
}
