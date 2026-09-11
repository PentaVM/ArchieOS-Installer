import test from "node:test";
import assert from "node:assert/strict";
import {randomBytes} from "node:crypto";
import {Pool} from "pg";
import {PostgresStatusStore} from "../lib/store.ts";
import {validateEvent} from "../lib/events.ts";

const url=process.env.ARCHIE_DASHBOARD_TEST_DATABASE_URL;
test("real PostgreSQL durability, parallel idempotency, limits and revocation",{skip:!url},async()=>{
  // A separate explicit test URL is required. Never clear a deployment's tables.
  const schema="archie_test_"+randomBytes(12).toString("hex");
  assert.match(schema,/^archie_test_[a-f0-9]{24}$/);
  const admin=new Pool({connectionString:url,max:1,connectionTimeoutMillis:5000});
  let pool:Pool|undefined;
  let created=false;
  try{
    await admin.query(`CREATE SCHEMA ${schema}`);created=true;
    pool=new Pool({connectionString:url,max:3,options:`-c search_path=${schema}`,connectionTimeoutMillis:5000});
    let store=new PostgresStatusStore(pool);await Promise.all(Array.from({length:8},()=>store.migrate()));
    const event=validateEvent({schema_version:1,event_id:"postgres-one",timestamp:new Date().toISOString(),phase:"TEST",module:"postgres",status:"pass",summary:"isolated integration fixture"});
    const results=await Promise.all(Array.from({length:8},()=>store.append(event)));
    assert.equal(results.filter(r=>r==="created").length,1);
    assert.equal((await store.recent(10)).length,1);await assert.rejects(store.append({...event,summary:"conflict"}));
    await store.putSession("test-session-hash",200);
    const attempts=await Promise.all(Array.from({length:12},()=>store.attempt("budget",100,10,50)));
    assert.equal(attempts.filter(Boolean).length,10);assert.equal(await store.attempt("budget",150,10,50),true);
    await pool.end();pool=new Pool({connectionString:url,max:1,options:`-c search_path=${schema}`,connectionTimeoutMillis:5000});
    store=new PostgresStatusStore(pool);assert.equal((await store.recent(10)).length,1);
    assert.equal(await store.validSession("test-session-hash",199),true);assert.equal(await store.validSession("test-session-hash",200),false);
    await store.revokeSession("test-session-hash");assert.equal(await store.validSession("test-session-hash",199),false);
    await pool.query("UPDATE dashboard_events SET payload='{}'");await assert.rejects(store.recent(10));
  }finally{
    if(pool)await pool.end();
    if(created)await admin.query(`DROP SCHEMA ${schema} CASCADE`);
    await admin.end();
  }
});
