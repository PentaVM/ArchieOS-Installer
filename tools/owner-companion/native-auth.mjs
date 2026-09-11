import {createHmac,timingSafeEqual} from 'node:crypto';
export const mac=(key,text)=>createHmac('sha256',key).update(text).digest('hex');
export function verifyNative(key,path,body,headers,seen,now=Date.now()){
  const stamp=headers['x-archie-time'],nonce=headers['x-archie-nonce'],signature=headers['x-archie-signature'];
  if(!/^\d{13}$/.test(stamp||'')||Math.abs(now-Number(stamp))>30000||!/^[-a-zA-Z0-9]{32,64}$/.test(nonce||'')||!/^[a-f0-9]{64}$/.test(signature||''))return false;
  for(const [id,expiry] of seen)if(expiry<now)seen.delete(id);
  if(seen.has(nonce)||seen.size>=4096)return false;
  const expected=mac(key,stamp+'\n'+nonce+'\n'+path+'\n'+body);
  if(!timingSafeEqual(Buffer.from(expected,'hex'),Buffer.from(signature,'hex')))return false;
  seen.set(nonce,now+30000);return true;
}
