# Nebius x NVIDIA Global AI Hackathon

## Provenance

The migration baseline is `PentaVM/ArchieOS` commit `045e782` (11 September 2026),
recorded locally in the source repository as annotated tag
`pre-nvidia-migration-2026-09-11`. The source repository and its remote branches were
not rewritten or deleted.

Pre-existing ArchieOS work includes the Rust control plane, PostgreSQL/TimescaleDB and
pgvector state, capability-aware routing, policy and execution boundaries, native Qt
observatory, GeoFabric foundations, OSM building ingestion, C++ Analytical Cortex,
Sentry, capture agents, installer, and ArchISO packaging.

Four components that a reader might reasonably assume were built for this hackathon were
not, and are named here so that nothing in this repository overstates what is new:

- **The learning admission firewall** (`archie-memory::admission`, RFC-011) and
  **bounded analytical synthesis** (`archie-memory::analysis`, RFC-012), which decide
  what may enter memory at all and keep perception from accumulating noise.
- **The conversational floor manager** (`archie-memory::floor`), which decides whether
  Archie should speak from speaker-hold, thought-completion and interruption-utility
  estimates, and defaults to silence.
- **Wanda** (`sentry/identity/wanda`), deterministic social inference that produces
  hypotheses a subject must declare before they become relationships.
- **Privacy as a routing disqualifier** rather than a weighted score, in `archie-router`.

## Work in this repository

- Extracted the active runtime into an independent history and excluded the retired
  Python/PySide6 application, stale release records, caches, and generated output.
- Added explicit NVIDIA NIM and Nebius Token Factory provider kinds without coupling
  cognitive roles to either provider.
- Added an OpenAI-compatible inference transport with endpoint validation, bounded
  responses, environment-only credentials, timeout handling, and a second privacy gate
  immediately before network egress.
- Added a small CLI smoke path for model discovery, readiness, and chat requests.
- Added an optional NVIDIA package profile and staged provider manifests in the ISO.
- Replaced legacy CI with checks that represent the migrated Rust and C++ stack.

### The privacy spine

- Added `PrivacyGrade::Secret`, a grade no locality permits. The four existing grades
  answer *where* data may be processed; this one answers *whether*, and the answer is no
  — a password contributes nothing to reasoning and its disclosure cannot be withdrawn.
- Added `DataRetention` with an `Unknown` default treated exactly as `Retains`. A
  provider advertising zero data retention and this deployment having verified it are
  different facts, and only the second is a safety property. Non-public data to an
  unverified third party is now disqualified at routing.
- Added `archie-vault`: AES-256-GCM sealed entries (via `ring`, already in the graph
  through rustls) whose grade is authenticated rather than labelled, so rewriting a
  stored record's grade yields a decryption failure instead of a readable entry.
  Credentials leave only through a broker whose handle cannot be cloned, serialised or
  printed.
- Added the context gateway: task-bound, TTL-limited capsules that pseudonymise entities
  when the destination is a third party, resolve the aliases locally afterwards, and are
  deliberately not serialisable, so private context cannot become a trace by accident.
- Added `archie-mcp`: Model Context Protocol entered through the capability model.
  `AuthorizedCall` has no public constructor, so the path from a model's proposed tool
  call to a transport necessarily passes the gate. The operator's allowlist is the
  authority; `tools/list` is the server describing itself.
- Added the streamable HTTP transport for it, covering `initialize` negotiation,
  session binding, `tools/list`, `tools/call`, `resources/list` and `resources/read`,
  both the JSON and event-stream response shapes, per-call credential brokering, and
  response budgets enforced before allocation. The `stdio` transport is not implemented,
  and no stub resembling one ships.

Vault references (`vault:...`) existed throughout the pre-existing codebase; what was
missing was anything that resolved, encrypted or enforced them. That half is the new
work here.

No benchmark numbers are claimed here. Test results and any demo measurements must come
from executed builds on identified hardware.

### Web search

- Added `archie-search`: web search through Tavily, entered through the capability model.
  A query is a disclosure, so it passes a gate the way a tool call does.
  `AuthorizedSearch` has no public constructor, a caller must hold `network.internet`,
  and the query's grade is checked against the provider's validated clearance. A cloud
  provider whose retention nobody has verified is cleared for public queries only;
  a manifest that claims more fails validation instead of taking effect.
- The client uses HTTPS only, follows no redirects, and enforces its response budget
  before allocating. It sends the key as a bearer header brokered from the vault per
  call. Tavily's generated answer, raw page content, images, country and automatic
  parameters are never requested, and each is sent as `false`.
- Results are bounded, stripped of control and bidirectional-override characters, kept
  only for ordinary `http`/`https` pages, and given a `web:` reference that a later stage
  can cite without quoting the page. Refusals, errors, receipts and debug output never
  contain the query, and provider error text has echoed keys and queries blanked out.
- `Cerebra::propose_search` records the search at `SelectCapability` with its grade and
  never its words. `archie-search` is a CLI smoke path, like `archie-infer`, that reads
  `TAVILY_API_KEY`. `providers/search/tavily.toml` ships public-only and unverified.
- Verified against loopback listeners, not the live API. Eleven rules were broken on
  purpose, and a test failed for each. One of them first survived: the only
  credentials-in-URL case carried both a username and a password, so either check hid
  the other. The test was fixed rather than recorded as passing. `extract`, `crawl` and
  credit accounting are not implemented.

### The Archie Stage

- Added the Archie Stage (`display/stage`, [docs/STAGE.md](docs/STAGE.md)), a Wayland
  compositor in which Archie operates applications with a visible pointer and keystrokes,
  with nobody's mouse needed and nobody's mouse locked out. Any physical input takes the
  stage back at once. The agent resumes after eight quiet seconds, and a pause chord holds
  it until a person resumes it. The agent's own input goes through the seat and cannot
  trigger either.
- An action takes one path, whoever asks: `Cerebra::propose_stage_action` or
  `archie_stage::Operator`, then the stage policy, a one-operation lease, a gate whose
  output is the only thing that can be sealed, an HMAC-sealed line, and the compositor's
  own re-check against the window's geometry revision.
- Applications get text the way they get a person's: keys found on the keymap they were
  given, with Shift, AltGr and modifier state moved as a physical key moves them. A text
  with characters off the keymap goes whole through the application's input method. With
  neither route, the receipt says `not_typeable` instead of `executed`. Qt's public seat
  API sends Ctrl+C as a plain "c" and drops text for applications without text-input, so
  this uses the private calls Qt's own keyboard handling uses.
- Cinematics for any application come from a pure, display-free model (`director.hpp`).
  The camera frames the action and does not chase the cursor, and cuts to the whole stage
  on a takeover. Pointer paths are Fitts-timed, and ripples are bounded. Captions and a
  strip of recent actions carry verbs, applications and counts, never typed text. Frames
  are requested only while something moves: 609 in a scripted 16 s session, against 960
  at 60 fps.
- Verified in WSLg against real clients, reading their Wayland protocol traces:
  - A demo form saved "Archie escribió esto solo" with no human input.
  - A Ctrl+A chord replaced a draft, and "Hola, ñandú: 100% listo!" arrived in order.
  - `weston-terminal` ran a typed shell command with quotes and redirection.
  - A mouse moved over the window during one run took the stage, and the agent resumed.
- Not working: `foot` (it needs protocol versions Qt 6.11's compositor does not offer),
  Alacritty at full size, targeting popups, XWayland, and any run on DRM/KMS hardware.

### Archie for IDE agents

- Added `archie-mcp-server`, which serves Archie's tools over MCP on stdio to VS Code's agent
  mode, Claude Code, Zed and any other MCP client. It is one Rust binary, with no IDE
  extension. The research behind the choice, including why Live Share is not viable and
  why Zed is the recommended editor for the image, is in
  [docs/research/IDE.md](docs/research/IDE.md).
- A tool result leaves the node for a model Archie cannot see, so each tool declares a
  privacy grade. The server's clearance defaults to public and cannot go above internal.
  Node status is an allowlist that leaves out the hostname, network interfaces and
  location.
- Operating stage applications is opt-in, goes through the same `Operator` and gate, runs
  as the IDE agent's own principal, and can never obtain an approval or clear a person's
  takeover.

### Tools Archie installs for itself

- Added `archie-toolpack` ([docs/TOOLPACK.md](docs/TOOLPACK.md)), with which Archie fetches
  a tool a task needs from a published release and installs it without a person running
  an installer.
- The catalog is read only after an Ed25519 signature by a trusted key verifies over its
  exact bytes. It is then validated like outside input: HTTPS URLs, declared SHA-256 and
  size, binary paths inside the tool, and no system administration tools.
- An install needs a one-operation lease on the tool's directory. Tools that can reach the
  network or devices also need a person's approval. Downloads enforce their size while
  streaming, check the digest at the end, and follow redirects only to hosts the catalog
  names.
- Unpacking refuses `..`, absolute paths, links, overwrites and setuid bits. A version
  appears in one rename and becomes current in another, and rollback swaps back.
- Not done: `.tar.xz`, `.tar.zst` and `.zip` artifacts, a published catalog or release
  repositories, registering installed capabilities, and revocation.

### Archie looks before it acts

- Added `archie-sight` and `archie-look` ([docs/STAGE.md](docs/STAGE.md#finding-what-a-task-names)).
  Given a description such as "el botón Guardar", Archie captures the window, asks a
  grounding model through IRIS's LocateAnything adapter where that is, and clicks it
  through the stage gate. It then waits for the control to redraw itself, and a click that
  changed nothing is reported as a failure. The crate joins IRIS and the stage and adds no
  authority.
- Looking is gated like touching, because a capture is a disclosure:
  - It needs the application's `snapshot` grant and a read lease. A read lease cannot
    click, and a click lease cannot look.
  - The compositor re-checks the command, grabs only that window, writes it owner-only,
    and tells only the connection that asked where the file is.
  - The caller accepts the file only if its digest and PNG signature match, then deletes
    it.
  - The shipped policy grants no capture.
- The description reaches the model in a one-entry Vault capsule for that task, next to a
  digest-bound image. A window declared internal is refused before egress to a cloud model
  whose retention nobody has verified.
- The model is not in the loop. A session keeps the stage connection and remembers where
  it found each control: a box nothing has been drawn over is used as it is, a box
  something was drawn over is confirmed from one capture whose shape must still match, and
  only a box that fails that is grounded again. One capture re-confirms every control the
  session knows in that window, and a click that is not followed by a repaint forgets the
  position it used.
- `--tempo agent` plays the same checked, sealed commands on a quarter of the clock, and a
  click is verified by the compositor's damage events instead of a fixed wait and a second
  capture. Twelve activations of the demo form then ran at 1.56 a second, all verified,
  with grounding served by a local stand-in so the number is Archie's own; the capture that
  confirms a control is now the slow part of it, at 135–590 ms.
- Added `archie-a11y`: what an application says about its own controls, read over the
  accessibility bus in Rust. Every toolkit publishes its buttons, fields and menu items so
  that screen readers work, and that is a structured source in IRIS's sense — no pixels, no
  model, no guess. The compositor now reports which process owns each surface, so the
  window on the stage and the application on that bus are known to be the same one, and a
  second window of the same application is told apart by its size.
- A declared answer is used only when it is unambiguous: a control whose name the task's
  words contain, of the kind the task asked for, and exactly one of them. A window that
  names nothing, or two controls with the same name, falls through to looking and to the
  model. It is read again for every action, so it cannot go stale.
- With that path, the same twelve activations of the demo form ran with no capture and no
  model at all: 4.84 a second, every one clicked and seen to redraw. Asking the application
  took about 11 ms and the click 166–208 ms.
- A grounding model is no longer in the way when it is needed: a session can be given the
  whole list of targets, take one capture, and ask about all of them in the background
  while it works on what it already knows. In one run against a model told to take 800 ms,
  the target that needed it waited 88 ms, and eight activations ran at 3.98 a second.
- Live in WSLg through Nebius Token Factory, with nobody touching the mouse:
  - `moonshotai/Kimi-K3` completed one flow and placed the demo form's field and button
    within a pixel of their real centres, but returned a usable answer in only three of
    five calls: two exhausted 512 output tokens while reasoning, and on a later day it
    answered nothing at all, timing out on every request while Gemma answered in about a
    second on the same account.
  - Archie clicked the field, typed, and pressed Guardar, and the form's own log recorded
    the note.
  - On the same capture, `google/gemma-3-27b-it` kept the output format but missed by
    100 px or more; its click changed nothing and was reported as failed.
  - `openbmb/MiniCPM-V-4_5` came within 3 px but ignored the format, so IRIS refused it.
- Added `archie-reflex`: the local model IRIS's Visual Reflex contract describes, trained
  in this repository in Rust — three convolutions and a head, about five thousand numbers,
  5 KB as int8, one window looked at in about 9 ms on one core. It says where a window's
  controls are and nothing else: no words, no meaning, no decision.
- Nobody labelled its corpus and no screen of anybody's was in it. Applications that
  publish their controls so screen readers work label their own captures: `archie-teach`
  takes a capture through the stage's snapshot gate, asks that application over the
  accessibility bus where its controls are, and keeps a 160×96 grey tile — too small to
  read — with the boxes. 448 windows and 3424 controls, from a demo window that rearranges
  itself every second and a half under three Qt styles.
- What it buys, measured live in WSLg: a stand-in grounding model was made to answer 15 px
  off the demo form's button — near it, not on it. Without the local model, four clicks of
  four landed on nothing, each was reported as having changed nothing, and the form never
  saved. With it, the answer was put on the control the model saw underneath, all four
  clicks landed, the form saved four times, and the run went at 3.81 activations a second.
- On the 89 windows it was never taught it finds 740 of every 1000 declared controls, 672
  near enough to click, and draws 344 boxes per 1000 around things nothing declared — an
  upper bound on its mistakes, since applications declare buttons and fields but not their
  labels and panels. `archie-reflex check` sweeps the confidence so the trade is visible.
- The first run of that trainer collapsed: it learned for one epoch, pushed three quarters
  of its own rectifiers permanently under zero with too large a step, then said the same
  thing about every cell at a loss that looked respectable. Rectifiers that leak and a step
  ten times smaller fixed it, and every epoch now prints how much the model still varies
  and how much of it still fires, so it cannot happen quietly again.
- A capture no longer means a window. A snapshot command may name one box of its surface;
  the box is sealed with the command, re-checked against the window as it is now, and the
  compositor encodes only those pixels. A look that only confirms what the session knows
  asks for those boxes — 355×88 instead of 1534×696 on the demo form — so it discloses less
  and costs less: encoding fell from 43 ms to 1–4 ms and a whole look from 135–590 ms to
  35–119 ms. The same twelve activations that ran at 1.56 a second when every look captured
  the window now run at 4.07, and eight of one control at 6.05, every one verified by the
  window redrawing. What is left of a capture is Qt rendering the item, 22–98 ms whatever
  size is asked for; at agent tempo the pointer's own 166–176 ms is now the slow part.
- A menu is now a surface Archie can name. It is not part of the window it belongs to: the
  application opens it over one and closes it as soon as it is done, and the stage gives it
  an item under that window and a place on the surface table, with the window's application
  id and `parent` pointing at it. A command names it, the compositor re-checks it, a capture
  is of the menu alone and its damage verifies the click.
- What goes where follows from what it is: a menu item goes to the newest open menu, and
  waits up to 600 ms for one because the click that opens a menu is answered before the
  menu is drawn; everything else goes to the window, menu or no menu. An answer about one
  surface is never used in another, and a menu item is never asked about before its menu
  exists — coordinates from the wrong window are a click in the wrong place, and while
  writing this one such click chose a menu entry nobody had asked for.
- Measured in WSLg on a demo window whose menu is a real popup surface: six activations,
  alternating the window's own button and an item in the menu it opens, at 4.44 a second,
  all verified by a repaint, with the application's log recording its three choices. The
  button was declared by the application in 3–23 ms with no capture and no model; the menu
  item needed one, and it was a capture of the 200×80 menu rather than of the 1334×596
  window behind it.
- Added a `stroke`: a command that carries up to 256 points, pressed at the first, followed
  through every one of them and let go at the last. A click was a press and a release in one
  place and a drag was two points; a signature, a selection across a canvas or a line in a
  drawing is a way through the window, and the stage can now say one. Every point is
  re-checked against the window as it is, the line is drawn straight between its points at a
  speed rather than bowed the way a reach for a target is, and the caption says "dibujando".
- Added `archie-paint`: it turns a picture into strokes and draws them, one ordinary command
  each. The hatching is an engraver's — parallel lines where the picture is dark, crossed at
  another angle where it is darker — with the tone levels taken from the picture's own
  histogram rather than from fixed numbers, and the grain softened first so that a pen
  follows tone instead of speckle. `--dry-run --preview` plans and draws it on paper without
  a stage, which is how it was tuned.
- Measured in WSLg on a sheet of paper with no controls on it: the Mona Lisa, 977 strokes on
  a portrait window, drawn at 4.8 strokes a second at agent tempo with none refused. Nothing
  about a picture is special to the stage — the policy has to allow `stroke` for that
  application, every stroke takes a lease for one operation, and each one is sealed and
  re-checked.
- Two things that run found. The pen has a width, and hatching closer than about three times
  it comes out solid black however careful the plan was, so the plan is made for the pen the
  canvas draws with. And a real mouse crossed the stage window mid-picture: the compositor
  handed control to the person, as it should, and the painter spent 551 strokes arguing with
  it. A stroke refused with `operator_has_control` now waits for the person and is tried
  again.
- The same strokes were then taken to a real application: Firefox on the stage with
  photopea.com, an image editor that is a web page. Photopea takes a script in its URL, so
  the document is made without a menu, and Archie finds where to draw by looking: one capture
  through the same gate and read lease as any other, and the largest light rectangle in the
  window is the paper.
- **Archie now gets the picture back out.** It draws with Photopea's own brush, opens
  Photopea's File menu, walks it to Export as ▸ PNG and clicks Save; the file lands in the
  browser's download directory, 760×1020 and 62 KB. This is the step that was open.
- A correction to an earlier entry. It said "647 strokes of the Mona Lisa went into Photopea
  at 4.5–4.7 a second with none refused". The strokes and the rate were true of the stage,
  the drawing was not: nothing was painted, because the brush had never been selected. A
  count of strokes the compositor executed is not a count of marks an application made.
- Photopea declares nothing. Its whole interface — menus, tools, panels, dialogs — is drawn
  inside one canvas, so the accessibility bus has nothing to say about any of it. In an
  application like this the fastest and most exact path, where the application names its own
  controls, is simply not available.
- **Fixed: the keyboard now reaches web content.** Keys used to reach Firefox and stop there.
  The Wayland trace showed Firefox asking the seat for a keyboard twice; Qt's compositor sends
  pointer events to every one of a client's bindings and keyboard events to exactly one, and
  Firefox listens on the other. The key arrived in the protocol log and the page's own
  `keydown` never ran. The stage now says the same thing — focus, key, modifiers — to the
  bindings Qt left out, found the way Qt finds the one it speaks to so nothing is sent twice,
  and a client with a single binding is untouched. A page that echoes what it receives reads
  `b/KeyB  z/KeyZ  Control/ControlRight+ctrl  s/KeyS+ctrl`. In Photopea, which reads keys only
  once its own page holds focus, `[` now thins the brush: the drawing it exports is 110 lines
  at 119 KB instead of 70 thick ones at 62.
- Agent tempo is too quick for a canvas that watches the pointer itself: at 32 ms of press
  Photopea registers nothing, every stroke is executed and none refused, and the page stays
  white. At a person's tempo the same strokes paint.
- A single click with the Move tool, sent only to give the page keyboard focus, left Photopea
  unable to paint afterwards. Removing it is the difference between a 465-byte white PNG and
  the picture.
- `archie-look` opens the File menu and then reports that verification saw no new scene
  revision, because the menu is drawn inside the page and not in a surface of its own. The
  activation worked; the check could not see it, and retrying on that closes the menu again.
- Of the vision models the provider offers, on this window: MiniCPM-V-4_5 placed the File
  menu well enough to open it (8.0 s), Gemma answered in the right format and pointed at the
  browser's tab bar (2.4 s), and Kimi-K3 missed four times out of four. None placed the
  smaller targets, so those are clicked from the window's own fixed layout. IRIS now accepts
  four bare integers as an answer as well as `<box>…</box>`, which is what MiniCPM returns —
  the same strict grammar, one more punctuation.
- **Pointer craft.** Drawing a picture in fifteen seconds is the most demanding thing a
  pointer can be asked to do, so it was used to find what the stage was missing. None of what
  came out is about pictures; all of it is about operating an application by hand.
  - Added `strokes`: up to 64 lines and 1024 points under one authorisation. One lease, one
    receipt, every point still re-checked against the live window, each line still refusable,
    the operator still able to take the stage between them. Measured on the same 56-line
    drawing sent both ways: 62 commands in 17.6 s against 12 commands in 13.4 s, which is 84
    milliseconds a command. `--batch 1` sends them one at a time so the difference stays
    measurable.
  - Fixed: the stroke speed was divided by the tempo's pace, so `--stroke-speed 942` at video
    tempo drew at eleven thousand pixels a second and every line arrived as a polygon.
    Reaching and drawing are now separate — the tempo flies between lines, the line is drawn
    at the speed it was given.
  - The budget is stated in seconds, not in lines, and the planner spends it: at each turn it
    takes the work that says the most for the time it costs to go and do it, travel included.
    Travel used to cost as much as the drawing (8.5 s against 8.4); it is now 4.0.
  - Weight without pressure: a pointer cannot press harder, so the darkest lines are gone over
    two or three times, a hair to one side and back the way they came. It costs ink and no
    commands.
  - The measure changed from commands a second to **pixels of line a second**, which is what
    decides whether a task finishes rather than how fast it was asking.
  - Measured end to end: a fifteen-second drawing is 49 lines in 4 commands, 15.1 s, none
    refused, 575 pixels of line a second, and what arrived is what was planned.
- Added `--flow`: lines that follow the form rather than shading across it. The picture's
  gradient is turned a quarter and averaged, lines are let go into that field from the dark
  places and followed until they leave the picture or reach ground with no form in it, and
  each takes the palette colour nearest the picture under its seed. The demo canvas now has
  a palette, so a colour change is what it is from outside: a click on a swatch.
- Fixed a bug that had made every flowed drawing wrong: the planner read a point of the
  canvas as a point of the picture without scaling, so a picture drawn into a canvas of a
  different size was drawn from its own top-left crop. A test now says the corner of the
  canvas is the corner of the picture.
- Wrote down a ceiling that is not the stage's to fix: an application receives the pointer's
  position at its own frame rate, not the path that was sent, so **a fifteen-second drawing
  is about nine hundred positions** whatever the commands carry. `seen_by` resamples a line
  the way the application will receive it, `--as-seen` makes the preview show that instead of
  the plan, and `--stroke-speed` lets the stage be asked for a speed that fits. Video tempo
  now draws at 1000 px/s rather than 2500.
- Measured in WSLg on the demo canvas, all with none refused: the film's intro is 40 lines
  in 3 colours in 13.2 s, the picture is 1428 lines in 5 colours in 7m09s, the same drawn
  with a one-pixel pen is 1408 lines in 6 colours in 6m32s, and the densest tried is 4438
  lines in 5 colours in 21m28s.
- The plan now says how much of the paper the pen will cover before anything is drawn, and
  the demo canvas takes its pen width as an argument and reports it, so a large drawing can
  be made with a fine pen and the plan made for the pen that will draw it.
- Four pictures in a row came out looking like a black ground with the drawing scratched into
  it. It was not too much ink: the files were transparent where the paper should have been,
  because Qt throws a canvas's picture away when the canvas is resized and the demo laid its
  white sheet only once. Every stroke was in place and every log agreed; what found it was
  reading the alpha of one pixel in the corner of the file. Verification that a window redrew
  cannot see a drawing made on no paper.
- The demo canvas itself had been dropping strokes — it held one line's points and painted
  when Qt got round to it, so the next press replaced them, and 22 of 34 strokes reached the
  application and never appeared on its paper while its log reported them as received. Worth
  recording because only the picture showed it; every count in the logs had agreed.
- Not done: more than one target per call, verification stronger than
  "the control redrew", and a run of LocateAnything-3B itself. Measured cloud vision latency
  was 1.32–1.69 s for Gemma or MiniCPM and 27.3–33.5 s for Kimi-K3, so the cloud adapter is
  a Cortex fallback rather than the interactive loop.

### Cerebra runtime additions

- Added a bounded advisory interpreter as an optional continuation after Cerebra's
  existing floor/context/router sequence. It uses real OpenAI-compatible HTTP and
  gateway capsules, with synthetic loopback tests and content-free refusal traces.
- The analytical gate and floor policy predate this work. The existing gate abstains
  throughout the ambiguous band; this work does not change it or claim a measured
  analytical improvement. Live provider and labelled-corpus evaluation remain pending.
- Added an integer context-utility learner, digest-bound to admitted outcomes, with
  exposure/risk costs, requested-reference-only gateway ranking and encrypted local
  parameter checkpoints. Tests compare explicitly synthetic contexts; no production
  training corpus, promotion or accuracy is claimed. The admission firewall is reused
  unchanged, including its eight-field payload-free curated record.
- Added a local floor baseline with bounded ephemeral audio features, online updates
  that keep only integer weights, and encrypted parameter checkpoints. This is new;
  `decide_floor` remains the pre-existing deterministic decision. No real labelled floor
  dataset, microphone driver integration, retained recordings or accuracy claim ships.
- Disabled inference HTTP redirects so an endpoint cannot forward admitted context
  beyond the destination selected by routing; verified with two loopback listeners.

### Workstation alpha additions

Built for this hackathon after the original control-plane work:

- Reproducible Arch Linux WSL rootfs and modern `.wsl` packaging, a fallback importer,
  systemd units, non-root session, disabled Windows automount/interop, and `archie`
  first-run/health commands.
- A native deterministic clarification gate in Cerebra with explicit permille inputs,
  floor-aware deferral, pre-commit/pre-external-effect states, answer-ref resumption and
  duplicate-question prevention.
- Provider discovery/config for Nebius, Cerebras, OpenRouter, Ollama and vLLM, plus a
  replaceable loopback UHP client for HarnessRouter-style Codex/Claude/Hermes workers.
- A pinned first-run build of NVIDIA NeMo-Speech.cpp and checksum-pinned Nemotron Speech
  Q8_0 weights. Audio buffers are bounded and ephemeral; partial/final text remains
  unadmitted until Cerebra decides.
- A policy-lease-gated FreeCAD process adapter and real headless parametric bracket
  smoke test using the official Arch package.

The original floor decision, learning-admission firewall, analytical gate, Vault and
capability policy remain pre-existing foundations. The workstation work composes them;
it does not relabel them as hackathon output. No speech accuracy, floor accuracy or
professional CAD fitness metric is claimed.

### IRIS perception and Foundry additions

Built for this hackathon:

- Added the `archie-iris` typed perception fabric: a revisioned scene graph, integer
  transforms, bounded ephemeral frames, damage/semantic ROI extraction, structured-first
  browser/CAD sources, explicit specialist seams, Visual Reflex/Cortex contracts, bounded
  scene memory, ACP, policy-lease motor compilation and shared-weight source sessions.
- Added a fixed-point INT8 deployment loader. `archie iris verify-model` checks file
  bounds, manifest shape, checkpoint digest and tensor bounds locally before inference.
- Created the separate `PentaVM/archie-iris` Foundry repository with license-gated
  streaming, a bounded cache, checkpoint/resume, deterministic synthetic training, sealed
  evaluation, INT8 export, champion/challenger promotion and bounded Forge experiments.
- Composed PostgreSQL 18, reviewed signed TimescaleDB/pgvector packages and `archie-db`
  into the WSL image. The control plane now waits for durable-state provisioning.
- Added a LocateAnything grounding adapter over the OpenAI-compatible vision API. It
  accepts only Vault-admitted context and a digest-bound requested ROI, returns advisory
  geometry, and was verified against a loopback server including a zero-connection
  refusal. The NVIDIA weights are not bundled and their non-commercial research license
  prevents treating them as Archie's production model.
- Exercised that boundary against the live Nebius Token Factory API with
  `google/gemma-3-27b-it`, a Public Vault capsule and a synthetic image created in memory.
  The returned proposal was mapped into Archie scene coordinates. This is one functional
  smoke test and carries no grounding-accuracy claim.

The Visual Reflex checkpoint in `models/iris/champion` and its 1000/1000 sealed result
use only the declared synthetic generator; they are pipeline validation, not a production
accuracy claim. The separate model in `models/reflex/controls` is a real one, trained on
captures of Qt windows that those applications labelled themselves, and what it scores on
windows it was never taught is in `models/reflex/README.md`. Production UI/camera
adapters, OCR/grounding providers, real labelled benchmarks and Nebius cloud training
remain unvalidated. No private frames, audio, transcripts or biometric examples are
retained.

### Wanda's local expert (LNCM)

- Added `archie-wanda-lncm` ([docs/WANDA-LNCM.md](docs/WANDA-LNCM.md)): a small next-concept
  model (Hazy Research's NCM, arXiv:2610.14162) with low-rank adapters per context domain,
  written in Rust on candle. A multiscale residual VQ-VAE turns text into coarse-to-fine
  codes; a prefix-LM generator predicts every level in one parallel pass. It answers social
  questions in two states: a decision head with abstention and calibrated confidence first,
  then a planned long answer whose names are checked against the people it may name.
- Added a persistent social state per domain: exact exposure (who was present for which
  message, who was told), expectations about each person as decayed Beta counts capped at
  0.95 unless the person declared them, moods and reply latency. Conversations of any length
  are read in chunks with constant memory; a model's interpretation enters only as delta
  lines validated against the chunk.
- Added the data and training side: owned Spanish/English synthetic worlds with exact labels,
  import of the Python `tomgen` tranches, and multi-teacher distillation (Nemotron, DeepSeek,
  Qwen, Kimi on Nebius Token Factory) through the existing egress gate at Public grade, with
  private-shape and benchmark-overlap refusal, draft → expert pairs and cross-family judging.
  Training stages (BPE codec, tokenizer, generator, adapters, calibration, evaluation) are
  resumable and record data and weight digests in a manifest that serving verifies.
- Wanda's service reads new conversation events into the Vault-held state, returns states 1
  and 2 with its answers, lets the owner see, confirm, retract or forget what it expects of
  someone, and answers any sign of crisis with a fixed message instead of a model.
- Verified on CPU by tests: a full tiny training and serving run, tamper refusal, and
  distillation against loopback stand-ins. No trained checkpoint is shipped and no accuracy,
  benchmark or GPU latency is claimed.
