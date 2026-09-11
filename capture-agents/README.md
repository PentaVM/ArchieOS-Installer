# capture-agents

Per-device agents that feed the Archie capture ingest. **Status: IMPL (source), validated
on-device.**

These are the "todos mis dispositivos" half of ambient capture. Each agent runs on one of the
owner's own devices, observes the owner's own activity through the OS's supported live
streams, and posts it — as [`protocols/capture-event.schema.json`](../protocols/capture-event.schema.json)
events — to the owner's node, where the tested C++ gate in `sentry/capture` decides what
becomes of each event.

The agents are not in the Sentry CMake build: they are Kotlin and JavaScript, built with their
own toolchains and side-loaded onto the owner's devices. They are here, in the repo, because
they are half of one design and the event contract binds them to the tested other half.

## The honest capture path

There is no supported API for reading a personal WhatsApp or Instagram session, and driving a
headless web login violates those platforms' terms and gets accounts banned. So the agents do
not do that. They use the mechanisms the operating systems actually provide for live,
persistent observation:

| Device | Mechanism | Captures |
|---|---|---|
| Android | `NotificationListenerService` | WhatsApp, Instagram, Messenger, Telegram, SMS, Gmail message previews — live, as they arrive |
| Android | `BiometricPrompt` | the owner-present signal that gates everything |
| Browser | `webNavigation` (Manifest V3) | searches and page navigation |
| Desktop | *(not yet built)* | window/app focus; email via IMAP IDLE |

This is what "sesión abierta y persistente" is in practice: the OS's own live stream, not a
scraped login.

## The two rules the agents must honour

The gate on the node enforces these regardless, but the agents are built to report honestly so
the gate rarely has to suppress:

1. **Actor attribution.** Every event carries who was at the device. Android confirms the
   owner biometrically and decays the attribution over time; the browser reports only
   `presumed_owner`, because a laptop has no per-action owner check. When someone else is
   using the device — "someone else is using my phone" in the pairing screen — the agent
   stamps `confirmed_other`, and the gate suppresses it. **Detecting a guest stops capture; it
   never profiles them.**

2. **No content.** No agent sends a message body, a page body, a form field, or a password.
   The Android agent reads a notification only far enough to know a message happened and who
   with; the browser sends the fact of a search, not the query text. A capture stream that
   carried content would become the most damaging file on the device, and the schema has no
   field for it.

## linkearse solo a mí

Every agent talks to exactly one endpoint: the owner's node, configured at pairing with a URL
and a token. Nothing fans out to a cloud. The node approves each device as an enrollment, and
an event from a device the owner never enrolled is discarded before its content is considered.

## Building

**Android** (`android/`): `./gradlew assembleDebug`, then `adb install` the APK. Grant
*Notification access* in Settings, open the app, pair it to the node, and confirm you are the
owner. A persistent notification shows capture is running — you always know it is on.

**Browser** (`browser/`): load `browser/` as an unpacked extension (chrome://extensions →
Developer mode → Load unpacked), open the popup, and pair it to the node.

The node side — the HTTP endpoint that receives the NDJSON and hands it to `ingest_stream` — is
`archied`'s job, and is the small remaining wiring on the daemon.
