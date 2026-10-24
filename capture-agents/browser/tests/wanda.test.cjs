const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('playwright');
const collectorPath = path.join(__dirname, '..', 'wanda-content.js');
const transportPath = path.join(__dirname, '..', 'wanda-transport.js');

test('visible WhatsApp collector preserves group sender, direction and excludes drafts/hidden rows', { timeout: 120000 }, async () => {
  assert.ok(fs.existsSync(collectorPath), 'the visible WhatsApp collector must exist');
  const browser = await chromium.launch({ channel: process.env.WANDA_TEST_BROWSER || 'msedge', headless: true, timeout: 60000 });
  try {
    const page = await browser.newPage();
    page.setDefaultTimeout(10000);
    page.setDefaultNavigationTimeout(10000);
    await page.route('https://wanda-fixture.test/**', route => route.fulfill({ contentType: 'text/html', body: '<html></html>' }));
    await page.goto('https://wanda-fixture.test/');
    await page.setContent(`<div id="main"><header><button title="Open profile">Profile</button><span title="Family">Family</span></header>
      <div class="message-in" data-id="false_120363042000000001@g.us_3EB0A1_34600000001@c.us"><div data-pre-plain-text="[10:30, 12/10/2026] Ana: "><span class="selectable-text">Meet tomorrow</span></div></div>
      <div class="message-out" data-id="true_120363042000000001@g.us_3EB0A2"><div data-pre-plain-text="[10:31, 12/10/2026] Me: "><span class="selectable-text">I can come</span></div></div>
      <div class="message-in" style="display:none" data-id="hidden"><span class="selectable-text">Hidden private text</span></div>
      <div class="message-in" data-id="draft"><div contenteditable="true"><span class="selectable-text">Unsent draft</span></div></div>
      <div class="message-in" style="position:absolute;top:9999px" data-id="offscreen"><span class="selectable-text">Offscreen</span></div>
      <footer contenteditable="true"><span class="selectable-text">Draft outside messages</span></footer></div>`);
    await page.addScriptTag({ path: collectorPath });
    const result = await page.evaluate(() => WandaContent.visibleMessages(document));
    assert.equal(result.length, 2);
    assert.deepEqual(result.map(m => [m.chat_id, m.sender, m.author, m.text]), [
      ['120363042000000001@g.us', 'Ana', 'counterparty', 'Meet tomorrow'], ['120363042000000001@g.us', 'Me', 'owner', 'I can come']
    ]);
    // Without a rendered address the title is sent as a label, which never routes.
    assert.equal(await page.evaluate(() => {
      const main = document.createElement('div');
      main.innerHTML = '<header><span title="Trabajo">Trabajo</span></header><div data-id="false_group_1"></div>';
      return WandaContent.conversationOf(main);
    }), 'title:Trabajo');
    assert.equal(result[0].sent_at_ms, null, 'locale DOM time must not be guessed as epoch');
    assert.equal(result[0].source_message_id, 'false_120363042000000001@g.us_3EB0A1_34600000001@c.us');
    const gates = await page.evaluate(async () => {
      const rows = WandaContent.visibleMessages(document);
      const collector = new WandaContent.Collector();
      const config = { deviceId: 'browser-fixture', wandaEnabled: true, wandaAllowPresumed: true, wandaSuspended: false };
      const first = await collector.collect(rows, config, 1790000000000);
      const replay = await collector.collect(rows, config, 1790000000001);
      rows[0].text = 'Edited plan';
      const edited = await collector.collect(rows, config, 1790000000002);
      return { first, replay, edited,
        guest: await collector.collect(rows, { ...config, wandaSuspended: true }, 1790000000003),
        disabled: await new WandaContent.Collector().collect(rows, { ...config, wandaEnabled: false }, 1790000000003),
        unconfirmed: await new WandaContent.Collector().collect(rows, { ...config, wandaAllowPresumed: false }, 1790000000003) };
    });
    assert.equal(gates.first.length, 2);
    assert.equal(gates.replay.length, 0);
    assert.equal(gates.edited.length, 1);
    assert.notEqual(gates.first[0].event_id, gates.edited[0].event_id, 'edited content is fresh evidence');
    assert.equal(gates.first[0].source, 'whatsapp_visible_dom');
    assert.equal(gates.first[0].coverage, 'visible_chat');
    assert.ok(!Object.hasOwn(gates.first[0], 'domain'));
    assert.deepEqual([gates.guest, gates.disabled, gates.unconfirmed], [[], [], []]);
    await page.evaluate(() => { document.querySelector('#main').remove(); });
    assert.deepEqual(await page.evaluate(() => WandaContent.visibleMessages(document)), []);
  } finally { await browser.close(); }
});

test('disabled or guest content is gated before the content script reads DOM', async () => {
  const vm = require('node:vm');
  for (const flags of [{wandaEnabled:false}, {wandaAllowPresumed:false}, {wandaSuspended:true}]) {
    let reads = 0, scheduled;
    const document = { visibilityState:'visible', defaultView:{}, documentElement:{}, addEventListener(){}, querySelector(){ reads++; return null; } };
    const chrome = { runtime:{ id:'fixture-extension', sendMessage:async () => ({received:true}) }, storage:{ local:{ get:async () => ({
      deviceId:'browser-fixture', wandaEnabled:true, wandaAllowPresumed:true, wandaSuspended:false,
      wandaUrl:'https://owner.invalid/v1/events', wandaToken:'fixture-token', ...flags }) }, onChanged:{addListener(){}} } };
    const context = vm.createContext({document,chrome, TextEncoder, location:{origin:'https://web.whatsapp.com'},
      window:{addEventListener(){}}, MutationObserver:class { observe(){} },
      setTimeout:callback => { scheduled=callback; return 1; }, clearTimeout(){}, crypto:require('node:crypto').webcrypto });
    vm.runInContext(fs.readFileSync(collectorPath, 'utf8'), context);
    scheduled();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(reads, 0, 'content flags must be checked before visibleMessages is called');
  }
});

test('content transport caps memory and request, rechecks actor enablement and never retries bodies', async () => {
  assert.ok(fs.existsSync(transportPath), 'a separate Wanda transport must exist');
  const { Transport } = require(transportPath);
  const config = { wandaUrl: 'http://127.0.0.1:8769/v1/events', wandaToken: 'fixture-token', wandaEnabled: true, wandaAllowPresumed: true, wandaSuspended: false, deviceId: 'browser-fixture' };
  const calls = [];
  const transport = new Transport(async () => config, async (url, init) => { calls.push({url, init}); return { ok: true }; });
  const event = i => ({ schema_version: 1, event_id: `event-${i}`, device_id: config.deviceId, actor: 'presumed_owner', author: 'counterparty', source: 'whatsapp_visible_dom', chat_id: 'chat', sender: 'Ana', source_message_id: null, sent_at_ms: null, observed_at_ms: 1790000000000, text: '🙂'.repeat(2000), media_omitted: false, coverage: 'visible_chat' });
  for (let i=0;i<300;i++) assert.equal(transport.enqueue(event(i)), true);
  assert.equal(transport.pending, 256);
  await transport.flush();
  assert.equal(calls.length, 1);
  const body = JSON.parse(calls[0].init.body);
  assert.ok(body.length <= 32);
  assert.ok(Buffer.byteLength(calls[0].init.body) <= 256 * 1024);
  assert.equal(body[0].event_id, 'event-44');
  assert.equal(calls[0].init.headers.Authorization, 'Bearer fixture-token');
  assert.equal(calls[0].url, config.wandaUrl);
  config.wandaSuspended = true;
  await transport.flush();
  assert.equal(transport.pending, 0);
  assert.equal(calls.length, 1, 'guest suspension clears queued bodies');
  config.wandaSuspended = false;
  assert.equal(transport.enqueue({ ...event(301), text: 'x'.repeat(2001) }), false);
  assert.equal(transport.enqueue({ ...event(301), actor: 'confirmed_other' }), false);
  assert.equal(transport.enqueue({ ...event(301), actor: 'confirmed_owner' }), false, 'a browser must never upgrade actor attribution');
  assert.equal(transport.enqueue({ ...event(301), sender: 'é'.repeat(97) }), false, 'ID bound counts UTF-8 bytes');
  assert.equal(transport.enqueue({ ...event(301), domain: 'work/other' }), false);
  const failed = new Transport(async () => config, async () => { throw Error('offline'); });
  failed.enqueue(event(302));
  await failed.flush();
  assert.equal(failed.pending, 0);
  config.wandaUrl = 'http://192.0.2.1/v1/events';
  transport.enqueue(event(303));
  await transport.flush();
  assert.equal(calls.length, 1, 'remote cleartext content endpoints are refused');
});

test('worker accepts only active top-frame WhatsApp content from its own extension and pairing', async () => {
  const vm = require('node:vm');
  const handlers = {}, posts = [];
  const config = { wandaUrl: 'https://owner.invalid/v1/events', wandaToken: 'fixture-token', wandaEnabled: true, wandaAllowPresumed: true, wandaSuspended: false, deviceId: 'browser-fixture' };
  const chrome = { runtime: { id: 'fixture-extension', onMessage: { addListener: listener => handlers.message = listener } },
    storage: { local: { get: async () => ({ ...config }) }, onChanged: { addListener: listener => handlers.changed = listener } },
    alarms: { create() {}, onAlarm: { addListener() {} } }, webNavigation: { onCommitted: { addListener() {} } } };
  const context = vm.createContext({ chrome, TextEncoder, URL, AbortController, setTimeout, clearTimeout,
    fetch: async (url, init) => { posts.push({url, init}); return { ok: true }; }, crypto: require('node:crypto').webcrypto });
  context.importScripts = (...files) => files.forEach(file => vm.runInContext(fs.readFileSync(path.join(__dirname, '..', file), 'utf8'), context));
  vm.runInContext(fs.readFileSync(path.join(__dirname, '..', 'wanda-background.js'), 'utf8'), context);
  const event = { schema_version: 1, event_id: 'fixture', device_id: config.deviceId, actor: 'presumed_owner', author: 'counterparty', source: 'whatsapp_visible_dom', chat_id: 'chat', sender: 'Ana', source_message_id: null, sent_at_ms: null, observed_at_ms: 1790000000000, text: 'Hello', media_omitted: false, coverage: 'visible_chat' };
  const sender = { id: chrome.runtime.id, frameId: 0, url: 'https://web.whatsapp.com/', tab: { active: true } };
  const send = (source, value = event) => new Promise(resolve => handlers.message({ type: 'wanda-events', events: [value] }, source, resolve));
  for (const bad of [{...sender, url:'https://other.invalid/'}, {...sender, id:'another-extension'}, {...sender, frameId:1}, {...sender, tab:{active:false}}]) await send(bad);
  await send(sender, { ...event, device_id: 'unpaired-device' });
  assert.equal(posts.length, 0);
  await send(sender);
  assert.equal(posts.length, 1);
  assert.equal(JSON.parse(posts[0].init.body)[0].source, 'whatsapp_visible_dom');
  config.wandaSuspended = true;
  handlers.changed({ wandaSuspended: { newValue: true } }, 'local');
  await send(sender);
  assert.equal(posts.length, 1);
});
