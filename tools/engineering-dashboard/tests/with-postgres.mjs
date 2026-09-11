// Disposable local PostgreSQL validation. Never connects to an existing cluster.
import {spawn} from 'node:child_process';
import {mkdtempSync,writeFileSync,unlinkSync,appendFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {randomBytes} from 'node:crypto';
import {createServer} from 'node:net';
const workspaceMode=process.argv.includes('--workspace');
const coreMode=process.argv.includes('--core')||workspaceMode;
const bin=process.env.ARCHIE_POSTGRES_BIN;
if(!bin)throw Error('Set ARCHIE_POSTGRES_BIN to an existing PostgreSQL bin directory');
const executable=name=>join(resolve(bin),name+(process.platform==='win32'?'.exe':''));
const directory=mkdtempSync(join(tmpdir(),'archie-isolated-postgres-'));
const data=join(directory,'data'),passwordFile=join(directory,'init-password');
const password=randomBytes(32).toString('hex');
writeFileSync(passwordFile,password,{flag:'wx',mode:0o600});
async function run(command,args,env=process.env){
  const child=spawn(command,args,{env,windowsHide:true,shell:false,stdio:['ignore','pipe','pipe']});
  child.stdout.on('data',b=>process.stdout.write(b.toString().split(password).join('[REDACTED]')));
  child.stderr.on('data',b=>process.stderr.write(b.toString().split(password).join('[REDACTED]')));
  const backgroundControl=/pg_ctl(?:\.exe)?$/.test(command);
  const code=await new Promise((done,reject)=>{child.once('error',reject);child.once(backgroundControl?'exit':'close',done);});
  if(backgroundControl){child.stdout.destroy();child.stderr.destroy();}
  if(code!==0)throw Error(`Isolated database check exited ${code}`);
}
const reservation=createServer();await new Promise(done=>reservation.listen(0,'127.0.0.1',done));
const port=reservation.address().port;await new Promise(done=>reservation.close(done));
let started=false;
try{
  await run(executable('initdb'),['-D',data,'--username=archie_test','--auth=scram-sha-256',`--pwfile=${passwordFile}`,'--encoding=UTF8','--no-locale']);
  unlinkSync(passwordFile);
  const extensionRoot=process.env.ARCHIE_POSTGRES_EXTENSION_ROOT;
  if(extensionRoot){
    const root=resolve(extensionRoot).replaceAll('\\','/').replaceAll("'","''");
    appendFileSync(join(data,'postgresql.conf'),`\nshared_preload_libraries = '${root}/lib/timescaledb'\ndynamic_library_path = '${root}/lib;$libdir'\nextension_control_path = '${root}/share;$system'\n`);
  }
  if(coreMode&&!extensionRoot)appendFileSync(join(data,'postgresql.conf'),"\nshared_preload_libraries='timescaledb'\n");
  await run(executable('pg_ctl'),['-D',data,'-l',join(directory,'server.log'),'-w','-t','30','-o',`-h 127.0.0.1 -p ${port} -c shared_buffers=16MB -c max_connections=16`,'start']);started=true;
  const url=`postgresql://archie_test:${password}@127.0.0.1:${port}/postgres`;
  if(coreMode){
    const {Pool}=await import('pg');const pool=new Pool({connectionString:url,max:1});
    try{
      await pool.query(`CREATE ROLE archie_app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '${password}'`);
      await pool.query('CREATE DATABASE archieos_test OWNER archie_app TEMPLATE template0');
    }finally{await pool.end();}
    const setup=new Pool({connectionString:url.replace('/postgres','/archieos_test'),max:1});
    try{await setup.query("CREATE EXTENSION timescaledb VERSION '2.29.2'; CREATE EXTENSION vector VERSION '0.8.6'");}finally{await setup.end();}
    const applicationUrl=url.replace('archie_test:','archie_app:').replace('/postgres','/archieos_test');
    const packages=workspaceMode?['--workspace']:['-p','archie-state','-p','archie-core','-p','archied'];
    await run('cargo',['test','--manifest-path','../../core/Cargo.toml','--locked',...packages,'-j','1','--','--include-ignored','--test-threads=1'],{...process.env,ARCHIE_TEST_DATABASE_URL:applicationUrl,ARCHIE_TEST_ADMIN_DATABASE_URL:url,CARGO_INCREMENTAL:'0',CARGO_PROFILE_DEV_DEBUG:'0'});
    console.log('PASS: PostgreSQL-only Core/state/daemon tests with TimescaleDB and pgvector.');
  }else if(process.argv.includes('--owner')){
    await run(process.execPath,['--test','--test-concurrency=1','../owner-companion/tests/*.test.mjs'],{...process.env,ARCHIE_OWNER_TEST_DATABASE_URL:url});
  }else if(process.argv.includes('--cognitive')){
    if(process.env.ARCHIE_COGNITIVE_TEST_BINARY){
      await run(resolve(process.env.ARCHIE_COGNITIVE_TEST_BINARY),['--ignored','--test-threads=1'],{...process.env,ARCHIE_COGNITIVE_TEST_DATABASE_URL:url});
    }else{
    await run('cargo',['test','--manifest-path','../../core/Cargo.toml','--locked','-p','archie-postgres','--test','persistence','-j','1','--','--ignored','--test-threads=1'],{...process.env,ARCHIE_COGNITIVE_TEST_DATABASE_URL:url,CARGO_INCREMENTAL:'0',CARGO_PROFILE_DEV_DEBUG:'0'});
    }
    console.log('PASS: isolated PostgreSQL cognitive persistence integration.');
  }else{
  await run(process.execPath,['--test','--test-concurrency=1','tests/*.test.ts'],{...process.env,ARCHIE_DASHBOARD_TEST_DATABASE_URL:url,ARCHIE_DATA_EXTENSIONS_ENABLED:extensionRoot?'1':''});
  await run(process.execPath,['tests/browser.mjs'],{...process.env,ARCHIE_DASHBOARD_BROWSER_DATABASE_URL:url});
  console.log('PASS: isolated real PostgreSQL adapter and production browser integration.');
  }
}finally{
  if(started)await run(executable('pg_ctl'),['-D',data,'-m','fast','-w','stop']);
  // Retain local server diagnostics; no recursive cache cleanup is performed.
}
