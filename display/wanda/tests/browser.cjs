// Integration against the real Wanda executable and C++ graph authority.
// No existing browser profile, private chats, microphone or model is used.
const {test}=require('node:test'); const assert=require('node:assert/strict');
const {chromium}=require('playwright'); const fs=require('node:fs/promises');
const path=require('node:path'); const os=require('node:os'); const {spawn}=require('node:child_process');
test('real service pairs, renders graph, switches scope and fits a phone viewport',async()=>{
  const root=path.resolve(__dirname,'../../..');
  const binary=process.env.WANDA_TEST_BINARY || path.join(root,'core/target/debug/archie-wanda.exe');
  const graphBinary=process.env.WANDA_TEST_GRAPH_BINARY; assert.ok(graphBinary,'set WANDA_TEST_GRAPH_BINARY');
  const dir=await fs.mkdtemp(path.join(os.tmpdir(),'wanda-browser-fixture-'));
  const quote=value=>JSON.stringify(value.replaceAll('\\','/'));
  const personal=path.join(dir,'personal.tsv'),work=path.join(dir,'work.tsv'),token=path.join(dir,'owner.token');
  await fs.writeFile(personal,'v\t1\nowner\towner\nsubject\towner\towner\tTú (fixture)\nsubject\tana\tconsented_person\tAna (fixture)\nsubject\tbeto\tconsented_person\tBeto (fixture)\ntrait\tana\twanda.tag\tfamilia\towner_declared\t-\tfixture\ntrait\tana\tflower_preference\ttulipanes\tsubject_declared\t-\tfixture\nedge\towner\tana\tpartner\towner_declared\towner\tfixture\nhyp\towner\tbeto\tfriend\tmodel_hypothesis\t\tfixture\n');
  await fs.writeFile(work,'v\t1\nowner\towner\nsubject\towner\towner\tTú (trabajo fixture)\nsubject\tboss\tconsented_person\tJefe (fixture)\nedge\towner\tboss\tmanager\towner_declared\towner\tfixture\n');
  const config=path.join(dir,'wanda.toml');
  await fs.writeFile(config,`state_dir=${quote(path.join(dir,'state'))}\ngraph_binary=${quote(graphBinary)}\nmodels_dir=${quote(path.join(dir,'models'))}\noperator_token_file=${quote(token)}\nsources=[]\n[[domains]]\ndomain="personal/owner"\ngraph_path=${quote(personal)}\n[[domains]]\ndomain="work/fixture/house"\ngraph_path=${quote(work)}\n`);
  const processHandle=spawn(binary,['--config',config,'--listen','127.0.0.1:0'],{windowsHide:true,stdio:['ignore','ignore','pipe']});
  let browser;
  try {
    const address=await new Promise((resolve,reject)=>{
      let text='';const timeout=setTimeout(()=>reject(new Error('Wanda startup timeout')),30000);
      processHandle.stderr.on('data',chunk=>{text+=chunk;const match=text.match(/Wanda listening on (127\.0\.0\.1:\d+)/);if(match){clearTimeout(timeout);resolve(match[1]);}});
      processHandle.once('exit',code=>{clearTimeout(timeout);reject(new Error(`Wanda exited ${code}: ${text}`));});
    });
    browser=await chromium.launch({channel:process.env.WANDA_TEST_BROWSER || 'msedge',headless:true});
    const page=await browser.newPage({viewport:{width:1440,height:1080}});const errors=[];page.on('pageerror',error=>errors.push(error.message));
    await page.goto(`http://${address}/`);await page.locator('#pair-button').click();await page.locator('#token').fill((await fs.readFile(token,'utf8')).trim());await page.locator('#pair-form button[type=submit]').click();
    await page.waitForFunction(()=>document.querySelector('#person-count').textContent.includes('3 personas'));
    await page.locator('#scene [data-id="ana"]').click();assert.match(await page.locator('#profile').innerText(),/tulipanes/);
    await page.locator('#list-view').click();await page.locator('#search').fill('tulipanes');await page.waitForFunction(()=>document.querySelector('#person-count').textContent.includes('1 persona'));
    await page.locator('#search').fill('');await page.locator('#domain').selectOption('work/fixture/house');
    await page.waitForFunction(()=>document.querySelector('#person-count').textContent.includes('2 personas'));assert.doesNotMatch(await page.locator('#profile').innerText(),/tulipanes/);
    await page.locator('#domain').selectOption('personal/owner');await page.waitForFunction(()=>document.querySelector('#person-count').textContent.includes('3 personas'));
    await page.locator('#map-view').click();await page.locator('#scene [data-id="ana"]').click();
    const output=process.env.WANDA_QA_DIR || dir;await fs.mkdir(output,{recursive:true});
    await page.screenshot({path:path.join(output,'wanda-desktop.png'),fullPage:true});
    await page.setViewportSize({width:390,height:844});await page.locator('#list-view').click();
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'no horizontal overflow at phone width');
    await page.screenshot({path:path.join(output,'wanda-mobile.png'),fullPage:true});assert.deepEqual(errors,[]);
  } finally {await browser?.close();processHandle.kill();}
});
