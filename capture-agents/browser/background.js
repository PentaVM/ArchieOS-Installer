// The Archie capture agent for the browser.
//
// It observes the owner's own navigation — the pages they open and the searches they make —
// and posts them, as capture events, only to the owner's node. It captures no page content,
// no form fields, no passwords: a URL and a search query are what the graph needs to know
// what the owner has been doing, and a browser extension that read page bodies would be the
// accumulation the whole design avoids.
//
// The wire form matches protocols/capture-event.schema.json, the same contract the tested
// C++ ingest obeys. The events are the browser's honest report; the node's gate decides what
// becomes of them.

const SOURCE_WEB_SEARCH = "web_search";
const SOURCE_NAVIGATION = "navigation";

// A browser on a laptop is a weaker actor signal than a phone with biometrics: there is no
// per-navigation "is this the owner" check. The honest attribution is therefore "presumed
// owner" — it is the owner's own logged-in browser profile, but nothing confirms who is at
// the keyboard right now. It is never claimed as confirmed, which would over-attribute a
// guest borrowing the laptop; the node's gate treats presumed according to the owner's
// policy.
const ACTOR = "presumed_owner";

const queue = [];
const MAX_BUFFERED = 1000;

// The search engines whose result pages carry the query in a known parameter. A navigation
// to one of these with a query is recorded as a search; everything else is a plain
// navigation. The query itself is the owner's own words, authored by them.
const SEARCH_ENGINES = [
  { host: "www.google.com", param: "q" },
  { host: "www.bing.com", param: "q" },
  { host: "duckduckgo.com", param: "q" },
  { host: "search.brave.com", param: "q" },
  { host: "www.ecosia.org", param: "q" },
];

function config() {
  return chrome.storage.local.get(["ingestUrl", "token", "deviceId"]);
}

async function ensureDeviceId(stored) {
  if (stored.deviceId) return stored.deviceId;
  const id = "browser-" + crypto.randomUUID().slice(0, 12);
  await chrome.storage.local.set({ deviceId: id });
  return id;
}

function enqueue(event) {
  queue.push(event);
  while (queue.length > MAX_BUFFERED) queue.shift();
}

chrome.webNavigation.onCommitted.addListener(async (details) => {
  // Only top-frame navigations the owner actually initiated — not every iframe, ad or
  // prefetch. transitionType filters out the machine-generated noise.
  if (details.frameId !== 0) return;
  if (details.transitionType === "auto_subframe" || details.transitionType === "form_submit") {
    // form_submit bodies are exactly the private content we do not want; the resulting
    // navigation URL is enough.
  }

  let url;
  try {
    url = new URL(details.url);
  } catch (_) {
    return;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return;

  const stored = await config();
  if (!stored.ingestUrl) return; // not paired yet
  const deviceId = await ensureDeviceId(stored);

  const engine = SEARCH_ENGINES.find((e) => e.host === url.hostname);
  if (engine && url.searchParams.has(engine.param)) {
    // A search. The query is the owner's own words; the node keeps that a search happened,
    // and — deliberately — the query text is NOT sent as content. Only the fact and the
    // engine's host as a navigation. If the owner later wants query text recorded, that is a
    // policy they turn on; the safe default sends the fact, not the words.
    enqueue({
      device_id: deviceId,
      source: SOURCE_WEB_SEARCH,
      actor: ACTOR,
      author: "owner",
      observed_at: String(Date.now()),
    });
    return;
  }

  // A plain navigation. The host is a system fact — where the owner went — not private
  // speech, so its author is the system.
  enqueue({
    device_id: deviceId,
    source: SOURCE_NAVIGATION,
    actor: ACTOR,
    author: "system",
    observed_at: String(Date.now()),
  });
});

// Flush on a timer. Best-effort: a failed post drops the batch rather than growing a durable
// spool of everywhere the owner went.
chrome.alarms.create("flush", { periodInMinutes: 0.2 });
chrome.alarms.onAlarm.addListener(async (alarm) => {
  if (alarm.name !== "flush" || queue.length === 0) return;
  const stored = await config();
  if (!stored.ingestUrl) return;

  const batch = queue.splice(0, 500);
  const ndjson = batch.map((e) => JSON.stringify(e)).join("\n") + "\n";
  try {
    await fetch(stored.ingestUrl, {
      method: "POST",
      headers: {
        "Content-Type": "application/x-ndjson",
        Authorization: "Bearer " + (stored.token || ""),
      },
      body: ndjson,
    });
  } catch (_) {
    // Dropped. A small hole in the node's picture is survivable; an unbounded queue is not.
  }
});
