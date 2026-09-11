import test from 'node:test';
import assert from 'node:assert/strict';
import {validateAction} from '../service.mjs';
import {httpServer} from '../server.mjs';
test('action scope rejects hidden permissions and unbounded descriptions',()=>{
  assert.throws(()=>validateAction({kind:'sudo',description:'x',resource:'y'}));
  assert.throws(()=>validateAction({kind:'activate-node',description:'x',resource:'y',admin:true}));
  assert.throws(()=>validateAction({kind:'activate-node',description:'x'.repeat(241),resource:'y'}));
  assert.deepEqual(validateAction({kind:'approve-permission',description:'read sensor',resource:'sensor:1'}),{kind:'approve-permission',description:'read sensor',resource:'sensor:1'});
});
test('HTTP blocks cross-origin access, foreign hosts and missing sessions',async()=>{
  const server=httpServer({},27669);await new Promise(resolve=>server.listen(27669,'127.0.0.1',resolve));
  try{
    const foreign=await fetch('http://127.0.0.1:27669/');assert.equal(foreign.status,403);
    const bad=await fetch('http://localhost:27669/api/options/register',{method:'POST',headers:{Origin:'https://evil.test','Content-Type':'application/json'},body:'{}'});assert.equal(bad.status,403);
    const unauthenticated=await fetch('http://localhost:27669/api/options/register',{method:'POST',headers:{Origin:'http://localhost:27669','Content-Type':'application/json'},body:'{}'});assert.equal(unauthenticated.status,401);
    const home=await fetch('http://localhost:27669/');assert.equal(home.status,200);assert.match(home.headers.get('set-cookie'),/HttpOnly; SameSite=Strict/);
    assert.match(home.headers.get('content-security-policy'),/frame-ancestors 'none'/);
    const script=await fetch('http://localhost:27669/webauthn.js');assert.equal(script.status,200);
  }finally{await new Promise(resolve=>server.close(resolve));}
});

test('native channel authenticates both scope and freshness without sending its key',async()=>{
  const {mac,verifyNative}=await import('../native-auth.mjs');
  const key='fixture-key-with-at-least-32-bytes',now=Date.now(),nonce='a'.repeat(32),path='/native/request',body='{}';
  const headers={'x-archie-time':String(now),'x-archie-nonce':nonce,'x-archie-signature':mac(key,now+'\n'+nonce+'\n'+path+'\n'+body)};
  assert.equal(verifyNative(key,path,body,headers,new Map(),now),true);
  assert.equal(verifyNative(key,'/native/result',body,headers,new Map(),now),false);
  assert.equal(verifyNative(key,path,'{"changed":true}',headers,new Map(),now),false);
  assert.equal(verifyNative(key,path,body,headers,new Map(),now+30001),false);
  const seen=new Map();assert.equal(verifyNative(key,path,body,headers,seen,now),true);assert.equal(verifyNative(key,path,body,headers,seen,now),false);
});
