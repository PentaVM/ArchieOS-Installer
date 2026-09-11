import {digest} from "./events.ts";
import type {StatusEvent} from "./events.ts";

export interface StatusStore {
  append(event:StatusEvent):Promise<"created"|"duplicate">;
  recent(limit:number,phase?:string):Promise<StatusEvent[]>;
  putSession(hash:string,expires:number):Promise<void>;
  validSession(hash:string,now:number):Promise<boolean>;
  revokeSession(hash:string):Promise<void>;
  attempt(bucket:string,now:number,limit:number,windowMs:number):Promise<boolean>;
}
const schema=`CREATE TABLE IF NOT EXISTS dashboard_events(id TEXT PRIMARY KEY,at BIGINT NOT NULL,phase TEXT NOT NULL,payload TEXT NOT NULL,hash TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS dashboard_events_time ON dashboard_events(at DESC,id);
CREATE TABLE IF NOT EXISTS dashboard_sessions(hash TEXT PRIMARY KEY,expires BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS dashboard_limits(bucket TEXT PRIMARY KEY,count INTEGER NOT NULL,expires BIGINT NOT NULL);`;
// Postgres is the deployed adapter. It stores sessions and limits too: neither
// serverless memory nor its ephemeral filesystem is treated as authoritative.
export class PostgresStatusStore implements StatusStore {
  pool:import("pg").Pool;
  constructor(pool:import("pg").Pool){this.pool=pool;}
  async migrate(){
    const client=await this.pool.connect();
    try{
      await client.query("BEGIN");
      await client.query("SET LOCAL statement_timeout='5s'");
      await client.query("SELECT pg_advisory_xact_lock(1095910216)");
      await client.query(schema);await client.query("COMMIT");
    }catch(error){await client.query("ROLLBACK");throw error;}finally{client.release();}
  }
  async append(e:StatusEvent){
    const payload=JSON.stringify(e),hash=digest(payload);
    const result=await this.pool.query("INSERT INTO dashboard_events VALUES ($1,$2,$3,$4,$5) ON CONFLICT(id) DO NOTHING RETURNING id",[e.event_id,Date.parse(e.timestamp),e.phase,payload,hash]);
    if(result.rowCount)return "created" as const;
    const old=await this.pool.query("SELECT hash FROM dashboard_events WHERE id=$1",[e.event_id]);
    if(old.rows[0]?.hash!==hash)throw Error("event ID collision");return "duplicate" as const;
  }
  async recent(limit:number,phase?:string){
    if(!Number.isInteger(limit)||limit<1||limit>200)throw Error("invalid limit");
    const {rows}=phase?await this.pool.query("SELECT payload,hash FROM dashboard_events WHERE phase=$1 ORDER BY at DESC,id LIMIT $2",[phase,limit]):
      await this.pool.query("SELECT payload,hash FROM dashboard_events ORDER BY at DESC,id LIMIT $1",[limit]);
    return rows.map(r=>{if(digest(r.payload)!==r.hash)throw Error("corrupt status event");return JSON.parse(r.payload) as StatusEvent;});
  }
  async putSession(hash:string,expires:number){await this.pool.query("INSERT INTO dashboard_sessions VALUES ($1,$2)",[hash,expires]);}
  async validSession(hash:string,now:number){return !!(await this.pool.query("SELECT 1 FROM dashboard_sessions WHERE hash=$1 AND expires>$2",[hash,now])).rowCount;}
  async revokeSession(hash:string){await this.pool.query("DELETE FROM dashboard_sessions WHERE hash=$1",[hash]);}
  async attempt(bucket:string,now:number,limit:number,windowMs:number){
    const {rows}=await this.pool.query(`INSERT INTO dashboard_limits VALUES ($1,1,$2) ON CONFLICT(bucket) DO UPDATE
      SET count=CASE WHEN dashboard_limits.expires<=$3 THEN 1 ELSE dashboard_limits.count+1 END,
      expires=CASE WHEN dashboard_limits.expires<=$3 THEN $2 ELSE dashboard_limits.expires END RETURNING count`,[bucket,now+windowMs,now]);
    return rows[0].count<=limit;
  }
}
let current:Promise<StatusStore>|undefined;
export function getStore():Promise<StatusStore>{
  return current??=(async()=>{
    const url=process.env.ARCHIE_DASHBOARD_DATABASE_URL;
    if(url){
      const {Pool}=await import("pg");
      const pool=new Pool({connectionString:url,max:3,connectionTimeoutMillis:5000,idleTimeoutMillis:10000,statement_timeout:5000,query_timeout:10000});
      const store=new PostgresStatusStore(pool);
      try{await store.migrate();return store;}catch(error){await pool.end();throw error;}
    }
    throw Error("PostgreSQL status database required");
  })().catch(e=>{current=undefined;throw e;});
}
