import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {Pool} from "pg";
const url=process.env.ARCHIE_DASHBOARD_TEST_DATABASE_URL;
test("reviewed extensions perform real time-series aggregation and vector retrieval",{skip:!url||process.env.ARCHIE_DATA_EXTENSIONS_ENABLED!=="1"},async()=>{
  // Only enable through with-postgres.mjs, which creates a separate local cluster.
  const pool=new Pool({connectionString:url,max:1,connectionTimeoutMillis:5000,statement_timeout:10000});
  const client=await pool.connect();let created=false;
  try{
    await client.query("CREATE EXTENSION timescaledb VERSION '2.29.2'; CREATE EXTENSION vector VERSION '0.8.6'");
    await client.query(readFileSync(new URL("../../../distro/database/preflight.sql",import.meta.url),"utf8"));
    await client.query("CREATE SCHEMA archie_extension_fixture");created=true;
    await client.query("CREATE TABLE archie_extension_fixture.telemetry(at timestamptz NOT NULL, node text NOT NULL, value double precision NOT NULL)");
    await client.query("SELECT create_hypertable('archie_extension_fixture.telemetry',by_range('at'))");
    await client.query("INSERT INTO archie_extension_fixture.telemetry SELECT '2026-09-06T00:00:00Z'::timestamptz + i*interval '1 second','fixture',i FROM generate_series(0,999) AS i");
    const aggregate=await client.query("SELECT count(*)::int AS n,avg(value) AS mean FROM archie_extension_fixture.telemetry");
    assert.equal(aggregate.rows[0].n,1000);assert.equal(aggregate.rows[0].mean,499.5);
    await client.query("CREATE TABLE archie_extension_fixture.embedding(id text PRIMARY KEY, value vector(3) NOT NULL)");
    await client.query("INSERT INTO archie_extension_fixture.embedding VALUES ('a','[1,0,0]'),('b','[0,1,0]')");
    const nearest=await client.query("SELECT id FROM archie_extension_fixture.embedding ORDER BY value <=> $1::vector LIMIT 1",['[1,0,0]']);
    assert.equal(nearest.rows[0].id,'a');
    await assert.rejects(client.query("INSERT INTO archie_extension_fixture.embedding VALUES ('bad','[1,2]')"),/dimensions/);
    assert.equal((await client.query("SELECT count(*)::int AS n FROM archie_extension_fixture.embedding")).rows[0].n,2);
  }finally{
    await client.query("ROLLBACK");
    if(created)await client.query("DROP SCHEMA archie_extension_fixture CASCADE");
    client.release();await pool.end();
  }
});
