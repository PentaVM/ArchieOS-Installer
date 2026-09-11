import test from "node:test";
import assert from "node:assert/strict";
import {randomBytes} from "node:crypto";
import {PostgresStatusStore} from "../lib/store.ts";
import {Pool} from "pg";
import {validateEvent,redact} from "../lib/events.ts";
import {authenticated,login,sessionHash,SESSION_MS,validOrigin} from "../lib/auth.ts";
const url=process.env.ARCHIE_DASHBOARD_TEST_DATABASE_URL;
async function fixture(t:import("node:test").TestContext){
  const schema="archie_security_"+randomBytes(12).toString("hex");
  const admin=new Pool({connectionString:url,max:1,connectionTimeoutMillis:5000});
  await admin.query(`CREATE SCHEMA ${schema}`);
  const pools:Pool[]=[];
  t.after(async()=>{for(const pool of pools)await pool.end();await admin.query(`DROP SCHEMA ${schema} CASCADE`);await admin.end();});
  function reopen(){const pool=new Pool({connectionString:url,max:1,connectionTimeoutMillis:5000,options:`-c search_path=${schema}`});pools.push(pool);return new PostgresStatusStore(pool);}
  const store=reopen();await store.migrate();return {store,reopen};
}
const password=randomBytes(24).toString("hex");
process.env.ARCHIE_DASHBOARD_PASSWORD=password;
process.env.ARCHIE_DASHBOARD_SESSION_SECRET=randomBytes(32).toString("hex");
process.env.ARCHIE_DASHBOARD_ORIGIN="https://engineering.example.test";
function event(){return {schema_version:1,event_id:"fixture:one",timestamp:new Date().toISOString(),phase:"TEST",module:"security",status:"pass",summary:"fixture"};}

test("authentication rejects anonymous/wrong credentials, expires and revokes sessions",{skip:!url},async(t)=>{
  const {store}=await fixture(t);const now=Date.now();
  assert.equal(await authenticated(store,undefined,now),false);
  assert.equal(await login(store,"wrong",now),null);
  const token=await login(store,password,now);assert.ok(token);
  assert.equal(await authenticated(store,token,now+1),true);
  assert.equal(await authenticated(store,token,now+SESSION_MS),false);
  await store.revokeSession(sessionHash(token));assert.equal(await authenticated(store,token,now+1),false);
  assert.equal(await authenticated(store,token+"tampered",now),false);
});
test("login brute-force budget is durable and resets only after its window",{skip:!url},async(t)=>{
  const {store}=await fixture(t);const now=Date.now();
  for(let i=0;i<10;i++)assert.equal(await login(store,"wrong",now),null);
  assert.equal(await login(store,password,now+1),null);
  assert.ok(await login(store,password,now+600001));
});
test("origin gate rejects missing and foreign origin",()=>{
  assert.equal(validOrigin(null),false);assert.equal(validOrigin("https://evil.example"),false);
  assert.equal(validOrigin("https://engineering.example.test"),true);
});
test("sanitization removes credentials without destroying measurement hashes",()=>{
  const token="ghp_"+"x".repeat(30);
  const sanitized=validateEvent({...event(),summary:`credential ${password} ${token}`},Date.now(),[password]);
  assert.ok(!sanitized.summary.includes(password));assert.ok(!sanitized.summary.includes(token));
  assert.equal(redact("password=secret-value"),"password=[REDACTED]");
  assert.equal(redact("a".repeat(64)),"a".repeat(64));
});
test("schema rejects unknown fields, oversized text, invalid time and NaN metrics",()=>{
  assert.throws(()=>validateEvent({...event(),raw_audio:"private"}));
  assert.throws(()=>validateEvent({...event(),summary:"x".repeat(2001)}));
  assert.throws(()=>validateEvent({...event(),timestamp:"not a date"}));
  assert.throws(()=>validateEvent({...event(),metrics:{quality:NaN}}));
  assert.throws(()=>validateEvent({...event(),checks:{unit:{status:"pass"}}}));
  assert.throws(()=>validateEvent({...event(),maturity:"MAGIC"}));
  assert.equal(validateEvent({...event(),checks:{unit:{status:"pass",command:"npm test"}},maturity:"VALIDATED"}).checks?.unit?.command,"npm test");
});
test("XSS and traversal remain inert strings, with no content-read API",()=>{
  const e=validateEvent({...event(),summary:"<script>alert(1)</script>",files:[{path:"../../private.key",kind:"file",state:"ignored"}]});
  assert.equal(e.summary,"<script>alert(1)</script>");assert.equal(e.files?.[0].path,"../../private.key");
});
test("status append is idempotent, collisions fail and corruption is detected",{skip:!url},async(t)=>{
  const {store}=await fixture(t);const e=validateEvent(event());
  assert.equal(await store.append(e),"created");assert.equal(await store.append(e),"duplicate");
  await assert.rejects(store.append({...e,summary:"changed"}));assert.equal((await store.recent(5)).length,1);
  await store.pool.query("UPDATE dashboard_events SET payload='{}'");await assert.rejects(store.recent(5));
});
test("PostgreSQL history and session revocation survive reopening",{skip:!url},async(t)=>{
  const {store:first,reopen}=await fixture(t);
  await first.append(validateEvent(event()));const token=await login(first,password);assert.ok(token);
  const second=reopen();assert.equal((await second.recent(5)).length,1);assert.equal(await authenticated(second,token),true);
  await second.revokeSession(sessionHash(token));const third=reopen();assert.equal(await authenticated(third,token),false);
});
