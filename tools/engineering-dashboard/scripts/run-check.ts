//! A development wrapper, not an Archie executor. Command and arguments are passed
//! directly to spawn; no shell expansion. Full output stays local, status is bounded.
import {spawn} from "node:child_process";
import {mkdirSync,createWriteStream,writeFileSync} from "node:fs";
import {resolve} from "node:path";
import {randomUUID,createHash} from "node:crypto";
import {redact,validateEvent} from "../lib/events.ts";
import {enqueue,drain} from "../lib/outbox.ts";
import {LogRedactor} from "../lib/log-redactor.ts";
const [phase,module,command,...args]=process.argv.slice(2);
if(!phase||!module||!command)throw Error("usage: run-check <BUILD|TEST|BENCHMARK> <module> <executable> [args...]");
if(!["BUILD","TEST","BENCHMARK"].includes(phase))throw Error("invalid phase");
const directory=resolve(process.env.ARCHIE_DASHBOARD_LOG_DIR??".dashboard-local/checks");mkdirSync(directory,{recursive:true});
const run=randomUUID(),start=Date.now();
const secrets=Object.entries(process.env).filter(([k])=>/(PASSWORD|SECRET|TOKEN|DATABASE_URL|PRIVATE_KEY)/.test(k)).map(([,v])=>v??"");
const log=createWriteStream(resolve(directory,`${run}.log`),{flags:"wx",mode:0o600}),hash=createHash("sha256");
let tail="";
const child=spawn(command,args,{shell:false,windowsHide:true,stdio:["ignore","pipe","pipe"]});
const consume=(text:string)=>{log.write(text);hash.update(text);tail=(tail+text).slice(-4000);};
const stdout=new LogRedactor(secrets,consume),stderr=new LogRedactor(secrets,consume);
child.stdout.on("data",chunk=>stdout.push(chunk));child.stderr.on("data",chunk=>stderr.push(chunk));
const outbox=resolve(process.env.ARCHIE_DASHBOARD_OUTBOX??".dashboard-local/outbox");
let sequence=0,delivery=Promise.resolve();
const save=(status:"running"|"pass"|"fail",exit?:number)=>{
  const event=validateEvent({schema_version:1,event_id:`${run}-${sequence++}-${status}`,timestamp:new Date().toISOString(),phase,module,status,
    summary:exit===undefined?"Comprobación iniciada":`Comprobación finalizada con salida ${exit}`,
    command:redact([command,...args].join(" "),secrets).slice(0,1800),duration_ms:Date.now()-start,
    ...(exit===undefined?{}:{exit_code:exit}),...(exit&&tail?{next_action:redact(tail,secrets).slice(-1800)}:{})},Date.now(),secrets);
  writeFileSync(resolve(directory,`${event.event_id}.json`),JSON.stringify(event),{flag:"wx",mode:0o600});
  enqueue(outbox,event);
  if(process.env.ARCHIE_DASHBOARD_INGEST_URL)delivery=delivery.then(async()=>{
    try{await drain(outbox,process.env.ARCHIE_DASHBOARD_INGEST_URL!,process.env.ARCHIE_DASHBOARD_INGEST_SECRET??"");}
    catch{console.log("Status delivery unavailable; immutable local outbox retained.");}
  });
};
save("running");
const heartbeat=setInterval(()=>save("running"),30_000);
const result=await new Promise<number>(done=>{child.once("error",()=>done(127));child.once("close",code=>done(code??1));});
clearInterval(heartbeat);
stdout.end();stderr.end();
await new Promise<void>(done=>log.end(done));
save(result===0?"pass":"fail",result);
await delivery;
writeFileSync(resolve(directory,`${run}.sha256`),hash.digest("hex"),{flag:"wx",mode:0o600});
console.log(`Check completed (${result}); local logs and sanitized events retained.`);
process.exitCode=result;
