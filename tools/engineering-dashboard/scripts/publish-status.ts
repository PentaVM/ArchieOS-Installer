//! Local outbound-only publisher. Publishes path metadata, never source contents.
import {execFileSync} from "node:child_process";
import {lstatSync,readFileSync} from "node:fs";
import {resolve,dirname,relative} from "node:path";
import {fileURLToPath} from "node:url";
import {randomUUID} from "node:crypto";
import {redact,validateEvent,MAX_PAYLOAD} from "../lib/events.ts";
import type {StatusEvent} from "../lib/events.ts";
import {enqueue,drain} from "../lib/outbox.ts";
const root=resolve(dirname(fileURLToPath(import.meta.url)),"../../..");
const git=(...args:string[])=>execFileSync("git",args,{cwd:root,encoding:"utf8",maxBuffer:4*1024*1024});
const secrets=Object.entries(process.env).filter(([key])=>/(SECRET|PASSWORD|TOKEN|DATABASE_URL|PRIVATE_KEY)/.test(key)).map(([,v])=>v??"");
function metadata():NonNullable<StatusEvent["files"]>{
  const entries:NonNullable<StatusEvent["files"]>=[];
  const sets:[string,string[]][]=[['tracked',["ls-files","-z"]],['untracked',["ls-files","--others","--exclude-standard","-z"]],['ignored',["ls-files","--others","--ignored","--exclude-standard","--directory","-z"]]];
  for(const [state,args] of sets)for(const path of git(...args).split("\0").filter(Boolean)){
    // An ignored build tree is one metadata entry. Do not crawl credentials,
    // recordings or hundreds of thousands of package-cache descendants.
    const absolute=resolve(root,path),rel=relative(root,absolute);
    if(rel.startsWith("..")||rel.includes(":")||entries.length>=5000)throw Error("repository metadata exceeds safe bound");
    let info;try{info=lstatSync(absolute);}catch{continue;}
    entries.push({path:redact(path,secrets),kind:info.isSymbolicLink()?"symlink":info.isDirectory()?"directory":"file",state:state as "tracked"|"untracked"|"ignored",...(info.isFile()?{size:info.size}:{})});
  }
  return entries;
}
const supplied=process.argv[2];
const directory=resolve(process.env.ARCHIE_DASHBOARD_OUTBOX??resolve(root,".dashboard-local/outbox"));
if(supplied==="--release"){
  const report=JSON.parse(readFileSync(resolve(root,"docs/RELEASE_STATUS.json"),"utf8"));
  for(const row of report.subsystems){
    enqueue(directory,validateEvent({schema_version:1,event_id:randomUUID(),timestamp:new Date().toISOString(),phase:"DISCOVERY",module:row.name,status:"unverified",summary:row.scope,maturity:row.maturity,checks:row.checks,
      git:report.git},Date.now(),secrets));
  }
}else if(supplied!=="--retry"){
const base:StatusEvent=supplied?JSON.parse(readFileSync(resolve(supplied),"utf8")):{
  schema_version:1,event_id:randomUUID(),timestamp:new Date().toISOString(),phase:"DISCOVERY",module:"repository",
  status:"unverified",summary:"Inventario local del repositorio; no implica validación del runtime.",files:metadata()
};
base.git??={branch:git("branch","--show-current").trim(),commit:git("rev-parse","HEAD").trim(),dirty:git("status","--porcelain").length>0};
const event=validateEvent(base,Date.now(),secrets),body=JSON.stringify(event);
if(Buffer.byteLength(body)>MAX_PAYLOAD)throw Error("metadata exceeds ingest bound");
enqueue(directory,event);
}
const endpoint=process.env.ARCHIE_DASHBOARD_INGEST_URL;
if(!endpoint){console.log("Sanitized event saved to local outbox; no remote endpoint configured.");}
else{
  const count=await drain(directory,endpoint,process.env.ARCHIE_DASHBOARD_INGEST_SECRET??"");
  console.log(`${count} sanitized events delivered; local receipts retained.`);
}
