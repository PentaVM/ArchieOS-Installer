import {NextRequest,NextResponse} from "next/server";
import {getStore} from "../../../lib/store.ts";
import {authenticated,equal,login,secret,serverSecrets,sessionHash,SESSION_COOKIE,SESSION_MS,validOrigin} from "../../../lib/auth.ts";
import {MAX_PAYLOAD,validateEvent} from "../../../lib/events.ts";
export const runtime="nodejs";
export const dynamic="force-dynamic";
const reply=(body:unknown,status=200)=>NextResponse.json(body,{status,headers:{"Cache-Control":"private, no-store"}});
async function readLimited(request:NextRequest,max:number){
  if(Number(request.headers.get("content-length"))>max)throw Error("too large");
  const reader=request.body?.getReader();if(!reader)return "";
  const chunks:Uint8Array[]=[];let size=0;
  try{while(true){const {done,value}=await reader.read();if(done)break;size+=value.byteLength;
    if(size>max){await reader.cancel();throw Error("too large");}chunks.push(value);}}
  finally{reader.releaseLock();}
  return Buffer.concat(chunks).toString("utf8");
}
export async function GET(request:NextRequest,context:{params:Promise<{action:string}>}){
  try {
    const store=await getStore();
    if(!await authenticated(store,request.cookies.get(SESSION_COOKIE)?.value))return reply({error:"unauthorized"},401);
    if((await context.params).action!=="status")return reply({error:"not found"},404);
    return reply({events:await store.recent(100)});
  }catch{return reply({error:"status unavailable"},503);}
}
export async function POST(request:NextRequest,context:{params:Promise<{action:string}>}){
  try {
    const {action}=await context.params;
    if(action==="ingest"){
      const authorization=request.headers.get("authorization")??"";
      if(!equal(authorization,`Bearer ${secret("ARCHIE_DASHBOARD_INGEST_SECRET")}`))return reply({error:"unauthorized"},401);
      const store=await getStore();
      if(!await store.attempt("ingest-global",Date.now(),120,60_000))return reply({error:"rate limited"},429);
      let event;
      try{event=validateEvent(JSON.parse(await readLimited(request,MAX_PAYLOAD)),Date.now(),serverSecrets());}
      catch{return reply({error:"invalid or oversized event"},400);}
      try{return reply({result:await store.append(event)});}
      catch{return reply({error:"event conflict or storage failure"},409);}
    }
    if(!validOrigin(request.headers.get("origin")))return reply({error:"origin rejected"},403);
    const store=await getStore();
    if(action==="login"){
      let body;try{body=new URLSearchParams(await readLimited(request,4096));}catch{return reply({error:"invalid login"},400);}
      const token=await login(store,body.get("password")??"");
      if(!token)return reply({error:"login rejected or rate limited"},401);
      const response=NextResponse.redirect(new URL("/",secret("ARCHIE_DASHBOARD_ORIGIN",1)),303);
      response.cookies.set(SESSION_COOKIE,token,{httpOnly:true,secure:process.env.VERCEL!==undefined||process.env.ARCHIE_DASHBOARD_ORIGIN?.startsWith("https://"),sameSite:"strict",path:"/",maxAge:SESSION_MS/1000});
      return response;
    }
    if(action==="logout"){
      const token=request.cookies.get(SESSION_COOKIE)?.value;
      if(token)await store.revokeSession(sessionHash(token));
      const response=NextResponse.redirect(new URL("/login",secret("ARCHIE_DASHBOARD_ORIGIN",1)),303);
      response.cookies.set(SESSION_COOKIE,"",{httpOnly:true,secure:!!process.env.VERCEL,sameSite:"strict",path:"/",maxAge:0});return response;
    }
    return reply({error:"not found"},404);
  }catch{return reply({error:"dashboard unavailable"},503);}
}
