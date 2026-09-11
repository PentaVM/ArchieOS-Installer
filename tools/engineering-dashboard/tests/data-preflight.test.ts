import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {Pool} from "pg";
const url=process.env.ARCHIE_DASHBOARD_TEST_DATABASE_URL;
test("real PostgreSQL preflight refuses absent reviewed extensions without creating objects",{skip:!url},async()=>{
  const pool=new Pool({connectionString:url,max:1,connectionTimeoutMillis:5000});
  const client=await pool.connect();
  try{
    const before=await client.query("SELECT count(*)::int AS n FROM pg_namespace");
    const installed=await client.query("SELECT extname FROM pg_extension WHERE extname IN ('timescaledb','vector')");
    // This failure-injection case requires the isolated plain-PostgreSQL fixture.
    assert.equal(installed.rowCount,0);
    const sql=readFileSync(new URL("../../../distro/database/preflight.sql",import.meta.url),"utf8");
    await assert.rejects(client.query(sql),/Reviewed TimescaleDB.*unavailable|requires PostgreSQL major 18/);
    await client.query("ROLLBACK");
    assert.equal((await client.query("SELECT count(*)::int AS n FROM pg_namespace")).rows[0].n,before.rows[0].n);
  }finally{client.release();await pool.end();}
});
