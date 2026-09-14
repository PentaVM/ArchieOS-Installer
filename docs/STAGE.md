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

`archie-visual-stage` replays a scripted 16-second session through the same model and
writes an animated SVG. It uses 609 frames where a 60 fps loop would draw 960.

## Finding what a task names

`archie-sight` lets Archie act on something it is told about ("el botón Guardar") rather
than on coordinates. It joins IRIS and the stage and adds no authority of its own:

```text
 look      Operator::look               snapshot grant, read lease, compositor checks → PNG
 ground    LocateAnythingClient         the description in a one-entry Vault capsule for this
                                        task, the capture bound to its SHA-256 → one box
 activate  MotorController + Operator   a lease on the IRIS source, then an ordinary stage
                                        click at the box's centre: stage policy and lease again
 verify    Operator::look               350 ms later, another capture; unchanged pixels fail
```

```bash
archie-look --socket "$XDG_RUNTIME_DIR/archie-stage/control.sock" --key-file /etc/archie/stage/key --policy apps.toml --app org.qt-project.qml --find "el botón Guardar" --node form:save --kind button --model moonshotai/Kimi-K3 --screen public --keep /tmp/look
```

- The model answers with `<box> x1, y1, x2, y2 </box>` in 0–1000 coordinates, or IRIS
  refuses the answer. The box becomes window coordinates, and its centre is where Archie
  clicks.
- A click after which the window's pixels did not change ends with "world verification
  did not observe a newer scene revision" and exit code 4, not success.
- `--screen` is the caller's statement of what the window shows. Without `--endpoint`, the
  model is reached through Nebius Token Factory and is cleared for public windows only,
  because nobody has verified what it retains. An internal window is refused before a byte
  leaves: "privacy grade internal is not permitted for model …". `--endpoint` names a local
  OpenAI-compatible server, such as a NIM serving LocateAnything, which may see internal
  windows.
- The Nebius key is read from `NEBIUS_TOKEN_FACTORY_KEY` and never printed. `--keep` saves
  the captures before and after, owner-readable.

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

```bash
archie-stage-cli look --socket "$XDG_RUNTIME_DIR/archie-stage/control.sock" --key-file /etc/archie/stage/key --app org.qt-project.qml --out form.png
```

`look` saves one window as an owner-readable PNG, if the policy grants that application
`snapshot`.

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
  compositor. For captures, the tests check four things:
  - A capture is read, verified and removed.
  - A capture whose digest is wrong, or whose file is outside the snapshot directory, is
    not believed.
  - A capture needs its own grant.
  - A read lease never touches.
- `archie-sight`, with a mock compositor and a loopback model:
  - A named button is found, clicked and seen to change, and the model receives the capsule
    and the image.
  - A window that does not change is not verification.
  - A cloud model whose retention is unknown never sees an internal window, and nothing is
    clicked.
- In WSLg, against real clients, reading their Wayland protocol traces:
  - The demo form saved "Archie escribió esto solo".
  - A Ctrl+A chord replaced a draft, and "Hola, ñandú: 100% listo!" arrived whole and in
    order.
  - `weston-terminal` ran `echo 'Archie > terminal' > /tmp/stage-proof`, typed with quotes
    and redirection, then Enter.
  - "ñ" into that terminal was refused as `not_typeable`.
  - Once, a mouse moved over the window took the stage, and Archie resumed on its own
    afterwards.
  - Through Nebius Token Factory, `moonshotai/Kimi-K3` placed the demo form's text field
    and its Guardar button within a pixel of the centres the form itself reports. Archie
    clicked the field, saw it change, typed, pressed Guardar and saw that change too. The
    form logged `saved "Archie encontró este campo mirando"`, and the snapshot directory
    was empty afterwards.
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
  models standing in for it. Kimi-K3 took 11 to 28 s to find each target, too slow for
  anything but a demonstration.
