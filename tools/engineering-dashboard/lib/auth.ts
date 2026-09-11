import {createHash,createHmac,randomBytes,timingSafeEqual} from "node:crypto";
import type {StatusStore} from "./store.ts";
export const SESSION_COOKIE="archie_dashboard_session";
export const SESSION_MS=4*60*60*1000;
export function secret(name:string,min=32):string {const value=process.env[name];if(!value||value.length<min)throw Error("dashboard security configuration unavailable");return value;}
export function equal(a:string,b:string){return timingSafeEqual(createHash("sha256").update(a).digest(),createHash("sha256").update(b).digest());}
export function sessionHash(token:string){return createHmac("sha256",secret("ARCHIE_DASHBOARD_SESSION_SECRET")).update(token).digest("hex");}
export async function login(store:StatusStore,password:string,now=Date.now()):Promise<string|null>{
  if(!await store.attempt("login-global",now,10,10*60*1000))return null;
  if(!equal(password,secret("ARCHIE_DASHBOARD_PASSWORD",1)))return null;
  const token=randomBytes(32).toString("base64url");await store.putSession(sessionHash(token),now+SESSION_MS);return token;
}
export async function authenticated(store:StatusStore,token:string|undefined,now=Date.now()){
  return !!token&&/^[A-Za-z0-9_-]{43}$/.test(token)&&await store.validSession(sessionHash(token),now);
}
export function validOrigin(origin:string|null){
  const expected=process.env.ARCHIE_DASHBOARD_ORIGIN;
  return !!expected&&origin===expected&&(!process.env.VERCEL||expected.startsWith("https://"));
}
export function serverSecrets(){return ["ARCHIE_DASHBOARD_PASSWORD","ARCHIE_DASHBOARD_SESSION_SECRET","ARCHIE_DASHBOARD_INGEST_SECRET","ARCHIE_DASHBOARD_DATABASE_URL"].map(k=>process.env[k]??"").filter(Boolean);}
