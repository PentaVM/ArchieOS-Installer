import {createServer} from 'node:http';
import {readFileSync} from 'node:fs';
import {randomBytes} from 'node:crypto';
import {mac,verifyNative} from './native-auth.mjs';
import {fileURLToPath} from 'node:url';
import {Pool} from 'pg';
import {OwnerService,validateAction} from './service.mjs';
export function httpServer(service,port=7668){
  const origin=`http://localhost:${port}`,sessions=new Map(),requests=new Map(),awaiting=new Map(),nonces=new Map();
  const files=new Map([['/',['public/index.html','text/html; charset=utf-8']],['/app.js',['public/app.js','text/javascript']],['/webauthn.js',['node_modules/@simplewebauthn/browser/dist/bundle/index.umd.min.js','text/javascript']]]);
  return createServer(async(req,res)=>{
    res.setHeader('Cache-Control','no-store');res.setHeader('X-Content-Type-Options','nosniff');res.setHeader('Referrer-Policy','no-referrer');
    res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
    let authenticatedNative=false;
    const send=(code,value)=>{const payload=JSON.stringify(value);if(authenticatedNative)res.setHeader('X-Archie-Response-Signature',mac(service.nativeKey,req.headers['x-archie-nonce']+'\n'+code+'\n'+payload));res.writeHead(code,{'Content-Type':'application/json'});res.end(payload);};
    try{
      if(req.headers.host!==`localhost:${port}`){send(403,{error:'invalid host'});return;}
      if(req.method==='GET'&&files.has(req.url)){
        if(req.url==='/'){
          for(const [key,expiry] of sessions)if(expiry<Date.now()){sessions.delete(key);awaiting.delete(key);}
          if(sessions.size>=64){send(429,{error:'too many sessions'});return;}
          const session=randomBytes(32).toString('base64url');sessions.set(session,Date.now()+900000);
          res.setHeader('Set-Cookie',`archie_owner=${session}; HttpOnly; SameSite=Strict; Path=/; Max-Age=900`);
        }
        const [file,type]=files.get(req.url);res.writeHead(200,{'Content-Type':type});res.end(readFileSync(new URL(file,import.meta.url)));return;
      }
      const native=req.url.startsWith('/native/');
      if(req.method!=='POST'||(!native&&req.headers.origin!==origin)||!req.headers['content-type']?.startsWith('application/json')){send(403,{error:'same-origin JSON required'});return;}
      const session=/(?:^|; )archie_owner=([A-Za-z0-9_-]{43})(?:;|$)/.exec(req.headers.cookie||'')?.[1];
      if(!native&&(!session||!(sessions.get(session)>Date.now()))){send(401,{error:'session expired'});return;}
      let length=0;const chunks=[];for await(const chunk of req){length+=chunk.length;if(length>65536){send(413,{error:'request too large'});return;}chunks.push(chunk);}
      const raw=Buffer.concat(chunks).toString();
      if(native){
        if(!service.nativeKey||!verifyNative(service.nativeKey,req.url,raw,req.headers,nonces)){send(403,{error:'invalid native authentication'});return;}
        authenticatedNative=true;
      }
      const body=JSON.parse(raw);
      for(const [id,request] of requests)if(request.expires<Date.now())requests.delete(id);
      if(native){
        if(req.url==='/native/request'){
          if(requests.size>=32){send(429,{error:'too many pending requests'});return;}
          const action=validateAction(body.action),id=randomBytes(32).toString('base64url');
          requests.set(id,{action,expires:Date.now()+120000});send(200,{requestId:id});return;
        }
        if(req.url==='/native/result'){
          const request=requests.get(body.requestId);if(!request){send(410,{error:'expired'});return;}
          if(!request.proof){send(200,{approved:false});return;}
          const approved=await service.consume(request.proof.approvalId,request.proof.session,request.action);
          requests.delete(body.requestId);send(200,{approved:!!approved,action:approved});return;
        }
        send(404,{error:'unknown native operation'});return;
      }
      if(req.url==='/api/request'){
        const request=requests.get(body.requestId);if(!request){send(410,{error:'expired request'});return;}
        send(200,{action:request.action,nodeId:service.nodeId});return;
      }
      if(req.url==='/api/status'){send(200,{nodeId:service.nodeId,ownerName:service.ownerName,registered:(await service.credentials()).length>0});return;}
      const match=/^\/api\/(options|verify)\/(register|authenticate)$/.exec(req.url);
      if(!match){send(404,{error:'unknown operation'});return;}
      if(match[1]==='options' && match[2]==='authenticate' && body.requestId){
        const request=requests.get(body.requestId);if(!request){send(410,{error:'expired request'});return;}
        body.action=request.action;awaiting.set(session,body.requestId);
      }else if(match[1]==='options'){awaiting.delete(session);}
      const result=match[1]==='options'?await service.options(session,match[2],body.action,body.approvalId,body.activationCode):await service.verify(session,match[2],body.response);
      if(match[1]==='verify' && match[2]==='authenticate' && result.verified){
        const requestId=awaiting.get(session),request=requests.get(requestId);awaiting.delete(session);
        if(request && JSON.stringify(validateAction(result.action))===JSON.stringify(validateAction(request.action)))
          request.proof={approvalId:result.approvalId,session};
      }
      send(200,result);
    }catch{send(400,{error:'The request could not be verified. Start a new challenge.'});}
  });
}
if(process.argv[1]===fileURLToPath(import.meta.url)){
  const path=process.env.ARCHIE_OWNER_CONFIG;if(!path)throw Error('Set ARCHIE_OWNER_CONFIG to a private JSON configuration file');
  const config=JSON.parse(readFileSync(path,'utf8'));
  const pool=new Pool({connectionString:config.connection,max:4});
  const service=new OwnerService(pool,{...config,rpID:'localhost',origin:'http://localhost:7668'});
  await service.initialize();
  const server=httpServer(service);server.listen(7668,'127.0.0.1',()=>console.log('Owner companion ready on http://localhost:7668'));
  const stop=()=>server.close(()=>pool.end());process.on('SIGINT',stop);process.on('SIGTERM',stop);
}
