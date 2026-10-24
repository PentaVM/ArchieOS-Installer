/* Additive service-worker entry point: existing metadata worker remains unchanged. */
importScripts('background.js', 'wanda-transport.js');
const wanda = new WandaTransport.Transport(() => chrome.storage.local.get(['wandaUrl', 'wandaToken', 'wandaEnabled', 'wandaAllowPresumed', 'wandaSuspended', 'deviceId']));
chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (message?.type !== 'wanda-events') return;
  (async () => {
    const url = new URL(sender.url || 'about:blank');
    if (sender.id !== chrome.runtime.id || sender.frameId !== 0 || url.origin !== 'https://web.whatsapp.com' || !sender.tab?.active || !Array.isArray(message.events) || message.events.length > 32) return;
    const config = await wanda.readConfig();
    if (!WandaTransport.allowed(config)) { wanda.clear(); return; }
    for (const event of message.events) if (event.device_id === config.deviceId) wanda.enqueue(event);
    await wanda.flush();
  })().then(() => respond({ received: true }), () => respond({ received: false }));
  return true;
});
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && ['wandaEnabled', 'wandaAllowPresumed', 'wandaSuspended', 'wandaUrl', 'wandaToken', 'deviceId'].some(k => Object.hasOwn(changes, k))) wanda.clear();
});
chrome.alarms.create('wanda-flush', { periodInMinutes: 0.5 });
chrome.alarms.onAlarm.addListener(alarm => { if (alarm.name === 'wanda-flush') void wanda.flush(); });
