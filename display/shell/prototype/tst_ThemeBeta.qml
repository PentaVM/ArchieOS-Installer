import QtQuick
import QtTest

Item {
    width:1200; height:800
    SettingsWindow { id:settings; section:5; width:940; height:650 }
    TestCase {
        name:"ThemeBeta"; when:windowShown
        function cleanup() { if (typeof Theme.colorMode === "string") Theme.colorMode="dark"; Theme.runtimeConserve=false; Theme.reducedMotion=false }
        function channel(v) { return v<=0.04045 ? v/12.92 : Math.pow((v+0.055)/1.055,2.4) }
        function luminance(c) { return channel(c.r)*0.2126+channel(c.g)*0.7152+channel(c.b)*0.0722 }
        function contrast(a,b) { var x=luminance(a),y=luminance(b); return (Math.max(x,y)+0.05)/(Math.min(x,y)+0.05) }
        function test_theme_selector_reuses_panels() {
            var selector=findChild(settings,"themeMode")
            verify(selector!==null,"Appearance needs the beta selector")
            selector.picked(1)
            compare(Theme.colorMode,"light")
            verify(Theme.light)
            selector.picked(0)
            compare(Theme.colorMode,"dark")
        }
        function test_light_text_has_contrast_on_solid_surfaces() {
            verify(typeof Theme.colorMode === "string","Theme needs one centralized color mode")
            Theme.colorMode="light"
            for (var bg of [Theme.desktop,Theme.win,Theme.panel,Theme.side])
                for (var fg of [Theme.t1,Theme.t2,Theme.t3]) verify(contrast(fg,bg)>=4.5,"Readable text: " + fg + " on " + bg)
            verify(contrast(Theme.onAccent,Theme.acc)>=4.5)
        }
        function test_resource_conservation_keeps_user_motion_choice() {
            Theme.motion="full"; Theme.runtimeConserve=true
            compare(Theme.profile,"quiet")
            compare(Theme.motion,"full")
            Theme.reducedMotion=true
            compare(Theme.profile,"reduced")
            Theme.runtimeConserve=false; Theme.reducedMotion=false
            compare(Theme.profile,"full")
        }
    }
}
