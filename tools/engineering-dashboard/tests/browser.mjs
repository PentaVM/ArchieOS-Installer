// Production HTTP + isolated-browser checks. Uses generated credentials only.
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {randomBytes,randomUUID} from 'node:crypto';
import {spawn} from 'node:child_process';
import {mkdtempSync,readdirSync,readFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
const require=createRequire(import.meta.url);
const boundedFetch=globalThis.fetch;
globalThis.fetch=(input,options={})=>boundedFetch(input,{...options,signal:options.signal??AbortSignal.timeout(10000)});
const {chromium}=require(process.env.ARCHIE_PLAYWRIGHT_MODULE??'playwright');
assert.ok(process.env.ARCHIE_DASHBOARD_BROWSER_DATABASE_URL,'explicit PostgreSQL browser fixture required');
const password=randomBytes(24).toString('hex'),ingest=randomBytes(32).toString('hex');
const directory=mkdtempSync(join(tmpdir(),'archie-dashboard-browser-'));
const origin='http://127.0.0.1:3078';
const child=spawn(process.execPath,['node_modules/next/dist/bin/next','start','--hostname','127.0.0.1','--port','3078'],{
  cwd:process.cwd(),windowsHide:true,stdio:['ignore','pipe','pipe'],env:{...process.env,
    ARCHIE_DASHBOARD_PASSWORD:password,ARCHIE_DASHBOARD_SESSION_SECRET:randomBytes(32).toString('hex'),
    ARCHIE_DASHBOARD_INGEST_SECRET:ingest,ARCHIE_DASHBOARD_ORIGIN:origin,
    ARCHIE_DASHBOARD_DATABASE_URL:process.env.ARCHIE_DASHBOARD_BROWSER_DATABASE_URL}});
let output='';child.stdout.on('data',b=>{output=(output+b).slice(-4000)});child.stderr.on('data',b=>{output=(output+b).slice(-4000)});
let browser;
try{
  let ready=false;for(let i=0;i<90;i++){try{if((await fetch(`${origin}/login`)).ok){ready=true;break;}}catch{}await new Promise(r=>setTimeout(r,500));}
  assert.ok(ready,'production server must become ready');
  console.log('Browser check: HTTP access and ingest boundaries.');
  assert.equal((await fetch(`${origin}/api/status`)).status,401);
  assert.equal((await fetch(`${origin}/api/ingest`,{method:'POST',body:'{}'})).status,401);
  const post=body=>fetch(`${origin}/api/ingest`,{method:'POST',headers:{authorization:`Bearer ${ingest}`,'content-type':'application/json'},body:JSON.stringify(body)});
  const event={schema_version:1,event_id:randomUUID(),timestamp:new Date().toISOString(),phase:'TEST',module:'browser-security',status:'pass',summary:`<script>window.compromised=true</script> ${password}`,maturity:'VALIDATED',checks:{unit:{status:'pass',command:'node --test tests/security.test.ts'}},files:[{path:'../../private.key',kind:'file',state:'ignored'}]};
  assert.equal((await post(event)).status,200);assert.equal((await (await post(event)).json()).result,'duplicate');
  assert.equal((await post({...event,summary:'collision'})).status,409);
  assert.equal((await post({...event,event_id:randomUUID(),raw_audio:'forbidden'})).status,400);
  assert.equal((await fetch(`${origin}/api/login`,{method:'POST',headers:{origin:'https://foreign.test'},body:'password=x'})).status,403);
  console.log('Browser check: isolated Chrome authentication and views.');
  browser=await chromium.launch({channel:'chrome',headless:true});
  const context=await browser.newContext(),page=await context.newPage();
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(`${origin}/repo`);assert.equal(new URL(page.url()).pathname,'/login');
  await page.locator('input[name="password"]').fill(password);
  const loginResponse=page.waitForResponse(r=>r.url()===`${origin}/api/login`);
  await page.getByRole('button',{name:/entrar|acceder/i}).click();
  const result=await loginResponse;assert.equal(result.status(),303,`login failed: ${await result.text().catch(()=>'(redirect)')}`);
  await page.waitForURL(`${origin}/`);
  assert.equal(await page.evaluate(()=>window.compromised),undefined);
  assert.ok((await page.locator('main').innerText()).includes('<script>window.compromised=true</script>'));
  assert.ok(!(await page.content()).includes(password));
  const cookies=await context.cookies(),session=cookies.find(c=>c.name==='archie_dashboard_session');
  assert.ok(session?.httpOnly);assert.equal(session.sameSite,'Strict');
  for(const section of ['milestones','builds','tests','benchmarks','modules','repo','git','runtime','ipc','database','replay','security','deployments']){
    const response=await page.goto(`${origin}/${section}`);assert.equal(response.status(),200);await page.locator('h1').waitFor();
  }
  await page.goto(`${origin}/repo`);assert.ok((await page.locator('main').innerText()).includes('../../private.key'));
  await page.setViewportSize({width:390,height:844});assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.screenshot({path:join(directory,'dashboard-mobile.png'),fullPage:true});
  await page.setViewportSize({width:1440,height:960});await page.goto(origin);
  await page.screenshot({path:join(directory,'dashboard-desktop.png'),fullPage:true});
  await page.goto(`${origin}/modules`);assert.ok((await page.locator('main').innerText()).includes('VALIDATED'));
  await page.locator('details summary').click();assert.ok((await page.locator('main').innerText()).includes('node --test'));
  await page.screenshot({path:join(directory,'dashboard-matrix.png'),fullPage:true});
  const anonymous=await browser.newContext();assert.equal((await anonymous.request.get(`${origin}/api/status`)).status(),401);await anonymous.close();
  await Promise.all([page.waitForURL(`${origin}/login`),page.getByRole('button',{name:'Cerrar sesión'}).click()]);
  assert.equal((await fetch(`${origin}/api/status`,{headers:{cookie:`archie_dashboard_session=${session.value}`}})).status,401);
  assert.deepEqual(errors,[]);
  console.log('Browser check: client artifacts and final evidence.');
  for(const name of readdirSync('.next/static',{recursive:true}).filter(n=>/\.(js|html|json)$/.test(n)))assert.ok(!readFileSync(resolve('.next/static',name),'utf8').includes(password));
  console.log('PASS: production authentication, 14 views, API isolation, CSRF, revocation, XSS escaping, payload validation, responsive overflow and client-secret checks.');
  console.log(`Local visual evidence: ${directory}`);
}finally{
  if(browser)await browser.close();child.kill();
  await new Promise(done=>{if(child.exitCode!==null)return done();child.once('close',done);setTimeout(done,5000).unref();});
}
