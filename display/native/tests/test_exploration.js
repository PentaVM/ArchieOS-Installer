// Run with node display/native/tests/test_exploration.js (no Qt window or daemon).
const assert=require('node:assert/strict');
const fs=require('node:fs');const vm=require('node:vm');const path=require('node:path');
const api={};vm.createContext(api);
vm.runInContext(fs.readFileSync(path.join(__dirname,'../qml/Exploration.js'),'utf8').replace('.pragma library',''),api);
assert.equal(api.bytes(null),'Sin lectura');assert.equal(api.count(undefined),'Sin lectura');
assert.equal(api.bytes(0),'0 B');assert.equal(api.count(0),'0');
assert.equal(api.validCamera({latitude:0,longitude:0,radius_m:1000}),true);
assert.equal(api.validCamera({latitude:91,longitude:0,radius_m:1000}),false);
assert.equal(api.validCamera({latitude:null,longitude:0,radius_m:1000}),false);
const seen=[];const jobs=[{id:'personal-job',domain:'personal'},{id:'work-job',domain:'work'}];
assert.equal(api.newJobs(jobs,seen).length,2);
assert.equal(api.newJobs(jobs,seen).length,0); // A status poll cannot reclaim a manually ceded camera.
assert.equal(api.newJobs(jobs.map(x=>({...x,state:'complete'})),seen).length,0);
assert.equal(api.newJobs([...jobs,{id:'next-job'}],seen).length,1);
assert.equal(api.metrics({metrics:{counters:{requests:0},gauges:{queue:2}}}).length,2);
assert.equal(api.metrics({}).length,0); // Missing metrics never become fabricated zeroes/TPS.
assert.equal(api.cameraElevation({camera:{elevation_m:4200}}),4200);
assert.equal(api.cameraElevation({camera:{elevation_m:-430}}),-430);
assert.equal(api.cameraElevation({camera:{elevation_m:0}}),0);
assert.equal(api.cameraElevation({camera:{elevation_m:null}}),null);
assert.equal(api.cameraElevation({preview:{terrain:{elevations_m:[600,700]}}}),null); // No guessed or forced measured elevation.
assert.equal(api.elevationRange({preview:{terrain:{elevations_m:[null,-430,-400]}}}),'-430.0 … -400.0 m');
const closeFrame={explorationCamera:true,cameraEye:{x:0,y:0,z:20.01},cameraTarget:{x:0,y:0,z:19.99}};
assert.equal(api.localTerrainView(closeFrame,[{kind:'terrain'}]),true); // Global ocean/basemap must yield to real below-sea geometry.
assert.equal(api.localTerrainView({...closeFrame,explorationCamera:false},[{kind:'terrain'}]),false);
assert.equal(api.localTerrainView(closeFrame,[{kind:'aoi'}]),false);
assert.equal(api.localTerrainView({...closeFrame,cameraEye:{x:0,y:0,z:60}},[{kind:'terrain'}]),false);
assert.equal(api.inference({}).tps,'Sin fuente medida publicada');
assert.equal(api.inference({metrics:{gauges:{'inference.last.effective_output_tps_available':0,'inference.last.effective_output_tokens_per_second_x1000':-1}}}).tps,'Sin fuente medida publicada');
assert.equal(api.inference({metrics:{gauges:{'inference.last.effective_output_tps_available':1,'inference.last.effective_output_tokens_per_second_x1000':12345}}}).tps,'12.35 tok/s');
console.log('Exploration projection/camera helper contracts passed');
