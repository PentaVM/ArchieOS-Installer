import {createHash} from "node:crypto";

export const MAX_PAYLOAD = 512 * 1024;
export const maturities=["SCAFFOLDED","DATA_STARVED","SHADOW","BENCHMARKED","VALIDATED","ACTIVE","DEGRADED","UNAVAILABLE"] as const;
export const checkKinds=["build","unit","integration","replay","failure","perf","security"] as const;
export const phases = ["DISCOVERY","IMPLEMENTATION","BUILD","TEST","BENCHMARK","MIGRATION","COMMIT","DEPLOY","BLOCKER","RECOVERY"] as const;
export type StatusEvent = {
  schema_version: 1; event_id: string; timestamp: string; phase: typeof phases[number];
  module: string; summary: string; status: "pass"|"fail"|"running"|"blocked"|"unverified";
  command?: string; duration_ms?: number; exit_code?: number; next_action?: string;
  git?: {branch: string; commit: string; dirty: boolean};
  metrics?: Record<string,number>;
  maturity?: typeof maturities[number];
  checks?: Partial<Record<typeof checkKinds[number],{status:"pass"|"fail"|"blocked"|"unverified";command?:string}>>;
  files?: {path:string;kind:"file"|"directory"|"symlink";state:"tracked"|"untracked"|"ignored";size?:number}[];
};
const object = (v: unknown): v is Record<string,unknown> => v !== null && typeof v === "object" && !Array.isArray(v);
const bounded = (v: unknown,n: number): v is string => typeof v === "string" && v.length > 0 && Buffer.byteLength(v) <= n && !v.includes("\0");
const number = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v) && v>=0 && v<=Number.MAX_SAFE_INTEGER;
function keys(v:Record<string,unknown>, allowed:string[]) {if(Object.keys(v).some(k=>!allowed.includes(k))) throw Error("unknown field");}

export function redact(value: string, secrets: string[] = []): string {
  let out=value;
  for(const secret of secrets.filter(s=>s.length>=4)) out=out.split(secret).join("[REDACTED]");
  return out.replace(/-----BEGIN [^-]*PRIVATE KEY-----[\s\S]*?-----END [^-]*PRIVATE KEY-----/g,"[REDACTED KEY]")
    .replace(/\b(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{16,})\b/g,"[REDACTED TOKEN]")
    .replace(/(https?:\/\/)[^\s/@]+:[^\s/@]+@/gi,"$1[REDACTED]@")
    .replace(/\b(password|secret|authorization|api[_-]?key|access[_-]?token)\s*[:=]\s*[^\r\n,;]+/gi,"$1=[REDACTED]");
}
export function validateEvent(input:unknown, now=Date.now(), secrets:string[]=[]):StatusEvent {
  if(!object(input)) throw Error("object required");
  keys(input,["schema_version","event_id","timestamp","phase","module","summary","status","command","duration_ms","exit_code","next_action","git","metrics","files","maturity","checks"]);
  if(input.schema_version!==1 || !bounded(input.event_id,128) || !/^[\w:.-]+$/.test(input.event_id) ||
    !bounded(input.timestamp,40) || !Number.isFinite(Date.parse(input.timestamp)) ||
    Math.abs(Date.parse(input.timestamp)-now)>7*86400000 || !phases.includes(input.phase as typeof phases[number]) ||
    !bounded(input.module,96) || !bounded(input.summary,2000) || !["pass","fail","running","blocked","unverified"].includes(String(input.status))) throw Error("invalid event envelope");
  for(const key of ["command","next_action"]) if(input[key]!==undefined && !bounded(input[key],2000)) throw Error("invalid text");
  if(input.maturity!==undefined&&!maturities.includes(input.maturity as typeof maturities[number]))throw Error("invalid maturity");
  if(input.checks!==undefined){
    if(!object(input.checks))throw Error("invalid checks");keys(input.checks,[...checkKinds]);
    for(const check of Object.values(input.checks)){
      if(!object(check))throw Error("invalid check");keys(check,["status","command"]);
      if(!["pass","fail","blocked","unverified"].includes(String(check.status))||(check.command!==undefined&&!bounded(check.command,1000))||
        (check.status==="pass"&&!bounded(check.command,1000)))throw Error("passing check needs command evidence");
    }
  }
  if(input.duration_ms!==undefined && !number(input.duration_ms)) throw Error("invalid duration");
  if(input.exit_code!==undefined && (!Number.isInteger(input.exit_code)||Math.abs(input.exit_code as number)>65535)) throw Error("invalid exit");
  if(input.git!==undefined) {
    if(!object(input.git)) throw Error("invalid git");keys(input.git,["branch","commit","dirty"]);
    if(!bounded(input.git.branch,192)||typeof input.git.commit!=="string"||!/^[a-f0-9]{40,64}$/.test(input.git.commit)||typeof input.git.dirty!=="boolean") throw Error("invalid git");
  }
  if(input.metrics!==undefined && (!object(input.metrics)||Object.keys(input.metrics).length>32 ||
    Object.entries(input.metrics).some(([k,v])=>!(/^[a-zA-Z][\w.-]{0,63}$/.test(k))||!number(v)))) throw Error("invalid metrics");
  if(input.files!==undefined) {
    if(!Array.isArray(input.files)||input.files.length>5000) throw Error("invalid tree bound");
    for(const f of input.files) {
      if(!object(f)) throw Error("invalid entry");keys(f,["path","kind","state","size"]);
      if(!bounded(f.path,512)||!['file','directory','symlink'].includes(String(f.kind))||!['tracked','untracked','ignored'].includes(String(f.state))||
        (f.size!==undefined&&!number(f.size))) throw Error("invalid file metadata");
    }
  }
  // Only strings are sanitized, then the complete payload is size-bounded. Paths are
  // inert metadata: there is no server route that opens a published path.
  const sanitize=(v:unknown):unknown=>typeof v==="string"?redact(v,secrets):Array.isArray(v)?v.map(sanitize):object(v)?Object.fromEntries(Object.entries(v).map(([k,x])=>[k,sanitize(x)])):v;
  const output=sanitize(input) as StatusEvent;
  if(Buffer.byteLength(JSON.stringify(output))>MAX_PAYLOAD) throw Error("payload too large");
  return output;
}
export const digest=(text:string)=>createHash("sha256").update(text).digest("hex");
