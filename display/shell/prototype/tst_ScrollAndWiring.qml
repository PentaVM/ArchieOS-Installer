import QtQuick
import QtTest

Item {
    width:1920; height:1080
    Main { id:desktop; width:parent.width; height:parent.height }
    AppWindow { id:app; width:700; height:300; Item { width:400; height:1400 } }
    InspectorPanel { id:inspector; bridge: ({inspector:{source:"fixture", state:"ready", cortex_trajectories:Array.from({length:32}, function(_,i) { return {trajectory_id:i,correction_kind:"test",model_id:"test",selected_context_count:0,omitted_candidate_count:0} })}}) }
    TestCase {
        name:"ScrollAndWiring"; when:windowShown
        function test_large_inspector_is_scrollable() {
            var scroll=findChild(inspector,"projectionScroll")
            verify(scroll!==null,"Large projection needs a bounded scroll viewport")
            verify(scroll.clip)
            verify(scroll.contentHeight>scroll.height)
            scroll.contentY=scroll.contentHeight-scroll.height
            verify(scroll.contentY>0)
        }
        function test_long_app_catalog_is_scrollable() {
            var scroll=findChild(app,"appContentScroll")
            verify(scroll!==null,"Provider/model/job lists must remain reachable")
            verify(scroll.contentHeight>scroll.height)
        }
        function test_global_search_uses_transport_freshness() {
            verify(desktop.bridge.ingest(JSON.stringify({schema:1,at_ms:1,link:{state:"displayd_offline",reason:"test offline"},system:null,kernel:null})))
            var search=findChild(desktop,"globalSearch")
            verify(search!==null)
            compare(search.st,"offline")
        }
    }
}
