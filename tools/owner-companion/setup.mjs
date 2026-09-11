// Operator setup: files are private, create-only, and never print credentials.
import {readFileSync,mkdirSync,writeFileSync,existsSync,statSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {randomBytes} from 'node:crypto';
const [dbPath,profilePath,nodeId,output]=process.argv.slice(2);
if(!dbPath||!profilePath||!output||!/^[a-f0-9]{64}$/.test(nodeId||''))throw Error('usage: node setup.mjs <private-database.json> <wizard-profile.json> <node-id> <new-private-directory>');
for(const path of [dbPath,profilePath])if(statSync(path).size>65536)throw Error('configuration too large');
const db=JSON.parse(readFileSync(dbPath,'utf8')),profile=JSON.parse(readFileSync(profilePath,'utf8'));
if(typeof db.connection!=='string'||!profile.owner?.name?.trim()||profile.owner.name.length>80)throw Error('invalid database or owner profile');
if(!['postgres:','postgresql:'].includes(new URL(db.connection).protocol))throw Error('database connection must be a PostgreSQL URL');
const root=resolve(output);if(existsSync(root))throw Error('refusing to replace an existing directory');
mkdirSync(root,{mode:0o700});const bootstrapCode=randomBytes(32).toString('base64url');
writeFileSync(join(root,'owner.json'),JSON.stringify({connection:db.connection,nodeId,ownerName:profile.owner.name,bootstrapCode},null,2),{flag:'wx',mode:0o600});
writeFileSync(join(root,'activation'),bootstrapCode,{flag:'wx',mode:0o600});
console.log('Private owner configuration and activation file created. No credentials were printed.');
