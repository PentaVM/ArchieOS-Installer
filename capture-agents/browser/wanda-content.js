/* Read-only collector for rendered messages in the owner's selected WhatsApp chat. */
(function (root) {
  const encoder = new TextEncoder();
  const bounded = value => typeof value === 'string' && value.length > 0 && encoder.encode(value).length <= 192 && !/[\u0000-\u001f\u007f]/u.test(value);
  const textOf = element => (element?.textContent || '').trim();
  function visible(element, view) {
    if (element.closest('[contenteditable]:not([contenteditable="false"]), [hidden], [aria-hidden="true"]')) return false;
    for (let node = element; node instanceof view.Element; node = node.parentElement) {
      const style = view.getComputedStyle(node);
      if (style.display === 'none' || style.visibility !== 'visible' || style.opacity === '0') return false;
    }
    const box = element.getBoundingClientRect();
    return box.width > 0 && box.height > 0 && box.bottom > 0 && box.right > 0 && box.top < view.innerHeight && box.left < view.innerWidth;
  }
  function visibleMessages(document) {
    const view = document.defaultView;
    if (!view || document.visibilityState !== 'visible') return [];
    const main = document.querySelector('#main');
    if (!main) return [];
    const chat = main.querySelector('header span[title]');
    const chatId = chat?.getAttribute('title')?.trim();
    if (!bounded(chatId)) return [];
    const result = [];
    for (const bubble of main.querySelectorAll('.message-in, .message-out')) {
      if (result.length >= 256) break;
      if (!visible(bubble, view) || bubble.querySelector('[contenteditable]:not([contenteditable="false"])')) continue;
      const outgoing = bubble.classList.contains('message-out');
      if (outgoing && bubble.classList.contains('message-in')) continue;
      const parts = [...bubble.querySelectorAll('.selectable-text')].filter(node => visible(node, view) && !node.closest('[data-testid*="quoted"], [data-testid*="reply"]'));
      const text = parts.map(textOf).filter(Boolean).join('\n');
      if (!text || [...text].length > 2000) continue;
      const nativeId = (bubble.getAttribute('data-id') || bubble.closest('[data-id]')?.getAttribute('data-id') || '').trim();
      if (nativeId && !bounded(nativeId)) continue;
      const prefix = bubble.querySelector('[data-pre-plain-text]')?.getAttribute('data-pre-plain-text');
      const sender = prefix?.match(/\]\s*(.+):\s*$/u)?.[1]?.trim() || (outgoing ? 'self' : 'unknown');
      if (!bounded(sender)) continue;
      // WhatsApp's locale-formatted label lacks an unambiguous epoch/timezone.
      // Only a native, explicit epoch timestamp is admitted; otherwise time stays unknown.
      const rawTime = bubble.getAttribute('data-timestamp');
      const timestamp = rawTime && /^[0-9]{13}$/u.test(rawTime) ? Number(rawTime) : null;
      result.push({ chat_id: chatId, sender, author: outgoing ? 'owner' : 'counterparty',
        source_message_id: nativeId || null, sent_at_ms: timestamp, text, media_omitted: false });
    }
    return result;
  }
  async function digest(value) {
    const bytes = await crypto.subtle.digest('SHA-256', encoder.encode(value));
    return [...new Uint8Array(bytes)].map(b => b.toString(16).padStart(2, '0')).join('');
  }
  class Collector {
    constructor() { this.seen = new Set(); this.generation = 0; }
    clear() { this.seen.clear(); this.generation++; }
    async collect(messages, config, observedAt) {
      if (config.wandaEnabled !== true || config.wandaAllowPresumed !== true || config.wandaSuspended === true || !bounded(config.deviceId)) return [];
      const generation = this.generation;
      const result = [];
      for (const message of messages.slice(0, 256)) {
        const id = await digest(JSON.stringify([config.deviceId, message.chat_id, message.sender, message.author, message.source_message_id, message.sent_at_ms, message.text]));
        if (generation !== this.generation) return [];
        if (this.seen.has(id)) continue;
        this.seen.add(id);
        if (this.seen.size > 2048) this.seen.delete(this.seen.values().next().value);
        result.push({ schema_version: 1, event_id: id, device_id: config.deviceId, actor: 'presumed_owner',
          author: message.author, source: 'whatsapp_visible_dom', chat_id: message.chat_id, sender: message.sender,
          source_message_id: message.source_message_id, sent_at_ms: message.sent_at_ms, observed_at_ms: observedAt,
          text: message.text, media_omitted: message.media_omitted, coverage: 'visible_chat' });
      }
      return result;
    }
  }
  root.WandaContent = { visibleMessages, Collector };
  if (typeof chrome === 'undefined' || !chrome.runtime?.id || location.origin !== 'https://web.whatsapp.com') return;
  const collector = new Collector();
  let busy = false, rerun = false, timer;
  const configKeys = ['deviceId', 'wandaEnabled', 'wandaAllowPresumed', 'wandaSuspended', 'wandaUrl', 'wandaToken'];
  async function scan() {
    if (busy) { rerun = true; return; }
    busy = true;
    try {
      const config = await chrome.storage.local.get(configKeys);
      if (document.visibilityState !== 'visible' || !config.wandaUrl || !config.wandaToken ||
          config.wandaEnabled !== true || config.wandaAllowPresumed !== true || config.wandaSuspended === true) return;
      const generation = collector.generation;
      const events = await collector.collect(visibleMessages(document), config, Date.now());
      if (generation !== collector.generation) return;
      for (let offset = 0; offset < events.length; offset += 32) {
        await chrome.runtime.sendMessage({ type: 'wanda-events', events: events.slice(offset, offset + 32) });
      }
    } catch { /* A disconnected/reloaded extension drops this observation. */ }
    finally { busy = false; if (rerun) { rerun = false; schedule(); } }
  }
  function schedule() { if (!timer) timer = setTimeout(() => { timer = null; void scan(); }, 200); }
  new MutationObserver(schedule).observe(document.documentElement, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['data-id', 'data-pre-plain-text', 'style', 'class', 'hidden', 'aria-hidden'] });
  document.addEventListener('visibilitychange', schedule);
  window.addEventListener('scroll', schedule, { capture: true, passive: true });
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === 'local' && configKeys.some(k => Object.hasOwn(changes, k))) { collector.clear(); schedule(); }
  });
  schedule();
})(globalThis);
