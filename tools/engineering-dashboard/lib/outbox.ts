import {mkdirSync,readFileSync,writeFileSync,existsSync,readdirSync} from "node:fs";
import {resolve} from "node:path";
import {digest,validateEvent} from "./events.ts";
import type {StatusEvent} from "./events.ts";

// Immutable payloads retain the original timestamp and Git revision on retry.
export function enqueue(directory:string,event:StatusEvent){
  if(!/^[\w:.-]+$/.test(event.event_id))throw Error("unsafe outbox ID");
  mkdirSync(directory,{recursive:true});
  const path=resolve(directory,`${digest(event.event_id)}.json`),body=JSON.stringify(event);
  try{writeFileSync(path,body,{flag:"wx",mode:0o600});}
  catch(error){if((error as NodeJS.ErrnoException).code!=="EEXIST")throw error;if(readFileSync(path,"utf8")!==body)throw Error("outbox ID collision");}
  return path;
}
export async function drain(directory:string,endpoint:string,token:string,send:typeof fetch=fetch){
  const url=new URL(endpoint);
  if(url.protocol!=="https:"&&!(process.env.ARCHIE_DASHBOARD_ALLOW_LOCAL_HTTP==="1"&&url.protocol==="http:"&&url.hostname==="127.0.0.1"))throw Error("HTTPS ingest required");
  if(token.length<32)throw Error("ingest credential unavailable");
  mkdirSync(directory,{recursive:true});
  let delivered=0;
  for(const name of readdirSync(directory).filter(n=>/^[\w.-]+\.json$/.test(n)&&!existsSync(resolve(directory,`${n}.ack`))).sort().slice(0,200)){
    const path=resolve(directory,name),body=readFileSync(path,"utf8"),hash=digest(body),receipt=`${path}.ack`;
    if(existsSync(receipt)){if(readFileSync(receipt,"utf8")!==hash)throw Error("outbox receipt mismatch");continue;}
    validateEvent(JSON.parse(body));
    const response=await send(url,{method:"POST",headers:{"Content-Type":"application/json",Authorization:`Bearer ${token}`},body,signal:AbortSignal.timeout(10000),redirect:"error"});
    if(!response.ok)throw Error(`ingest refused (${response.status}); local event retained`);
    const acknowledgement=await response.json();
    if(!["created","duplicate"].includes(acknowledgement?.result))throw Error("ingest acknowledgement missing; local event retained");
    writeFileSync(receipt,hash,{mode:0o600});delivered++;
  }
  return delivered;
}
