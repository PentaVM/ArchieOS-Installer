import test from 'node:test';
import assert from 'node:assert/strict';
import {randomBytes} from 'node:crypto';
import {createRequire} from 'node:module';
import {Pool} from 'pg';
import {mac} from '../native-auth.mjs';
import {OwnerService} from '../service.mjs';
import {httpServer} from '../server.mjs';
const database=process.env.ARCHIE_OWNER_TEST_DATABASE_URL;
test('real PostgreSQL and browser authenticator: enrol, verify, exact scope, expiry and replay', {skip:!database,timeout:90000},async()=>{
  const {chromium}=createRequire(import.meta.url)(process.env.ARCHIE_PLAYWRIGHT_MODULE);
  const pool=new Pool({connectionString:database,max:3});let now=Date.now();
  const nodeId=randomBytes(32).toString('hex');
  const service=new OwnerService(pool,{nodeId,ownerName:'Owner fixture',bootstrapCode:'fixture-activation-code-32-characters',rpID:'localhost',origin:'http://localhost:27668'},()=>now);
  await service.initialize();const server=httpServer(service,27668);await new Promise(resolve=>server.listen(27668,'127.0.0.1',resolve));
  let browser;
  try{
    browser=await chromium.launch({channel:'chrome',headless:true});const context=await browser.newContext();const page=await context.newPage();
    const cdp=await context.newCDPSession(page);await cdp.send('WebAuthn.enable');
    await cdp.send('WebAuthn.addVirtualAuthenticator',{options:{protocol:'ctap2',transport:'internal',hasResidentKey:true,hasUserVerification:true,isUserVerified:true,automaticPresenceSimulation:true}});
    await page.goto('http://localhost:27668/');await page.waitForFunction(()=>typeof SimpleWebAuthnBrowser!=='undefined');
    const noCode=await page.evaluate(async()=>{try{await api('options/register');return false;}catch{return true;}});assert.equal(noCode,true);
    const registration=await page.evaluate(async()=>{
      const issued=await api('options/register',{activationCode:'fixture-activation-code-32-characters'});const response=await SimpleWebAuthnBrowser.startRegistration({optionsJSON:issued.options});return api('verify/register',{response});
    });assert.equal(registration.verified,true);
    assert.equal((await service.credentials()).length,1);
    const blocked=await page.evaluate(async()=>{try{await api('options/register');return false;}catch{return true;}});assert.equal(blocked,true);
    const action={kind:'approve-permission',description:'Read only this sensor',resource:'sensor:test'};
    const approved=await page.evaluate(async action=>{
      const issued=await api('options/authenticate',{action});const response=await SimpleWebAuthnBrowser.startAuthentication({optionsJSON:issued.options});
      const result=await api('verify/authenticate',{response});let replay=false;try{await api('verify/authenticate',{response});}catch{replay=true;}return {result,replay};
    },action);assert.equal(approved.result.verified,true);assert.equal(approved.replay,true);
    const cookie=(await context.cookies()).find(c=>c.name==='archie_owner').value;
    assert.equal(await service.consume(approved.result.approvalId,cookie,{...action,resource:'other'}),null);
    assert.deepEqual(await service.consume(approved.result.approvalId,cookie,action),action);
    assert.equal(await service.consume(approved.result.approvalId,cookie,action),null);
    // The native wizard submits immutable scope and receives a signed one-use result.
    async function native(path,body){
      const stamp=String(Date.now()),nonce=randomBytes(24).toString('hex'),raw=JSON.stringify(body),key='fixture-activation-code-32-characters';
      const response=await fetch('http://localhost:27668'+path,{method:'POST',headers:{'Content-Type':'application/json','X-Archie-Time':stamp,'X-Archie-Nonce':nonce,'X-Archie-Signature':mac(key,stamp+'\n'+nonce+'\n'+path+'\n'+raw)},body:raw});
      const text=await response.text();assert.equal(response.headers.get('x-archie-response-signature'),mac(key,nonce+'\n'+response.status+'\n'+text));return {status:response.status,body:JSON.parse(text)};
    }
    const requested=await native('/native/request',{action});
    const pending=await native('/native/result',{requestId:requested.body.requestId});assert.equal(pending.body.approved,false);
    await page.evaluate(async requestId=>{
      const issued=await api('options/authenticate',{requestId,action:{kind:'activate-node',description:'tampered',resource:'wrong'}});
      const response=await SimpleWebAuthnBrowser.startAuthentication({optionsJSON:issued.options});return api('verify/authenticate',{response});
    },requested.body.requestId);
    const delivered=await native('/native/result',{requestId:requested.body.requestId});assert.equal(delivered.body.approved,true);assert.deepEqual(delivered.body.action,action);
    const again=await native('/native/result',{requestId:requested.body.requestId});assert.equal(again.status,410);
    const revokedProof=await page.evaluate(async action=>{
      const issued=await api('options/authenticate',{action});const response=await SimpleWebAuthnBrowser.startAuthentication({optionsJSON:issued.options});return api('verify/authenticate',{response});
    },action);
    await pool.query('UPDATE archie_owner.credentials SET revoked=true WHERE node_id=$1',[nodeId]);
    assert.equal(await service.consume(revokedProof.approvalId,cookie,action),null);
    await pool.query('UPDATE archie_owner.credentials SET revoked=false WHERE node_id=$1',[nodeId]);
    const response=await page.evaluate(async action=>{
      const issued=await api('options/authenticate',{action});return SimpleWebAuthnBrowser.startAuthentication({optionsJSON:issued.options});
    },action);now+=120001;
    await assert.rejects(service.verify(cookie,'authenticate',response),/expired/);
    console.log('PASS: real WebAuthn signatures with PostgreSQL; virtual authenticator, not a physical phone.');
  }finally{
    if(browser)await browser.close();await new Promise(resolve=>server.close(resolve));await pool.end();
  }
});
