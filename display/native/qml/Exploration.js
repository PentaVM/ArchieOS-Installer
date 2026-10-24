.pragma library
function known(value) { return typeof value === "number" && Number.isFinite(value); }
function count(value) { return known(value) ? String(value) : "Sin lectura"; }
function bytes(value) {
    if (!known(value)) return "Sin lectura";
    var n=Number(value);
    return n < 1024 ? n + " B" : n < 1048576 ? (n/1024).toFixed(1)+" KiB" : (n/1048576).toFixed(1)+" MiB";
}
function state(value) {
    return ({queued:"En cola",running:"Explorando",waiting_rate_limit:"Esperando cuota",paused:"En pausa",complete:"Completo",failed:"Error",cancelled:"Cancelado"})[value] || "Sin lectura";
}
function tone(value) { return value==="failed" ? "bad" : value==="waiting_rate_limit" || value==="paused" ? "warn" : value==="complete" ? "ok" : "neutral"; }
function validCamera(camera) {
    return !!camera && known(camera.latitude) && Math.abs(camera.latitude)<=90 && known(camera.longitude) && Math.abs(camera.longitude)<=180 && known(camera.radius_m) && camera.radius_m>0 && camera.radius_m<=6371000;
}
function newJobs(jobs, seen) {
    var added=[];
    for(var i=0;i<jobs.length;++i) {
        var id=jobs[i].id;
        if(typeof id!=="string" || !id.length || seen.indexOf(id)>=0) continue;
        seen.push(id);added.push(jobs[i]);
    }
    return added;
}
function metrics(snapshot) {
    var rows=[], registry=snapshot.metrics || {};
    ["counters","gauges"].forEach(function(kind) {
        var values=registry[kind] || {};
        Object.keys(values).sort().forEach(function(key) {rows.push({name:key, value:String(values[key]), kind:kind});});
    });
    return rows;
}
function cameraElevation(job) {
    var value=job.camera && job.camera.elevation_m;
    return known(value) && value>=-12000 && value<=12000 ? value : null;
}
function elevationRange(job) {
    var samples=job.preview && job.preview.terrain && job.preview.terrain.elevations_m;
    var values=(samples || []).filter(function(value) { return known(value) && value>=-12000 && value<=12000; });
    return values.length ? Math.min.apply(null,values).toFixed(1)+" … "+Math.max.apply(null,values).toFixed(1)+" m" : "Sin lectura";
}
function localTerrainView(frame, meshes) {
    if(!frame || frame.explorationCamera!==true || !frame.cameraEye || !frame.cameraTarget) return false;
    var eye=frame.cameraEye,target=frame.cameraTarget;
    var distance=Math.hypot(eye.x-target.x,eye.y-target.y,eye.z-target.z);
    // At close AOI scale the decoded patch replaces the global basemap, including ocean.
    return Number.isFinite(distance) && distance<=0.5 && (meshes || []).some(function(mesh) { return mesh.kind==="terrain"; });
}

function inference(snapshot) {
    var gauges=snapshot.metrics && snapshot.metrics.gauges || {};
    var prefix="inference.last.";
    var tps=gauges[prefix+"effective_output_tokens_per_second_x1000"];
    var tokens=gauges[prefix+"output_tokens"];
    return {
        tps:gauges[prefix+"effective_output_tps_available"]===1 && known(tps) && tps>=0 ? (tps/1000).toFixed(2)+" tok/s" : "Sin fuente medida publicada",
        tokens:gauges[prefix+"output_tokens_available"]===1 && known(tokens) && tokens>=0 ? String(tokens) : "Sin lectura",
        elapsed:gauges[prefix+"request_elapsed_ms_available"]===1 && known(gauges[prefix+"request_elapsed_ms"]) && gauges[prefix+"request_elapsed_ms"]>=0 ? gauges[prefix+"request_elapsed_ms"]+" ms" : "Sin lectura",
        observed:known(gauges[prefix+"observed_at_ms"]) ? new Date(gauges[prefix+"observed_at_ms"]).toLocaleTimeString() : "Sin lectura"
    };
}
