import {randomBytes,createHash,timingSafeEqual} from 'node:crypto';
import {generateRegistrationOptions,verifyRegistrationResponse,generateAuthenticationOptions,verifyAuthenticationResponse} from '@simplewebauthn/server';
const hash=value=>createHash('sha256').update(value).digest('hex');
const actions=new Set(['activate-node','approve-permission','add-device']);
export function validateAction(action){
  if(!action || !actions.has(action.kind) || typeof action.description!=='string' || action.description.length<1 || action.description.length>240 || typeof action.resource!=='string' || action.resource.length>256)throw Error('invalid action');
  if(Object.keys(action).some(k=>!['kind','description','resource'].includes(k)))throw Error('unknown action field');
  return {kind:action.kind,description:action.description,resource:action.resource};
}
export class OwnerService {
  constructor(pool,{nodeId,ownerName,rpID,origin,bootstrapCode},clock=()=>Date.now()){
    if(!/^[a-f0-9]{64}$/.test(nodeId)||!ownerName?.trim()||ownerName.length>80)throw Error('invalid node/owner configuration');
    if(typeof bootstrapCode!=='string'||bootstrapCode.length<32)throw Error('private bootstrap code required');
    this.nativeKey=bootstrapCode;this.bootstrapHash=hash(bootstrapCode);this.bootstrapExpires=clock()+900000;
    this.pool=pool;this.nodeId=nodeId;this.ownerName=ownerName;this.rpID=rpID;this.origin=origin;this.clock=clock;
  }
  async initialize(){await this.pool.query(`
    CREATE SCHEMA IF NOT EXISTS archie_owner;
    CREATE TABLE IF NOT EXISTS archie_owner.credentials (
      node_id text NOT NULL, id text NOT NULL, public_key bytea NOT NULL, counter bigint NOT NULL,
      transports jsonb NOT NULL, created_at bigint NOT NULL, revoked boolean NOT NULL DEFAULT false,
      PRIMARY KEY(node_id,id));
    CREATE TABLE IF NOT EXISTS archie_owner.challenges (
      node_id text NOT NULL, session text NOT NULL, challenge text NOT NULL, ceremony text NOT NULL,
      action jsonb, expires_at bigint NOT NULL, PRIMARY KEY(node_id,session));
    CREATE TABLE IF NOT EXISTS archie_owner.approvals (
      id text PRIMARY KEY,node_id text NOT NULL,session text NOT NULL,credential_id text NOT NULL,
      action jsonb NOT NULL,action_sha256 text NOT NULL,created_at bigint NOT NULL,expires_at bigint NOT NULL,
      consumed_at bigint);
  `);}
  async credentials(){return (await this.pool.query('SELECT * FROM archie_owner.credentials WHERE node_id=$1 AND NOT revoked',[this.nodeId])).rows;}
  async options(session,ceremony,action,approvalId,activationCode){
    const credentials=await this.credentials();
    if(ceremony==='register' && !credentials.length &&
      (this.clock()>=this.bootstrapExpires || typeof activationCode!=='string' ||
       !timingSafeEqual(Buffer.from(hash(activationCode),'hex'),Buffer.from(this.bootstrapHash,'hex'))))
      throw Error('private installer activation code required');
    if(ceremony==='register' && credentials.length){
      const consumed=await this.consume(approvalId,session,{kind:'add-device',description:'Añadir otro dispositivo',resource:this.nodeId});
      if(!consumed)throw Error('existing owner approval required to add a device');
    }
    if(ceremony==='authenticate' && !credentials.length)throw Error('register an owner passkey first');
    const operation=ceremony==='authenticate'?validateAction(action):(credentials.length?{kind:'add-device',description:'Add owner passkey',resource:this.nodeId}:null);
    if(operation?.kind==='add-device'){operation.description='Añadir otro dispositivo';operation.resource=this.nodeId;}
    const options=ceremony==='register' ? await generateRegistrationOptions({
      rpName:'ArchieOS',rpID:this.rpID,userName:this.ownerName,userDisplayName:this.ownerName,
      userID:Buffer.from(this.nodeId,'hex'),
      attestationType:'none',authenticatorSelection:{residentKey:'required',userVerification:'required'},
      excludeCredentials:credentials.map(c=>({id:c.id,transports:c.transports})),
    }) : await generateAuthenticationOptions({rpID:this.rpID,userVerification:'required',
      allowCredentials:credentials.map(c=>({id:c.id,transports:c.transports}))});
    await this.pool.query('DELETE FROM archie_owner.challenges WHERE node_id=$1 AND expires_at<=$2',[this.nodeId,this.clock()]);
    await this.pool.query(`INSERT INTO archie_owner.challenges VALUES($1,$2,$3,$4,$5,$6)
      ON CONFLICT(node_id,session) DO UPDATE SET challenge=EXCLUDED.challenge,ceremony=EXCLUDED.ceremony,action=EXCLUDED.action,expires_at=EXCLUDED.expires_at`,
      [this.nodeId,hash(session),options.challenge,ceremony,operation,this.clock()+120000]);
    return {options,action:operation,nodeId:this.nodeId};
  }
  async verify(session,ceremony,response){
    const found=await this.pool.query('DELETE FROM archie_owner.challenges WHERE node_id=$1 AND session=$2 RETURNING *',[this.nodeId,hash(session)]);
    const challenge=found.rows[0];
    if(!challenge || challenge.ceremony!==ceremony || Number(challenge.expires_at)<=this.clock())throw Error('challenge expired or already used');
    const common={response,expectedChallenge:challenge.challenge,expectedOrigin:this.origin,expectedRPID:this.rpID,requireUserVerification:true};
    if(ceremony==='register'){
      const checked=await verifyRegistrationResponse(common);
      if(!checked.verified || !checked.registrationInfo)throw Error('registration not verified');
      const c=checked.registrationInfo.credential;
      // Serialize bootstrap registration for this node; no two first-owner races.
      const client=await this.pool.connect();
      try{
        await client.query('BEGIN');await client.query('SELECT pg_advisory_xact_lock(hashtext($1))',[this.nodeId]);
        // Existing credential addition was authorized by a consumed add-device receipt.
        // The challenge records whether an owner existed when it was issued (below).
        const count=Number((await client.query('SELECT count(*) FROM archie_owner.credentials WHERE node_id=$1',[this.nodeId])).rows[0].count);
        if(count>=16)throw Error('device limit');
        if(count>0 && challenge.action?.kind!=='add-device')throw Error('owner already registered');
        await client.query('INSERT INTO archie_owner.credentials(node_id,id,public_key,counter,transports,created_at) VALUES($1,$2,$3,$4,$5,$6)',
          [this.nodeId,c.id,Buffer.from(c.publicKey),c.counter,JSON.stringify(c.transports||[]),this.clock()]);
        await client.query('COMMIT');return {verified:true,credentialId:c.id,nodeId:this.nodeId};
      }catch(error){await client.query('ROLLBACK');throw error;}finally{client.release();}
    }
    const credential=(await this.credentials()).find(c=>c.id===response.id);
    if(!credential)throw Error('unknown credential');
    const checked=await verifyAuthenticationResponse({...common,credential:{id:credential.id,publicKey:new Uint8Array(credential.public_key),counter:Number(credential.counter),transports:credential.transports}});
    if(!checked.verified)throw Error('authentication not verified');
    const id=randomBytes(32).toString('base64url'),now=this.clock();
    const client=await this.pool.connect();
    try{
      await client.query('BEGIN');
      const update=await client.query('UPDATE archie_owner.credentials SET counter=$3 WHERE node_id=$1 AND id=$2 AND counter=$4 AND NOT revoked',
        [this.nodeId,credential.id,checked.authenticationInfo.newCounter,credential.counter]);
      if(update.rowCount!==1)throw Error('credential changed during verification');
      await client.query('INSERT INTO archie_owner.approvals VALUES($1,$2,$3,$4,$5,$6,$7,$8,NULL)',
        [id,this.nodeId,hash(session),credential.id,challenge.action,hash(JSON.stringify(validateAction(challenge.action))),now,now+60000]);
      await client.query('COMMIT');return {verified:true,approvalId:id,action:challenge.action,expiresAt:now+60000,nodeId:this.nodeId};
    }catch(error){await client.query('ROLLBACK');throw error;}finally{client.release();}
  }
  async consume(id,session,expectedAction){
    const action=validateAction(expectedAction);
    if(typeof id!=='string'||!actions.has(action.kind))return null;
    const result=await this.pool.query(`UPDATE archie_owner.approvals SET consumed_at=$4
      WHERE id=$1 AND node_id=$2 AND session=$3 AND consumed_at IS NULL AND expires_at>$4 AND action_sha256=$5
      AND EXISTS (SELECT 1 FROM archie_owner.credentials c WHERE c.node_id=$2 AND c.id=archie_owner.approvals.credential_id AND NOT c.revoked) RETURNING action`,
      [id,this.nodeId,hash(session),this.clock(),hash(JSON.stringify(action))]);return result.rows[0]?.action||null;
  }
}
