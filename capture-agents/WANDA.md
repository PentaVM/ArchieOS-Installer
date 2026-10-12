# Wanda live content collectors

Wanda content has a separate pairing and HTTP transport. Sentry's metadata
`CaptureEvent`, `capture-event.schema.json`, `EventPoster.kt` and `background.js`
remain unchanged. Content is disabled by default. These collectors never send
messages or operate an independent WhatsApp session.

## Server integration

`POST /v1/events`, `Content-Type: application/json`, `Authorization: Bearer <source token>`.
The body is an array of 1–32 `protocols/wanda-event.schema.json` v1 envelopes,
at most 256 KiB. Text is at most 2000 Unicode code points; identifiers are at
most 192 UTF-8 bytes. Each memory queue contains at most 256 events. Failed
requests drop the drained batch. Redirects are refused. Remote endpoints must
use HTTPS; HTTP is accepted only for loopback.

The server must bind the token to the enrolled `device_id`, one source, an actor
policy, and one trusted domain. An event never selects its own domain. The
expected response is `{accepted,duplicates,rejected,reasons:[]}`; collectors
do not log or retain response bodies. The node exposes authoritative acceptance
and rejection status. Disabling or changing a pairing clears queued content;
in-flight requests are cancelled on a best-effort basis. Revocation on the server
remains authoritative for requests already delivered.

Events use source time in `sent_at_ms`, or `null` when unknown. `observed_at_ms`
is observation time, never a substitute for source time. SHA256 event IDs are
stable for replay and change for edited content. The bounded dedup memory holds
fingerprints, never bodies. Server dedup is still necessary after restarts or
after a fingerprint ages out.

## Browser

Load `capture-agents/browser` as an unpacked extension. The existing metadata
pairing is independent. In its popup enter the Wanda `/v1/events` URL and source
token, select **Enable visible WhatsApp content** and explicitly permit this
browser's **presumed-owner attribution**, then save. Saving displays the stable
device ID to enroll on the node. **Someone else is using this browser** suspends
content; **I am using this browser** resumes its configured presumed-owner
policy, without claiming biometric confirmation.

Configuration keys in `chrome.storage.local`:

- `deviceId`: existing enrolled identity shared with the metadata agent.
- `wandaUrl`, `wandaToken`: independent content endpoint and source token.
- `wandaEnabled`, `wandaAllowPresumed`: both default false and must be true.
- `wandaSuspended`: explicit guest suspension, checked again by the worker.

`wanda-background.js` imports the original metadata worker without editing it.
Only the extension's own active, top-frame `https://web.whatsapp.com/` tab can
submit content. The content script is limited to that origin and the selected
rendered chat. It watches DOM changes, visibility and scroll, then reads visible
incoming/outgoing message bubbles. It excludes drafts, contenteditable fields,
hidden rows, offscreen rows, quoted reply regions and non-message page text.
Group sender comes from the bubble's prefix, separately from the chat header.
Unknown senders remain `unknown`; locale timestamps without an epoch stay null.

Coverage is `visible_chat`. Unopened/offscreen/virtualized history, media-only
bubbles and a background inactive tab are unavailable. DOM selectors are based
on sanitized fixtures and may need adaptation after WhatsApp markup changes;
no private WhatsApp session was inspected. This collector does not prove a
real WhatsApp deployment or full account coverage.

## Android

Open the existing pairing screen, set the separate Wanda URL/token, enable
WhatsApp notification content, and confirm owner through the existing biometric
flow. Allowing presumed-owner content after that confirmation decays is an
explicit independent checkbox. The existing guest button suppresses content
and clears its queued bodies.

Separate `archie_wanda` SharedPreferences contain `endpoint`, `token`,
`enabled` and `allow_presumed`; the device ID remains the existing AgentConfig
enrollment. `WandaPoster` reads these dynamically and rechecks actor/config at
flush. It checks permission before extracting notification text. Its queue
flushes every two seconds, with four-second connect/read timeouts.

`NotificationCompat.MessagingStyle` yields individual senders, text and source
message timestamps. Conversation title stays separate from the sender. Summary
notifications are skipped. Reposts deduplicate by source/chat/sender/time/content.
If MessagingStyle is absent, big-text/text extras provide an incoming preview
with unknown send time. Empty/redacted text is not evidence. Coverage is
`notification_preview`, only for `com.whatsapp` and `com.whatsapp.w4b`.

Disabled/hidden/redacted notifications and outbound messages absent from the
notification stream are unavailable. Android notification access does not
provide full chat history. No real phone or notification feed was exercised.

## Verification

Node 22+ and Playwright with an installed Edge browser:

```powershell
node --test capture-agents/browser/tests/wanda.test.cjs
```

Set `NODE_PATH` to the bundled Playwright package directory when using Codex's
workspace runtime. Set `WANDA_TEST_BROWSER=chrome` to use an installed Chrome
instead of Edge. Tests run an isolated headless browser against synthetic DOM,
intercepting the fixture URL; they never use an existing profile or chat login.
The suite covers incoming/outgoing/group/draft/hidden/offscreen/repost/edit,
actor and source gates, request/memory bounds, failed delivery and TLS restrictions.

With JDK 17+, Android SDK 34 and Gradle 8.7 installed:

```powershell
gradle -p capture-agents/android testDebugUnitTest assembleDebug --max-workers=2
```

Android Robolectric tests cover real NotificationCompat serialization, group
sender separation, reposts/edits, summaries, fallback time, actor/config gates
and text caps. They are authored but unexecuted on this Windows host: Gradle,
Kotlin compiler and Android SDK are unavailable. Android compilation and device
proof remain required; Java availability alone does not establish either.

## IRIS producer seam

The schema additionally supports `source=iris_semantic`,
`coverage=semantic_observation`, and optional `monitor_id`/`window_id`.
Those events come from the existing IRIS/host producer. These collectors add no
screen capture, OCR, UIA or all-monitor replacement. Full desktop coverage depends
on the corresponding producer and its separately validated permissions/runtime.
