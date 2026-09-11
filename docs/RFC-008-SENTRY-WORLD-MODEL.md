# RFC-008 — Sentry: the analytical world model

- **Status:** S0 landed. Contracts and invariants implemented and gated in CI; no
  providers, no rendering, no camera I/O yet.
- **Scope:** the types and rules Sentry's world model is built on, and the four
  constitutional invariants that are enforced mechanically rather than documented.
- **Language:** C++23. `archied` stays Rust and stays the authority boundary; this RFC
  does not propose migrating it.

## 1. What Sentry is, and what it is not

Sentry is Archie's analytical world-intelligence subsystem: a persistent,
provenance-first world model fusing open data, operator-authorized sensors,
Earth observation, orbital catalogues, aviation, maritime, cyber, markets and local
telemetry.

The globe is one renderer. It is not the product.

The hierarchy is fixed and one-directional:

```text
raw observations → normalization → time/geo/entity alignment → statistics
→ specialist analytics → fusion → evidence graph → world state → generative reasoning
```

Generative models sit at the end, and they explain, summarize, hypothesise and propose
research. They do not write world state. §4 is that rule.

## 2. Why the contracts came before the providers

The obvious first slice is an HTTP client and a USGS adapter — something visible.

It was the wrong place to start. Every rule that makes this system trustworthy is a rule
about what may be *believed*, and those rules are cheap to write while there is nothing
to break and nearly impossible to retrofit once ten provider adapters have each invented
their own idea of a timestamp. A confidence float added in week one is a confidence float
in every table in year two.

So S0 is contracts, and it has no dependencies at all: no libcurl, no simdjson, no GDAL.
Rules that need a network stack to be checked are rules that eventually go unchecked.

## 3. The four invariants, and where they live

| # | Invariant | Enforced in |
|---|---|---|
| 1 | Operator camera authority is deterministic | `camera::refuse` |
| 2 | Identity resolution is consent-gated | `policy::refuse` |
| 3 | Sensor health is not authorization | `camera::CameraAuthority` — by having no parameter through which health could reach it |
| 4 | Generative output is never verified world state | `world::refuse` |

Each is a function that returns *why* it refused, never a bool. "Refused" is not
actionable; "a generative model tried to mark this verified" tells an operator exactly
what happened.

### 3.1 Invariant 4 — the constitutional rule

`Actor::GenerativeModel` may write exactly one epistemic state: `ArchieHypothesis`. It
may not mark anything verified, corroborated or inferred; it may not retract a fact it
disagrees with; it may not resolve a dispute. The check runs before every other check, so
no combination of conditions can route around it.

A hypothesis is also a dead end rather than a rung on the ladder. Not even the fusion
engine may promote one — the way a hypothesis becomes world state is that something goes
and observes the thing, producing a *new* statement with its own provenance.

Corroboration carries an evidential floor of two sources that are independent **at the
root of their provenance chain**. Four outlets running one wire story arrive at that
check as one source, which is the entire point of counting roots rather than documents.

`ProvenanceGraph::independent_source_count` is what computes it. That matters more than
it looks: the floor takes an integer, and until the graph existed that integer was a
caller's promise — which made the most important rule in the fusion engine a matter of
trust in whoever was calling it. Counting rows finds four; walking to the roots finds one.

Three cases the walk has to get right, and each errs deliberately:

- **A syndicated record resolves to what it credits.** Ten outlets carrying one wire
  story collapse to the wire service.
- **An uncredited copy counts as the outlet itself.** It might be original reporting, and
  refusing to count it would discard real evidence.
- **A cycle reports no roots.** Provenance is assembled from provider claims and cannot be
  trusted acyclic; reporting none refuses at the floor rather than accepting.

### 3.2 Invariants 1 and 3 — authority and health are different axes

```text
AUTHORITY   may Sentry look at this camera at all?    the operator decides
HEALTH      can Sentry reach it right now?            the network decides
```

There is no function taking a health state and returning an authority, because no such
function should exist. A timeout is not a revocation, an expired camera password is not a
revocation, and a model's opinion is not a revocation.

`AuthFailed` gets its own sentence in the status line — *"the camera rejected our
credentials; operator authority is unaffected"* — because that is the state most often
misread as Archie deciding to stop watching.

`DiscoveredOnly` is the important negative: finding a camera-like service in an
Internet-wide scan tells you a device is exposed, which is a security finding about its
owner, not consent from them. It never permits opening the stream.

### 3.3 Invariant 2 — three authorities, none substituting for another

```text
operator policy   ∩   subject consent   ∩   sensor authority   =   permitted
```

Owning the camera is not consent from the people in front of it. A subject consenting
does not authorize a camera the operator never approved. An operator policy disabling
biometrics is not overridden by a subject who would have allowed them.

The operator's ceiling is checked first, deliberately: on a node with biometrics
disabled, reporting "no lease" would send someone to collect a consent form that would
not have helped.

Where consent is absent, work degrades to anonymous rather than stopping. A person with
no lease is still counted for occupancy and still avoided by a robot; they are simply
never resolved to a name.

Association resolution refuses to let similarity beat physics: an authorized location fix
placing the subject elsewhere, or two sightings too far apart to walk between, rejects the
association outright. Appearance matching is confidently wrong about twins, uniforms and
dark coats.

## 4. Time is two clocks, not one

| | |
|---|---|
| **event time** | when the world did the thing — `observed_at`, `provider_timestamp`, `published_at` |
| **knowledge time** | when Archie found out — `fetched_at`, `normalized_at` |

An earthquake at 03:00, published 03:14, fetched 03:31 is one event with three answers
depending on which clock is asked.

Replay filters on **knowledge time**. A harness filtering on event time hands detectors
observations that had not yet been fetched when the decision was made, and every detector
looks prescient. `known_by(t)` is the predicate; it exists so the mistake is hard to make.

Staleness is measured in event time, and a record with no event time reports *no*
staleness rather than zero — a provider catalogue entry is not stale a day later, it is
simply not about a moment, and zero would claim it is current.

### 4.1 What the plausibility window cannot do

The global window is 1870–2100. It catches a zeroed struct and a far-future garbage
value. It provably **cannot** catch the most common ingestion bug there is — seconds
handed to a field expecting milliseconds — because a 2026 timestamp divided by a thousand
lands in January 1970, a perfectly ordinary date.

The window cannot be narrowed to exclude it either: UCDP conflict records legitimately
reach back to 1946.

What separates the bug from the history is the **provider**. An earthquake feed promising
data minutes old has not legitimately returned a 56-year-old record; a historical archive
has. So that check takes the provider's freshness expectation
(`implausibly_stale_for`) and cannot be a global rule. This is recorded because the
tempting fix — tightening the window — silently discards real history.

## 5. Epistemic states

`Raw → Parsed → Normalized → {Corroborated | Inferred} → Verified`, with `Disputed`,
`Stale`, `Superseded` and `Retracted` as things the world does to a statement rather than
rungs on the ladder, and `ArchieHypothesis` off to one side.

The graph is deliberately narrow: a jump from `Raw` straight to `Verified` should be
impossible to express, not merely discouraged.

Two consequences worth stating:

- **Staleness demotes, and recovery does not restore standing.** A stale statement
  re-enters at `Normalized` and climbs again. Resuming `Verified` would carry an old
  verification onto new bytes.
- **Retracted and superseded are terminal for automation.** An operator can still act — a
  source retracting in error is a real situation — but no detector, fusion pass or model
  may quietly resurrect a withdrawn fact.

There is no confidence float anywhere in this model. 0.87 does not say whether four
independent sensors agreed or one wire service was copied four times, and those demand
different actions.

## 6. What is built, and what is not

**Built (S0):** `sentry/` — the temporal model, epistemic states and transitions, the
provenance graph and its independence counting, camera authority, consent and identity
association. 123 checks across two suites, compiled `-Werror` with
`-Wswitch-enum`, gated in CI by a dedicated job in the Arch container, since the compiler
that must accept this code is the one the ISO is built with.

**Owner-biometric contract (S0.1).** The C++ identity layer now expresses a first,
strictly local owner enrollment and presence-verification gate. It holds only an opaque
SHA-256 template reference; it accepts an isolated worker's finite live-match result
only when the subject is the owner, the policy and current self-consent permit
biometrics, the named sensor matches an operator-authorized camera record already in
force at capture time, the current template is bound, and the capture is fresh. Each use
also binds to an opaque challenge issued after the current enrollment and expiring within
thirty seconds, so a stored result cannot become a later presence assertion. Revocation,
missing liveness, stale capture, a mismatch, or a
non-owner subject fail with a precise refusal. It cannot enroll another person or become
general face recognition. A consent lease is also bound to exactly one subject: another
person's valid grant is an integrity mismatch, never a usable substitute or override.

**Not built.** No provider adapters, no HTTP, no Cesium Native, no camera I/O, no face
or gait model, no secure-template store, no inference, and no message ingestion.
The person graph and its store are built (see 6.1); everything that would fill them
automatically is not. Those are S1
onward, and none of them can quietly violate the rules above — which was the point of
doing this first.

## 6.1 The person graph

The most consent-sensitive part of Sentry, and the part most likely to be built twice if
the types do not exist first. They now do, in `sentry/identity/` and `sentry/store/`.

### Four capacities, and never confusing them

| Kind | Resolvable from a sensor | Why |
|---|---|---|
| `Owner` | yes | consents for themselves, and is still bound by their own policy |
| `ConsentedPerson` | yes | a lease is on file |
| `PublicFigure` | **no** | being in the public record is not consent to be recognised in a street |
| `AnonymousTrack` | **no** | a body a sensor saw; counted, avoided, never named |

`resolvable()` is a property of the kind and is evaluated before any evidence is weighed.
The alternative — weighing appearance first and consulting the kind afterwards — puts a
face model's confidence in the position of deciding whether the rule applies. A byline
database plus a face model is face recognition assembled from two things that each looked
harmless, and this is where that conjunction is broken.

### Properties are open; keys are classified, not trusted

A profile holds height, eye colour, hobbies, skills, the year someone was met — anything.
`AttributeClass` is a closed enum, which is right for the handful of things policy branches
on and useless for the rest.

An open key/value bag beside a classified enum is a bypass with extra steps: write
`trait{"home_address", ...}` and walk around `AttributeClass::HomeAddress` while nothing
notices. So `classify()` normalises the key — lowercased, separators dropped, so
`home_address`, `homeAddress` and `subject_home_address_line_1` are one key — and decides
what it is. The stricter of the declared and derived classifications wins: a caller may say
a trait is more sensitive than it looks, never less.

The classifier reads keys and cannot read values. `lives_in` holds a city as often as a
street address and no key matching tells them apart, so it is a floor rather than a
guarantee, and `declared_sensitivity` is how a caller who knows better says so.

### Where a property came from is part of the property

"Rafa is 14" read out of a chat is Archie's reading of a sentence — people mistype, joke,
and talk about third parties. `InferredFromMessages` and `ModelInferred` are hypotheses;
`asserted()` skips them and `any()` does not, so "Archie does not know" and "Archie
suspects" never return the same thing. This is accuracy rather than restriction: the owner
is the one who benefits from being able to tell.

### The graph is rooted at the owner

Every person has a path back to the root, and that path is the answer to "why does Sentry
hold a record about this person". There is no depth limit; a five-hundred-hop chain
resolves. `mutual()` answers "amigos en común" over asserted edges only. Records with no
path are reported by `orphans()` — reported, not deleted, because one is evidence of the
bug that produced it.

`EdgeBasis::CoPresenceInference` exists as a named value and is refused for every
relationship kind. Two people in the same place at the same time is not a relationship, and
a year of that is a social graph built out of surveillance, assembled from people who
consented to nothing. It is spelled out rather than omitted because a value that is present
and refused is a decision, while a value that is absent is a gap someone fills later.

Personal ties — family, household, friendship — are never read out of the public record and
must be declared by one of the people they are about.

### Revocation

Checking consent at collection is the easy half. When a lease is withdrawn the decision is
per record, and turns on whether the record came from that consent or from an independent
authority.

| Record | Outcome |
|---|---|
| biometric template, location history | deleted; a template with the name stripped still re-identifies |
| track attribution | **anonymised**; the camera did see a body at 14:02 |
| edge from the public record | kept — revocation does not un-publish |
| edge someone declared | kept; withdrawing a declaration is a separate act |
| edge with no recorded provenance | deleted; it cannot be shown to rest on anything independent |
| anonymous aggregate | kept; it names nobody |
| access-grant audit | kept; a fact about the operator's own conduct |

Deletion leaves a tombstone holding an opaque id, a time and a reason, and no personal
data. An operator who cannot show what was deleted cannot demonstrate that a revocation was
honoured.

### The store, and why reading is a trust boundary

`sentry/store/` is the only part of Sentry that touches a disk, in its own library so the
contracts stay provably I/O-free.

Validating on write and trusting the read is the natural design and it is wrong. A file is
hand-edited, restored from a backup taken before a rule existed, written by an older build,
or synchronised from another machine. `load()` therefore re-runs every refusal: a family
edge sourced from the public record does not enter the graph because someone put it in the
file. `RelationEdge` carries `declarer_id` for exactly this reason — a rule that only holds
at the moment of writing is not a rule.

`save()` refuses the same things, because persisting a record that cannot be loaded back
gives a store that silently loses data on every round trip.

One bad line costs its line; an unknown format version is fatal, because a format change
can alter the meaning of a field that still parses. Rejection messages never quote the value
they rejected.

### The owner overrules all of it

The strict defaults describe what the automated pipeline may do unattended. `can_override`
is the second question, asked when a person is present with a reason, and the answer is
recorded. A refusal about a public figure is a refusal about a *capacity*, and the owner
sets the capacity: declaring someone a person in their life rather than a subject of the
news is the override, visible in the file.

## 6.2 `archie-sentry`

The first program that links `sentry::world`. Until something did, the rules were a
compiler's opinion rather than something applying to a file on a machine.

```
archie-sentry init nelson Nelson
archie-sentry add-person rafa consented_person Rafa
archie-sentry add-edge nelson rafa friend owner_declared nelson "conocida en 2024"
archie-sentry set rafa age 14 subject_declared
archie-sentry show            # sensitive values redacted unless --all
archie-sentry mutual rafa ana
archie-sentry path rafa       # why this person is in the graph
archie-sentry check           # orphans and unconfirmed traits
```

A tool, not a daemon: nothing about a relationship graph should happen without someone
asking for it, so there is no unit to enable. The graph lives at `$ARCHIE_SENTRY_GRAPH`,
else `$STATE_DIRECTORY/graph.tsv`, else `/var/lib/archie/sentry/graph.tsv`.

## 6.3 Messaging and social ingestion — the honest position

The graph is designed to be filled from the owner's own correspondence. Two things about
how that can actually happen, recorded here so nobody rediscovers them the expensive way.

**There is no read API for personal WhatsApp.** The Business API is for businesses
messaging customers. Reading personal chats means driving WhatsApp Web through an
unofficial library, which violates the terms of service and gets accounts banned, or
extracting the local encrypted database from a rooted device. Neither is a foundation to
build on.

**Instagram's Graph API does not enumerate a person's friends.** It exposes a
Business/Creator account's own media and aggregate counts. Scraping is rate-limited and
ends the same way.

**The supported route is export.** WhatsApp has "Export chat"; Meta has "Download Your
Information", which includes messages, followers and following. Both are first-party,
neither risks the account, and both produce files an ingester can read.
`TraitBasis::InferredFromMessages` exists for what comes out of them: a reading of a
sentence, carried as a hypothesis until a person confirms it.

The ingester is not built. When it is, it writes hypotheses and nothing else.


## 6.4 Ambient capture

The owner asked for Archie to see what they do — the email they write, the search they make,
the message that arrives — live, across every device they own. Legitimate: their activity,
their hardware. Two rules keep it from being anything else, and both live in `capture/`
rather than in the device agent.

**A guest suppresses capture; it is not profiled.** "Sepa cuando alguien más lo usa, como mi
mamá" has a safe reading and a dangerous one. `ActorAttribution::ConfirmedOther` — a
biometric non-match, a different unlock, an explicit "not me" — is never recorded and never
attributed to a person. At most the device is counted as in use. It is the `AnonymousTrack`
rule applied to a keyboard, and no policy switch turns guest capture back on. A matcher that
was merely unavailable says nothing: a covered camera is not a guest.

**Received content belongs to whoever sent it.** A message arriving on the owner's phone
carries someone else's words; folding "everything X said to me" into a searchable store is
§40 through the inbox. Counterparty-authored content is metadata only — that the owner is in
contact with X, never the text.

**Linkearse solo a mí.** Capture flows only from devices the owner enrolled, checked before
content and before the actor. A withdrawn or never-enrolled device feeds nothing.

The ingest folds a stream into counts and contacts and drops the events. What the graph needs
is that the owner searched forty times today and has been in contact with Rafa — not a
transcript. Every suppression is counted by reason so the owner can audit the gate.

The honest capture path is the OS's own live streams — a notification listener, an
accessibility service, an IMAP IDLE session — not a headless web login, which violates a
platform's terms and gets an account banned. The source enum is grouped by which mechanism
delivers each.

## 7.1 Vision and camera control

`camera/authority.hpp` says who may look at a camera. `camera/connection.hpp` decides what to
do about a stream over time — connect, retry, back off, hold, report lost — and never revokes:
an auth-failed handshake is the camera's own password, retried, not the operator withdrawing;
an operator-authorized camera is never silently disabled; backoff is bounded so it never
becomes give-up. `vision/track.hpp` gates the one operation that turns a body into a name.

A track is anonymous by construction — counted, avoided, ephemeral id, no name. Resolution is
the composition of three checks in order: resolvable kind (a public figure never resolves from
a camera), the operator-subject-sensor consent gate, and only then the evidence. A hard
contradiction — two cameras too far apart to walk between in the time available, computed by
`world/geo.hpp` from a haversine distance and a plausible speed — defeats any appearance
match, which is exactly what appearance matching is most confidently wrong about.

The RTSP handshake, the decoder and the detector are built, behind two interfaces so the
orchestration around them stays testable. `runtime/frame_source.hpp` and
`runtime/detector.hpp` are the seams; `CameraWorker` drives them and is tested against a
scripted source and detector, without a camera in the loop. The real halves —
`gstreamer_source.cpp` and `onnx_detector.cpp`, and the `sentry-camd` daemon that wires them
— compile only under `-DSENTRY_WITH_GSTREAMER=ON -DSENTRY_WITH_ONNX=ON`, so the ISO and CI,
which have no camera, never build them. They are validated on a machine that has a camera and
a GPU, which is the only place they can be.

Two properties are structural in those implementations. The RTSP credential is set as a
pipeline property and never logged — not even a GStreamer error, which may carry the URL
(§90); `sentry-camd` reads it from `ARCHIE_CAM_<id>_URL`, which systemd fills from a
credential store. And the detector returns where, never who: it has no access to a face
database and could not name anyone if it tried.

## 7.2 The globe

The observatory plots real observations — USGS earthquakes at their real latitude and
longitude, sized by real magnitude, coloured by real severity — on a textured, rotating 3D
earth in Qt Quick3D. `world/geo.hpp` is the placement maths. Full Cesium 3D Tiles
(photorealistic terrain streaming) is a separate integration needing the Cesium Native library
and an ion token, and is not built.

## 6.5 Capture agents

The per-device half of ambient capture lives in `capture-agents/`, bound to the tested C++
ingest by `protocols/capture-event.schema.json`. They are Kotlin and JavaScript, built with
their own toolchains and side-loaded onto the owner's own devices — personal tools, not store
apps, and not built or installed by the image.

| Device | Mechanism | Captures |
|---|---|---|
| Android | `NotificationListenerService` | WhatsApp, Instagram, Messenger, Telegram, SMS and Gmail previews, live |
| Android | `BiometricPrompt` | the owner-present signal that gates everything |
| Browser | Manifest V3 `webNavigation` | searches and page navigation |

The agents report honestly so the gate rarely has to suppress. Android confirms the owner
biometrically and decays the attribution with time since that signal; a "someone else is using
my phone" control stamps `confirmed_other`. The browser claims only `presumed_owner`, because
a laptop has no per-action owner check and over-claiming would record a guest borrowing it.

No agent sends a message body, a page, a form field or a password. The schema has no field for
content, deliberately. The contract was validated end to end: the exact NDJSON these agents
emit was fed through `ingest_stream`, and the gate behaved correctly — the owner's activity
recorded, a guest's message suppressed, a received message kept as a contact but not as
content, and an un-enrolled device feeding nothing.

The node endpoint that receives the NDJSON is `archied`'s job and is not built.

## 7. Process boundaries, when they arrive

`sentryd` may host modules in-process initially, but these stay explicit extraction
points, because each is a privilege boundary rather than a packaging preference:

| Process | Why it is separate |
|---|---|
| `sentry-netd` | the only component with arbitrary egress |
| `sentry-camd` | holds camera credentials; restricted to registered endpoints |
| `sentry-visiond` | GPU inference, no secrets, no egress |
| `sentry-display` | read-only, no credentials, no authority socket |

Provider payloads and camera firmware are both untrusted input.

## 8. Excluded, deliberately

- **Analytics in kernel space.** Drivers, cgroups, namespaces, DRM/KMS and eBPF hooks
  stay in the kernel. Provider clients, CV and world-model logic do not. A visualization
  crash must not take down the node.
- **Autonomous target selection, and weapon-quality live coordinate fusion.** Sentry
  supports public-source situational awareness, historical research and aggregate pattern
  analysis, preserving source fidelity and uncertainty. It does not enrich public
  observations into targeting coordinates.
- **Satellite person-tracking.** Open Sentinel imagery cannot identify an individual.
  Person-level work is consented camera/GPS/device fusion; EO is for environment,
  infrastructure and regional change.
- **A confidence number standing in for provenance.**
