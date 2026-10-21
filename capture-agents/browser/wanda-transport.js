/* Wanda content has its own pairing and memory queue; Sentry metadata is unchanged. */
(function (root) {
  const MAX_REQUEST = 256 * 1024;
  const MAX_QUEUE = 256;
  const encoder = new TextEncoder();
  const keys = ['schema_version', 'event_id', 'device_id', 'actor', 'author', 'source', 'chat_id', 'sender', 'source_message_id', 'sent_at_ms', 'observed_at_ms', 'text', 'media_omitted', 'coverage'];
  const boundedId = value => typeof value === 'string' && value.length > 0 && encoder.encode(value).length <= 192 && !/[\u0000-\u001f\u007f]/u.test(value);
  function validEvent(event) {
    return event && Object.keys(event).length === keys.length && keys.every(k => Object.hasOwn(event, k)) &&
      event.schema_version === 1 && ['event_id', 'device_id', 'chat_id', 'sender'].every(k => boundedId(event[k])) &&
      (event.source_message_id === null || boundedId(event.source_message_id)) &&
      event.actor === 'presumed_owner' &&
      ['owner', 'counterparty', 'system'].includes(event.author) &&
      event.source === 'whatsapp_visible_dom' && event.coverage === 'visible_chat' &&
      Number.isSafeInteger(event.observed_at_ms) && event.observed_at_ms >= 0 &&
      (event.sent_at_ms === null || (Number.isSafeInteger(event.sent_at_ms) && event.sent_at_ms > 0)) &&
      typeof event.text === 'string' && [...event.text].length <= 2000 &&
      typeof event.media_omitted === 'boolean' && (event.text.trim().length > 0 || event.media_omitted);
  }
  function allowed(config) {
    return config.wandaEnabled === true && config.wandaAllowPresumed === true && config.wandaSuspended !== true;
  }
  function endpoint(value) {
    try {
      const url = new URL(value);
      const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
      return !url.username && !url.password && url.pathname === '/v1/events' && !url.search && !url.hash &&
        (url.protocol === 'https:' || (url.protocol === 'http:' && local));
    } catch { return false; }
  }
  class Transport {
    constructor(readConfig, fetcher = fetch) { this.readConfig = readConfig; this.fetcher = fetcher; this.queue = []; this.busy = false; this.generation = 0; }
    get pending() { return this.queue.length; }
    clear() { this.queue.length = 0; this.generation++; if (this.abort) this.abort.abort(); }
    enqueue(event) {
      if (!validEvent(event)) return false;
      this.queue.push({ ...event });
      if (this.queue.length > MAX_QUEUE) this.queue.shift();
      return true;
    }
    async flush() {
      if (this.busy) return;
      this.busy = true;
      const generation = this.generation;
      try {
        const config = await this.readConfig();
        if (generation !== this.generation) return;
        if (!allowed(config) || !endpoint(config.wandaUrl) || !config.wandaToken) { this.clear(); return; }
        const batch = [];
        while (this.queue.length && batch.length < 32) {
          const event = this.queue[0];
          if (event.device_id !== config.deviceId) { this.queue.shift(); continue; }
          if (encoder.encode(JSON.stringify([...batch, event])).length > MAX_REQUEST) break;
          batch.push(this.queue.shift());
        }
        if (!batch.length) return;
        this.abort = new AbortController();
        const timer = setTimeout(() => this.abort?.abort(), 4000);
        try {
          await this.fetcher(config.wandaUrl, { method: 'POST', credentials: 'omit', redirect: 'error', cache: 'no-store',
            headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + config.wandaToken },
            signal: this.abort.signal, body: JSON.stringify(batch) });
        } finally { clearTimeout(timer); this.abort = null; }
      } catch { /* Failed bodies are dropped, never persisted or replayed by transport. */ }
      finally { this.busy = false; }
    }
  }
  const api = { Transport, validEvent, allowed, endpoint };
  root.WandaTransport = api;
  if (typeof module !== 'undefined') module.exports = api;
})(globalThis);
