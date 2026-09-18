# The Archie Stage

The Archie Stage is the compositor ArchieOS runs applications in. The agent operates those
applications the way a person would, with a visible pointer and keystrokes, while a person
watches and can take the machine back at any moment. It is what "the OS works on its own"
means in practice: nobody's mouse is needed, and nobody's mouse is locked out.

Nothing in it knows what an application is. A terminal, a CAD package and something
installed yesterday get the same input path, the same safety rules and the same
cinematics, because the stage only knows rectangles, application ids, pointer paths and
key counts.

## The path an action takes

```text
 model proposes            policy authorises                    the compositor checks again
 ──────────────            ─────────────────                    ───────────────────────────
 Cerebra::propose_stage_action
   │  StageProposal
   ▼
 StageGate ── stage policy (apps.toml): is this application allowed this action?
   │        └ archie-policy Lease on stage:<app_id>, one operation
   │  AuthorizedStageCommand   (no public constructor)
   ▼
 Sealer ── HMAC-SHA256 over "archie-stage/1\0" + payload, rising sequence, TTL ≤ 10 s
   │  one JSON line on $XDG_RUNTIME_DIR/archie-stage/control.sock
   ▼
 archie-stage (C++) ── MAC first, before a byte is parsed
   │                 └ check_command: authority, surface, app id, geometry revision, bounds
   │                 └ keymap: can this text or chord be typed here at all?
   ▼
 Playback ── a Fitts-timed pointer path, presses, keys, on the stage's seat
   │
   ▼
 receipt: executed | cancelled | refusal code ── back to every connected client
```

Every caller goes through `archie_stage::Operator`. That includes `archie-stage-cli`, the
MCP server that lets IDE agents operate applications (see
[research/IDE.md](research/IDE.md)), and in time Cerebra's executor. The gate cannot be
skipped, because only its output can be sealed.

## Rules

1. **A person always wins.** Physical input takes the stage at once: a pointer moving more
   than 48 px within 400 ms, any button, wheel, touch or key, modifiers included. The action
   in flight stops, whatever it held is released, and the camera cuts to the whole stage.
   Archie resumes after eight quiet seconds. **Pause** or **Ctrl+Alt+Esc** holds the stage
   until **Ctrl+Alt+Enter**.
2. **The agent's input cannot trigger that.** Agent input goes into the compositor's seat
   directly and never passes the window's event path, so it can neither take over from
   itself nor press the pause or resume chord.
3. **Physical input does not leak into an action.** While Archie holds the stage,
   applications take no input from the scene. Otherwise Qt Quick would forward the hover
   it synthesises under a still physical pointer, and move the agent's pointer out from
   under its own click. A held physical key blocks new commands, because the compositor
   sends a held modifier's state to the focused application whatever happens to the key
   event.
4. **A command is bound to the window it was prepared for.** It names a surface, the
   application id and the geometry revision. A window that moved, resized, changed
   application or closed refuses it with `surface_changed`. A stale plan does not click
   wherever the window happens to be now.
5. **One action at a time.** A command that arrives during another is refused as
   `stage_busy`. A queue would run commands against a stage that has moved on.
6. **Typed text is never shown or logged.** Captions, the activity strip, receipts and
   traces carry verbs, application ids and counts. Text cannot contain control or
   bidirectional-override characters, so a newline is always an Enter key, and the policy
   can hold Enter in a terminal or Delete in CAD for a person's approval. Super and
   Ctrl+Alt chords are never sent.
7. **Receipts do not overstate.** A text the keyboard cannot type returns `not_typeable`,
   never `executed` (see Typing). A line the compositor cannot verify is answered with
   sequence 0, and only to the connection that sent it.
8. **Protected surfaces cannot be targeted.** The model refuses them. The compositor does not
   mark any surface protected yet; approval prompts and a lock screen are what the flag is
   for.
9. **Looking is authorised like touching.** A capture needs the application's `snapshot`
   grant and a lease to read it. A read lease cannot click, and a lease to click cannot
   look. The compositor checks a capture like any other command, grabs only that window, and
   writes it owner-only under `snapshots/` beside the control socket. It names the file only
   to the connection that asked, and the caption says "mirando" while it happens. The caller
   believes the file only if its SHA-256 and PNG signature match, and deletes it once read.
   The shipped policy grants no capture of anything.

## Typing

Applications read the agent's text the way they read a person's: as key events,
interpreted through the keymap the compositor gave them. `KeyTyper` finds the key for each
character on that keymap, holds Shift or AltGr when the character is on another level, and
moves the seat's modifier state exactly as a physical key does. A text with a character the
keymap lacks, such as "ñ" on a US layout, goes whole through the application's input
method, provided it has enabled text-input v2, v3 or Qt's text-input-method. A text never
mixes the two routes: applications read them through different queues, and a mixed text
arrived as "ñanúd" for "ñandú". An application with neither route refuses the command with
`not_typeable` before its first character.

Qt's public seat API is not enough for this. `QWaylandSeat::sendKeyEvent` sends a key
without updating modifiers, so Ctrl+C reaches the application as a plain "c".
`sendUnicodeKeyPressEvent` silently does nothing for an application without text-input.
So `KeyTyper` uses the private keyboard calls that Qt's own handling of a physical key
uses, in the same order. A Qt release that moves them fails the build instead of typing
wrongly.

The stage's keymap follows `XKB_DEFAULT_LAYOUT` and the other `XKB_DEFAULT_*` variables. On
a node it must match the physical keyboard, or a person typing after a takeover gets the
wrong characters. A Spanish or Latin American layout puts ñ, ¿ and ¡ on keys.

## Cinematics

Everything drawn comes from `StageDirector` (`display/visual/director.hpp`), a pure model
with no Qt, no clock and no socket, tested on machines without a display.

- **The camera frames the action, not the cursor.** It frames the region an action spans,
  with 180 px margin, at up to 1.8× zoom, easing zoom in log space over 650 ms. It holds
  that framing while later actions stay inside it. Typing frames the whole window, because
  the caret is somewhere the stage cannot see. After 2.6 s of quiet the camera returns to
  the whole stage. A takeover cuts to it in 260 ms.
- **The pointer moves like a hand.** It follows a minimum-jerk path along a slightly bowed
  curve, timed by Fitts's law between 160 and 900 ms. A press ripples once, with at most
  eight ripples on screen, and a carried drag shows a dashed line from where it was picked
  up.
- **The caption says what, never the content.** For example "Archie · escribiendo 25
  caracteres · qml". A refusal shows in amber with its reason.
- **The activity strip keeps the last five actions,** past tense and newest first, in the
  status line for 20 s. A person who looks up, or has just taken over, sees what the agent
  did without opening a log.
- **The status line says who holds the stage:** ARCHIE TRABAJA, CONTROL HUMANO or PAUSADO.
  A banner repeats it while a person holds the stage.
- **Motion costs frames only while something moves.** Each frame names when the next one is
  needed; a still stage requests nothing. Reduced motion cuts instead of easing and drops
  ripples. The conserve tier drops ripples. Headless draws nothing.
- **The tempo is a choice.** By default the pointer moves at a brisk person's speed, so
  somebody watching can follow it. `--tempo agent` runs the same playback on a quarter of
  the clock — a 150 px move in about 130 ms — and stops holding a capture on screen, which
  is what a four-action-a-second control loop needs. Nothing about what is checked, sealed
  or refused changes with it.

`archie-visual-stage` replays a scripted 16-second session through the same model and
writes an animated SVG. It uses 609 frames where a 60 fps loop would draw 960.

## Finding what a task names

`archie-sight` lets Archie act on something it is told about ("el botón Guardar") rather
than on coordinates. It joins IRIS and the stage and adds no authority of its own:

```text
 look      Operator::look               snapshot grant, read lease, compositor checks → PNG
                                        of the window, or of the one box that was asked for
 ground    LocateAnythingClient         the description in a one-entry Vault capsule for this
                                        task, the capture bound to its SHA-256 → one box
 activate  MotorController + Operator   a lease on the IRIS source, then an ordinary stage
                                        click at the box's centre: stage policy and lease again
 verify    the compositor's damage      the control redrew itself within 300 ms, or the
                                        click is reported as having done nothing
```

A grounding model answers in seconds and a hand works in tenths of one, so the model is
not in the loop. Archie asks the cheapest thing that can answer, in this order:

- **declared**: the application itself says where the control is, over the accessibility
  bus, in the coordinates of its own window. Every toolkit publishes this so that screen
  readers work, and `archie-a11y` reads it the same way. No capture, no model, about 11 ms;
- **trusted**: nothing has been drawn over a box it found earlier, so it is used as it is —
  no capture, no model;
- **confirmed**: something has, so Archie looks once, and the box still holds the same
  control. That look asks the compositor for the boxes it knows in that window rather than
  for the window, so one small capture also re-confirms every other control it knows there;
- **refound**: the box no longer holds it, so the local model in `archie-reflex` says where
  the controls in that capture are, and the one of about the same size and the same shape,
  nearest to where it was, is that control moved. Nothing is asked over a network;
- **grounded**: nothing else could say, or the window resized, so a grounding model is
  asked — and where the local model sees a control under that answer, the control is what
  is clicked, not the box the model drew.

The declared answer is read again for every action, so it cannot go stale, and it is only
used when it is unambiguous: a control whose name the task's words contain, of the kind the
task asked for, and exactly one of them. Anything else — a window that names nothing, two
controls with the same name, a toolkit with no bridge — falls through to looking. The
window is matched by the process the compositor reported for that surface, and when an
application has more than one window, by that window's size, because coordinates from the
wrong window are a click in the wrong place.

A session can be told the whole list of targets before it starts. `start_finding` takes one
capture, asks a grounding model about everything nothing else can answer for, and returns
at once; Archie goes on clicking what it already knows while the answers arrive, and a
target whose answer is in by the time it comes up costs nothing more than a click.
`archie-look --session` does that with the list on its input: in one run against a model
told to take 800 ms, the target that needed it waited 88 ms of that, because the asking had
happened while three other controls were being clicked.

What confirms a box is its own shape: a 9×8 grid of average brightness over it, compared
after bringing both to the same brightness. Hover, focus and a press move the whole box
and leave the shape alone; different writing in the same place does not. It decides
nothing — it only says whether to look again or to ask again.

### Looking at a box, not at a window

Pixels are a disclosure, so looking is authorised like touching — and asking for less of
them is both faster and less seen. A snapshot command may name one box of its surface. The
box travels sealed with the rest of the command and is re-checked against the window as it
is now: a box that pokes outside the window is refused, and so is one with no pixels in it,
which would otherwise read as "the whole window". The compositor encodes only those pixels
and says which corner of the window they came from, so what it hands over cannot be more
than what was asked for.

A look that only confirms what this session already knows asks for the boxes it knows in
that window, with eight pixels of context around them, whenever those come to less than
half of it; one such capture still re-confirms every one of them. A look that has to find
something asks for the window, because a model has to see the window to find anything in
it, and so does a control that has moved out of the box it was in.

### The local model

`archie-reflex` is the fast path IRIS's Visual Reflex contract describes, trained in this
repository in Rust: three convolutions and a head, about five thousand numbers, 5 KB
quantised to int8, one window looked at in about 9 ms on one core. It says where the
controls in a capture are and nothing else — not what they say, not what they do. Sight
uses it three ways:

- a grounding model points near a control; the local model knows where the control is, so
  the box that gets clicked is the control, not the answer;
- a box that no longer holds what it held is looked for again nearby, by size and by the
  same 9×8 shape, without asking anything over a network (**refound**);
- what there is to click at all, in a window that declares nothing.

Nobody labelled its corpus and no screen of anybody's was in it: applications that publish
their controls for screen readers label their own captures. `archie-teach` takes the
capture through the stage's snapshot gate, asks that application over the accessibility bus
where its controls are, and keeps the pair — a 160×96 grey tile, too small to read, and the
boxes in the tile's own coordinates. The corpus in `models/reflex/corpus` is 448 windows
and 3424 controls collected that way from `display/stage/demo/controls.qml`, which
rearranges itself every second and a half, under Qt's Basic, Fusion and Material styles.

```bash
cargo run --release -p archie-reflex --bin archie-reflex -- train --corpus models/reflex/corpus --out models/reflex/controls --epochs 200
cargo run --release -p archie-reflex --bin archie-reflex -- check --model models/reflex/controls --corpus models/reflex/corpus
archie-look … --reflex models/reflex/controls
```

On the 89 windows it was never taught, at the confidence Sight requires of it, it finds
740 of every 1000 declared controls, 672 of them near enough to click, and says 344 things
per 1000 that nothing declared. That last number is an upper bound on mistakes rather than
a count of them: an application declares its buttons and fields, not its labels and
panels, so a box around something real but undeclared counts against it here. `check`
sweeps the confidence so the trade is visible: at 600 it finds 784 and invents 453.

Training is plain stochastic gradient descent, written out rather than pulled in, and
seeded, so the same corpus gives the same model anywhere. Two numbers per epoch say
whether a run is still a run: **spread**, how much the model's answer varies from cell to
cell, and **alive**, how much of it still fires. The first attempt at this model learned
for one epoch, pushed three quarters of its own rectifiers permanently under zero with too
large a step, and then said the same thing about every cell — at a loss that looked
respectable, because saying "nothing is here" everywhere is nearly right. Rectifiers that
leak and a step ten times smaller fixed it; the two numbers are printed so that it cannot
happen quietly again.

```bash
archie-look --socket "$XDG_RUNTIME_DIR/archie-stage/control.sock" --key-file /etc/archie/stage/key --policy apps.toml --app org.qt-project.qml --find "el botón Guardar" --node form:save --kind button --model moonshotai/Kimi-K3 --screen public
```

```bash
archie-look --socket … --key-file … --policy apps.toml --app org.qt-project.qml --model moonshotai/Kimi-K3 --session
```

- The model answers with `<box> x1, y1, x2, y2 </box>` in 0–1000 coordinates, or IRIS
  refuses the answer. The box becomes window coordinates, and its centre is where Archie
  clicks.
- A click after which the control did not redraw ends with "world verification did not
  observe a newer scene revision" and exit code 4, not success, and the position is
  forgotten.
- `--session` reads one target per line — `<node> <kind> <description>` — keeping the
  stage connection, what it has learned and the connection to the model, and answers with
  one JSON line per activation: how the position was known, where the click went, and how
  many milliseconds each step took.
- `--screen` is the caller's statement of what the window shows. Without `--endpoint`, the
  model is reached through Nebius Token Factory and is cleared for public windows only,
  because nobody has verified what it retains. An internal window is refused before a byte
  leaves: "privacy grade internal is not permitted for model …". `--endpoint` names a local
  OpenAI-compatible server, such as a NIM serving LocateAnything, which may see internal
  windows.
- Reading what an application declares needs an accessibility bus in the session
  (`at-spi2-core`, which `at-spi-bus-launcher` starts under a `dbus-run-session`), and a
  toolkit that publishes to it — Qt does with `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`. With
  none of that, `archie-look` says so once and goes on looking. `--no-a11y` skips it, which
  is how the difference was measured.
- `cargo run -p archie-a11y --example declared -- <pid>` prints what one application says
  about its own controls, for telling "it declares nothing" apart from "Archie is asking
  the wrong window".
- `--reflex <dir>` loads the local model, and without it a grounding answer stands exactly
  as it came. Measured in WSLg on the demo form, with a stand-in model answering 15 px up
  and to the left of the button — near it, not on it: without the local model four clicks
  out of four landed on nothing, were reported as having changed nothing, and the form
  never saved; with it, all four landed on the button, the form saved four times, and the
  run went at 3.81 activations a second.
- The crate's `grounding_stub` example answers the same contract instantly from a box
  given on the command line, for driving the loop where no model runs and for measuring
  what the loop itself costs:
  `cargo run -p archie-sight --example grounding_stub -- --port 8099 --model local/stub --box "el botón Guardar=376,507,428,565"`.
- The Nebius key is read from `NEBIUS_TOKEN_FACTORY_KEY` and never printed. Captures have
  no persistence option: the verified temporary file is removed after it enters the
  bounded in-memory pass.

## Running it

### WSLg, for development

On an Arch environment with `qt6-base qt6-declarative qt6-wayland cmake ninja`:

```bash
cmake -S display/stage -B build/stage -G Ninja -DCMAKE_BUILD_TYPE=RelWithDebInfo -DARCHIE_STAGE_BUILD_TESTS=ON
```

```bash
cmake --build build/stage
```

```bash
cargo build --manifest-path core/Cargo.toml -p archie-stage --bin archie-stage-cli
```

Then the demo, in which Archie fills and saves a form with nobody's input:

```bash
STAGE=build/stage/archie-stage STAGE_CLI=core/target/debug/archie-stage-cli HOST_WAYLAND=/mnt/wslg/runtime-dir/wayland-0 GRABS=1 display/stage/demo/run-demo.sh
```

Clients inside WSLg have no EGL buffer sharing: set `QT_QUICK_BACKEND=software` for Qt
Quick applications and `LIBGL_ALWAYS_SOFTWARE=1` for GL ones.

### Driving it by hand

```bash
archie-stage-cli watch --socket "$XDG_RUNTIME_DIR/archie-stage/control.sock"
```

```bash
archie-stage-cli act --socket "$XDG_RUNTIME_DIR/archie-stage/control.sock" --key-file /etc/archie/stage/key --app org.freecad.FreeCAD click 420 310
```

Other actions: `right-click x y`, `double-click x y`, `move x y`, `drag x y to_x to_y`,
`scroll x y steps`, `type text…`, `key ctrl+s`, `focus`. Exit codes: 0 executed, 2
refused before sending, 3 needs approval (`--approve` is the person at the keyboard
giving it), 4 the compositor did not run it.

### On a node

`archie-stage` runs full screen on DRM/KMS through EGLFS (`QT_QPA_PLATFORM=eglfs`). The
policy is `/etc/archie/stage/apps.toml`; `archie-stage-cli keygen` writes a key readable
only by its owner.

## Tested, and how

- `display/visual`: the stage and director models in `test_stage` and `test_director`, as
  two of the library's 18 suites. Each rule was broken on purpose and a test failed for
  it: 42 mutants against the first version, then 14 more against the focus settle, the
  failure caption and the activity strip.
- `display/stage/tests/test_codec`: 42 checks on the C++ side of the sealed format, against
  `protocols/fixtures/stage-command-v1.json`, whose MAC was computed independently with
  `openssl`.
- `archie-stage`: gate, sealing, protocol client and `Operator`, with an in-memory
  compositor. For captures and repaints, the tests check six things:
  - A capture is read, verified and removed.
  - A capture whose digest is wrong, or whose file is outside the snapshot directory, is
    not believed.
  - A capture needs its own grant.
  - A read lease never touches.
  - A read abandoned halfway through a line resumes that line instead of starting inside
    it, which is what a loop that waits briefly for a repaint does all day.
  - The mirror answers what repainted where since a mark, says nothing about another
    window's repaints, and says it cannot tell rather than guessing once more has been
    redrawn than it keeps.
- `archie-sight`, with a mock compositor that redraws what it is clicked on and a loopback
  model:
  - A named button is found, clicked and seen to redraw, and the model receives the capsule
    and the image.
  - The model is asked once, however many times the button is pressed: the passes after it
    look and confirm.
  - A control nothing has been drawn over is used without looking at all.
  - A click after which nothing redraws is not verification, and that position is not kept.
  - A box holding something else is grounded again, and so is one in a window that moved.
  - A cloud model whose retention is unknown never sees an internal window, and nothing is
    clicked.
  - What confirms a box tells a hovered or pressed control, and a caret blinking in a
    field, from different writing in the same place.
- In WSLg, against real clients, reading their Wayland protocol traces:
  - The demo form saved "Archie escribió esto solo".
  - A Ctrl+A chord replaced a draft, and "Hola, ñandú: 100% listo!" arrived whole and in
    order.
  - `weston-terminal` ran `echo 'Archie > terminal' > /tmp/stage-proof`, typed with quotes
    and redirection, then Enter.
  - "ñ" into that terminal was refused as `not_typeable`.
  - Once, a mouse moved over the window took the stage, and Archie resumed on its own
    afterwards.
  - Through Nebius Token Factory, `moonshotai/Kimi-K3` completed one full demo flow. It
    placed the form's text field and Guardar button within a pixel of the centres the form
    itself reports; Archie clicked, typed and saved, and the snapshot directory was empty
    afterwards. This was not reliable: across five grounding calls it returned a usable
    answer three times. In the latest two timed calls it consumed the 512-token output
    budget while reasoning and returned no box.
  - Other vision models Nebius offered, on the same capture:
    - `google/gemma-3-27b-it` kept the box format but pointed 100 px or more away. Its click
      changed nothing, and `archie-look` reported that as a failure.
    - `openbmb/MiniCPM-V-4_5` came within 3 px but answered without `<box>`, which IRIS
      refuses.
    - `MiniMaxAI/MiniMax-M3` answered in format and missed.
    - `moonshotai/Kimi-K2.6` and `Kimi-K2.7-Code` used up 512 tokens reasoning.
    - Qwen3.5, GLM-5.3 and the Nemotron models refuse images.
  - The same window declared internal was refused before egress, and a policy without
    `snapshot` refused the capture with `stage_action_not_allowed`.
  - At `--tempo agent`, with grounding served by the crate's `grounding_stub` example so
    the numbers are Archie's own and not a provider's, twelve activations of the form's
    field and button ran back to back: all twelve clicked and were seen to redraw, in
    7.7 s — 1.56 a second. The two that had to ask took about 1.07 s each; the other ten
    confirmed what they already knew in 422–706 ms, averaging 550. Of that the click
    itself was 170–200 ms and waiting for the redraw was none of it, because the control
    had already redrawn by the time the receipt arrived. The form logged its six saves and
    no capture was left on disk.
  - With the accessibility bus in the session and the form naming its field, the same
    twelve activations ran entirely on what the application declares: no capture and no
    model at all, 4.84 a second, every one clicked and seen to redraw. Asking the
    application took about 11 ms, the click 166–208 ms, and the form logged its six saves.
  - With one target the application does not name and a stand-in model told to take 800 ms
    over it, eight activations ran in 2.0 s — 3.98 a second — and the one that needed the
    model waited 88 ms, because it had been asked while three other controls were being
    clicked.

## Not working yet

- **foot exits.** It needs `wl_seat` v5 and `wl_data_device_manager` v3; Qt 6.11's
  compositor offers v4 and v1. ArchieOS's default terminal does not run on the stage until
  that changes, or the stage moves to a compositor library that offers them.
- **Alacritty dies at full size** with its own `wl_shm_pool` stride error when it draws
  client-side decorations. It ran at half width.
- **Popups and menus are drawn but cannot be targeted.** Commands address toplevel
  surfaces, and a click on an open menu goes to the window beneath it.
- **Clients that ignore xdg-decoration** keep their own title bars inside the stage's
  frame, weston-terminal among them.
- **No XWayland**, so X11-only applications do not run.
- **Nothing has run on DRM/KMS hardware.**
- **Seeing is one window and one target per call.** A capture is the toplevel's own
  buffer, so menus and popups are neither in the picture nor within reach of the click.
- **"Changed" is not "right".** Verification shows that the window's pixels changed, not
  that the intended thing happened. A click that only takes focus away from a field
  passes.
- **LocateAnything-3B itself has not run.** Its adapter was exercised with cloud vision
  models standing in for it. The measured Nebius round trip was about 255 ms, a new TLS
  connection about 510 ms, and vision calls with Gemma or MiniCPM took 1.32–1.69 s.
  Kimi-K3 took 27.3–33.5 s when it answered, returned no box in two of five calls, and on
  a later day answered nothing at all: every request timed out while Gemma answered in
  about a second on the same account. Cloud grounding is a slow-path Cortex operation, not
  an interactive click loop.
- **A window that declares nothing still costs a capture.** The fastest path needs a
  toolkit that publishes to the accessibility bus and a control with a name on it. Without
  both — an unnamed field, a canvas, a game, a remote desktop — an activation falls back to
  a capture, and the first time to a grounding model, which the local model then puts on a
  real control. The local model itself costs about 9 ms; the capture around it is what
  costs.
- **The local model is small, and trained on one toolkit.** Its corpus is Qt windows from
  one demo, collected on one machine in an afternoon. It finds 740 of every 1000 declared
  controls on windows it never saw and draws 344 boxes per 1000 around things nothing
  declared. **refound** — a control found again after the window moved it, without asking
  anything — is covered by tests but has not been measured live: the demo that moves
  controls draws different ones each time, so there is nothing there to find again.
- **A capture is now the rendering, and little else.** A snapshot command may ask for one
  box of a surface rather than the whole of it, and a look that only confirms asks for the
  boxes this session knows in that window. Measured on the demo form, 355×88 pixels instead
  of 1534×696: encoding fell from 43 ms to 1–4 ms and a whole look from 135–590 ms to
  35–119 ms. What is left is Qt rendering the item to an offscreen image, 22–98 ms
  whatever size is asked for, because it redraws the window either way. Reading back only
  the damaged part, or the client's buffer directly, is what would cut that.
- **The pointer is now the slow part.** At agent tempo a verified click is 166–176 ms of
  trajectory, dwell and receipt, against a 35–119 ms look. Twelve activations of the demo
  form ran at 4.07 a second, and eight of a single control at 6.05.
- **Damage is often the whole window.** Qt Quick's software backend reported 1534×696 for
  16 of the 27 repaints in one run, so "the control redrew" frequently means no more than
  "the window redrew", and a control nothing has touched is rarely trusted without a look.
