// Development-only PostgreSQL node. No system service, existing cluster, or
// pre-existing configuration is modified. All credentials stay in ignored files.
import {spawn} from 'node:child_process';
import {mkdirSync,existsSync,readFileSync,writeFileSync,appendFileSync,unlinkSync,cpSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {join,resolve} from 'node:path';
import {randomBytes} from 'node:crypto';
import {createServer} from 'node:net';
import {Pool} from 'pg';
const repo=fileURLToPath(new URL('../../../',import.meta.url));
const root=join(repo,'.dashboard-local','local-node');
const data=join(root,'postgres'),node=join(root,'node');
const marker=join(root,'owner.json'),operator=join(root,'operator.json');
const secrets=[];
function save(path,value){writeFileSync(path,JSON.stringify(value,null,2),{flag:'wx',mode:0o600});}
async function run(command,args){
  const child=spawn(command,args,{windowsHide:true,shell:false,stdio:['ignore','pipe','pipe']});
  for(const pipe of [child.stdout,child.stderr])pipe.on('data',b=>{
    let text=b.toString();for(const secret of secrets)text=text.split(secret).join('[REDACTED]');process.stdout.write(text);
  });
  const background=/pg_ctl(?:\.exe)?$/.test(command);
  const code=await new Promise((done,reject)=>{child.once('error',reject);child.once(background?'exit':'close',done);});
  if(background){child.stdout.destroy();child.stderr.destroy();}
  if(code!==0)throw Error('Local node command failed');
}
async function main(){
  const action=process.argv[2];
  if(!['start','stop','check'].includes(action))throw Error('Use start, stop or check');
  let settings;
  if(existsSync(marker)){
    settings=JSON.parse(readFileSync(marker,'utf8'));
    if(settings.version!==1||!existsSync(join(data,'PG_VERSION')))throw Error('Incomplete local cluster; operator inspection required');
  }else{
    if(action!=='start'||existsSync(root))throw Error('Refusing to replace an unrecognized local directory');
    if(!process.env.ARCHIE_POSTGRES_BIN||!process.env.ARCHIE_POSTGRES_EXTENSION_ROOT)throw Error('Set PostgreSQL bin and staged extension root');
    const server=createServer();await new Promise(done=>server.listen(0,'127.0.0.1',done));
    const port=server.address().port;await new Promise(done=>server.close(done));
    settings={version:1,port,bin:resolve(process.env.ARCHIE_POSTGRES_BIN),extensionRoot:resolve(process.env.ARCHIE_POSTGRES_EXTENSION_ROOT)};
    mkdirSync(root,{mode:0o700});save(marker,settings);
    const password=randomBytes(32).toString('hex');secrets.push(password);
    const passwordFile=join(root,'init-password');writeFileSync(passwordFile,password,{flag:'wx',mode:0o600});
    await run(join(settings.bin,'initdb'),['-D',data,'--username=archie_operator','--auth=scram-sha-256',`--pwfile=${passwordFile}`,'--encoding=UTF8','--no-locale']);
    unlinkSync(passwordFile);
    const extension=settings.extensionRoot.replaceAll('\\','/').replaceAll("'","''");
    appendFileSync(join(data,'postgresql.conf'),`\nlisten_addresses='127.0.0.1'\nport=${port}\nshared_buffers='32MB'\nmax_connections=16\nshared_preload_libraries='${extension}/lib/timescaledb'\ndynamic_library_path='${extension}/lib;$libdir'\nextension_control_path='${extension}/share;$system'\ntimescaledb.telemetry_level='off'\n`);
    save(operator,{connection:`postgresql://archie_operator:${password}@127.0.0.1:${port}/archieos`,schema:'archie_node'});
  }
  const pgctl=join(settings.bin,'pg_ctl');
  if(action==='stop'){await run(pgctl,['-D',data,'-m','fast','-w','stop']);return;}
  if(action==='start'&&!existsSync(join(data,'postmaster.pid')))await run(pgctl,['-D',data,'-l',join(root,'server.log'),'-w','start']);
  const privateSettings=JSON.parse(readFileSync(operator,'utf8'));
  secrets.push(new URL(privateSettings.connection).password);
  const application=join(node,'etc','database.json');
  if(!existsSync(application)){
    const admin=new Pool({connectionString:privateSettings.connection.replace('/archieos','/postgres'),max:1});
    const password=randomBytes(32).toString('hex');secrets.push(password);
    try{
      await admin.query(`CREATE ROLE archie LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD '${password}'`);
      await admin.query('CREATE DATABASE archieos OWNER archie TEMPLATE template0');
      await admin.query('REVOKE ALL ON DATABASE archieos FROM PUBLIC');
    }finally{await admin.end();}
    const provision=new Pool({connectionString:privateSettings.connection,max:1});
    try{await provision.query("CREATE EXTENSION timescaledb VERSION '2.29.2'; CREATE EXTENSION vector VERSION '0.8.6'");}finally{await provision.end();}
    mkdirSync(join(node,'etc'),{recursive:true,mode:0o700});
    save(application,{connection:`postgresql://archie:${password}@127.0.0.1:${settings.port}/archieos`,schema:'archie_node'});
    cpSync(join(repo,'modules'),join(node,'etc','modules'),{recursive:true,errorOnExist:true,force:false});
  }
  await run(join(repo,'core','target','debug','archie-db'),['check',application]);
  await run(join(repo,'core','target','debug','archied'),['--root',node,'--check']);
  console.log('PASS: persistent PostgreSQL node booted with the application role.');
}
main().catch(()=>{console.error('Local node operation failed; inspect its private diagnostics. No existing data was replaced.');process.exitCode=1;});
