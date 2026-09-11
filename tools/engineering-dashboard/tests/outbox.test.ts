import test from "node:test";
import assert from "node:assert/strict";
import {mkdtempSync,rmSync,readFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {enqueue,drain} from "../lib/outbox.ts";
import {validateEvent} from "../lib/events.ts";
test("disconnection preserves immutable events; retry delivers once and collision fails",async()=>{
  const dir=mkdtempSync(join(tmpdir(),"archie-outbox-"));
  try{
    const event=validateEvent({schema_version:1,event_id:"retry-one",timestamp:new Date().toISOString(),phase:"TEST",module:"outbox",status:"pass",summary:"fixture"});
    const path=enqueue(dir,event),body=readFileSync(path,"utf8");
    assert.equal(enqueue(dir,event),path);
    assert.throws(()=>enqueue(dir,{...event,summary:"changed"}));
    await assert.rejects(drain(dir,"https://example.test/api/ingest","a".repeat(32),async()=>{throw Error("offline");}));
    assert.equal(readFileSync(path,"utf8"),body);
    await assert.rejects(drain(dir,"https://example.test/api/ingest","a".repeat(32),async()=>new Response('{}')));
    let calls=0;
    const send:typeof fetch=async(_url,options)=>{calls++;assert.equal(options?.body,body);return new Response('{"result":"created"}');};
    assert.equal(await drain(dir,"https://example.test/api/ingest","a".repeat(32),send),1);
    assert.equal(await drain(dir,"https://example.test/api/ingest","a".repeat(32),send),0);
    assert.equal(calls,1);
    await assert.rejects(drain(dir,"http://example.test","a".repeat(32),send));
    assert.throws(()=>enqueue(dir,{...event,event_id:"../escape"}));
  }finally{rmSync(dir,{recursive:true,force:true});}
});
