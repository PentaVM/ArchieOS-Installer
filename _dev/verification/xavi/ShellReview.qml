import QtQuick

// Audit harness, separate from Xavi's implementation. Use stock qml with an external
// timeout and inspect stderr as well: the author's assertions do not count warnings.
Item {
    Loader { id: bridge; source: "../../../display/shell/prototype/BridgeModelTest.qml" }
    Loader { id: surfaces; source: "../../../display/shell/prototype/SurfacesTest.qml" }
    Loader { id: windows; source: "../../../display/shell/prototype/WmTest.qml" }
    Timer {
        interval: 500
        running: true
        onTriggered: {
            for (const entry of [{name: "BridgeModelTest", loader: bridge},
                                 {name: "SurfacesTest", loader: surfaces},
                                 {name: "WmTest", loader: windows}]) {
                const test = entry.loader.item
                console.log("REVIEW_RESULT " + JSON.stringify(test
                    ? {name: entry.name, passes: test.passes, failures: test.failures, log: test.log}
                    : {name: entry.name, loadFailed: true, status: entry.loader.status}))
            }
            Qt.quit()
        }
    }
}
