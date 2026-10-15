import QtQuick
import QtTest

Item {
    width:800; height:900
    QtObject { id:source; property var jobs:({state:"ok",jobs:[]}); property bool silent:false }
    NotificationCenter { id:center; bridge:source }
    TestCase {
        name:"NotificationJobs"; when:windowShown
        function test_dismiss_is_local_and_a_new_job_update_reappears() {
            var original={workspace:"fixture-work",id:"fixture-job",bridge:"fixture-bridge",kind:"fixture",state:"running",updated_ms:1}
            source.jobs={state:"ok",jobs:[original]}
            compare(center.visibleRecords.length,1)
            center.dismiss(original)
            compare(center.visibleRecords.length,0)
            compare(source.jobs.jobs.length,1)
            source.jobs={state:"ok",jobs:[Object.assign({},original,{state:"done",updated_ms:2})]}
            compare(center.visibleRecords.length,1)
            center.dismissAll()
            compare(center.visibleRecords.length,0)
            center.restore()
            compare(center.visibleRecords.length,1)
            compare(source.jobs.jobs[0].state,"done")
        }
    }
}
