import QtQuick
import QtTest

// Every option of the orb's and the appearance's settings, clicked through Qt's input path:
// each one becomes the picked one, read back from the state it sets (none is a picture of a
// choice). The shell's UI walk stopped at the first rows that fit on screen; this covers all.
Item {
    width: 1200; height: 900
    SettingsWindow { id: settings; shown: true; width: 940; height: 860 }
    TestCase {
        name: "SettingsOptions"; when: windowShown
        property var saved
        function initTestCase() {
            saved = { show: Demo.orbShow, profile: Demo.orbProfile, nodes: Demo.orbNodesI, grade: Demo.orbGradeI,
                      dust: Demo.orbDust, bright: Demo.orbBrightI, act: Demo.orbActI, spin: Demo.orbSpin,
                      mode: Theme.colorMode, vivid: Theme.vivid, motion: Theme.motion, opaque: Theme.opaque,
                      reduced: Theme.reducedMotion }
        }
        function cleanupTestCase() {
            Demo.orbShow = saved.show; Demo.orbProfile = saved.profile; Demo.orbNodesI = saved.nodes
            Demo.orbGradeI = saved.grade; Demo.orbDust = saved.dust; Demo.orbBrightI = saved.bright
            Demo.orbActI = saved.act; Demo.orbSpin = saved.spin
            Theme.colorMode = saved.mode; Theme.vivid = saved.vivid; Theme.motion = saved.motion
            Theme.opaque = saved.opaque; Theme.reducedMotion = saved.reduced
        }
        function options(item, out) {
            if (item.picked !== undefined && item.options !== undefined && item.visible) out.push(item)
            for (var i = 0; i < item.children.length; i++) options(item.children[i], out)
            return out
        }
        function chips(opt) {
            for (var i = 0; i < opt.children.length; i++) {
                var row = opt.children[i]
                if (row.visible && row.spacing !== undefined && row.currentIndex === undefined) {
                    var out = []
                    for (var j = 0; j < row.children.length; j++) if (row.children[j].radius !== undefined) out.push(row.children[j])
                    return out
                }
            }
            return []
        }
        function test_every_orb_and_appearance_option_takes_effect() {
            tryVerify(function() { return settings.opacity === 1 })
            var clicked = 0
            for (var section of [4, 5]) {
                settings.section = section
                wait(50)
                var opts = options(settings, [])
                verify(opts.length >= 5, "section " + section + " has its options")
                for (var opt of opts) {
                    var list = chips(opt)
                    if (list.length === 0) {
                        // More than seven choices: a combo box, which emits the same signal.
                        for (var c = opt.options.length - 1; c >= 0; c--) { opt.picked(c); compare(opt.index, c, opt.label) }
                        continue
                    }
                    compare(list.length, opt.options.length, opt.label)
                    for (var k = list.length - 1; k >= 0; k--) {
                        mouseClick(list[k])
                        compare(opt.index, k, opt.label + " · " + opt.options[k])
                        clicked++
                    }
                }
            }
            verify(clicked >= 30, "clicked " + clicked)
        }
    }
}
