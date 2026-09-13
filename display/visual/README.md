# `display/visual` — what the observatory shows

This library decides **what** appears on screen. `display/native` draws it.

The split is the design, not an accident of layout. Everything here answers a question
that has a right answer — how old is this reading, what can the machine afford to draw,
which node is this, has the camera arrived, does this node fit — and none of those
questions needs a scene graph to ask or a screen to answer. So this library links no Qt,
opens no socket and reads no clock, which means the interesting half of the observatory
is testable on a machine with no GPU. A decision that can only be checked by looking at a
running window is a decision that will not be checked.

## The rule underneath all of it

The display may not claim more than it knows. Every module here exists because there is
a specific way a status display lies:

| Header | The lie it prevents |
|---|---|
| `freshness.hpp` | A four-minute-old reading drawn exactly like a current one. Motion stops before data disappears, because motion is what a viewer reads as "happening now". |
| `budget.hpp` | Spending a GPU the machine needed. The tier comes from the same kernel pressure and thermal telemetry the scheduler uses, and a rich tier needs positive evidence of headroom. |
| `cognition.hpp` | A spinner that turns whether or not Archie is doing anything, at sixty frames a second. Motion starts only from a changed `HudProjection`, a running pass breathes only while its reading is fresh, "no data" never looks like a finished silent pass, and every frame says when the next one is needed — so a still panel costs one frame per freshness boundary. |
| `graph.hpp` | Recomputing identity every poll, so every refresh is a cut. A node is its id, holds its seat for life, and a single absent poll is a grace period rather than a death. |
| `layout.hpp` | Position that looks like structure and carries none. Radius is dependency depth, angle is the node's slot through the golden angle, height is severity. Nothing is hashed. |
| `palette.hpp` | "Red means trouble", which tells roughly one man in twelve nothing. Luminance rises with severity too, and age never touches hue. |
| `camera.hpp` | A move that snaps back to where the last one started. A new instruction is obeyed from wherever the camera actually is. |
| `scene.hpp` | The screen reorganising itself because something happened elsewhere. Only the operator changes the view, and each view shows what it is about rather than being a camera angle on one scene. |
| `lod.hpp` | Nodes blinking in and out along the budget cutoff, so the eye goes to whatever flickers. Incumbents must be beaten by a clear margin. |
| `events.hpp` | Pulses for things that did not happen, and a queue that grows without bound. Every event is derived from a graph delta. |
| `ingest.hpp` | Reading a missing field as zero, which draws a healthy node for something unknown. |
| `geo.hpp` | A globe that looks plausible and is mirrored, or puts an event off Fiji somewhere off Somalia. Landmarks, antipodes and handedness are pinned. |
| `history.hpp` | Answering "what was true at 14:02" with the state from 14:05. |
| `observatory.hpp` | The places where two correct modules still add up to a bug. |

## Using it

```cpp
archie::visual::Observatory observatory;

observatory.ingest_snapshot(snapshot, age_ms, now_ms);
observatory.ingest_world_payload(world, age_ms, now_ms);
observatory.ingest_kernel_payload(kernel, age_ms, display_present, now_ms);
observatory.request_mode(archie::visual::SceneMode::Watcher);  // operator input only

const archie::visual::Frame frame = observatory.tick(now_ms);
```

`Frame` is pure data. Every node arrives with its position, shape, detail level, colour,
opacity and a single `animate` boolean already decided, so the renderer applies the frame
rather than deriving anything of its own.

`Value` is a neutral tree, deliberately not a JSON parser: whether bytes are valid JSON
is the transport's job and Qt already does it. Whether a *valid* payload says something
usable is this library's, and that is the half where a display quietly starts showing the
wrong thing.

## Tests

```bash
cmake -S display/visual -B build/visual -G Ninja -DARCHIE_VISUAL_BUILD_TESTS=ON
cmake --build build/visual
ctest --test-dir build/visual --output-on-failure
```

16 suites, 7159 checks, `-Wall -Wextra -Wpedantic -Wswitch-enum -Werror`. Every module has
been mutation-tested: each rule above was deliberately broken to confirm a test fails.
Where a mutation survived, it was because the code was unreachable or the test was weak,
and both were fixed rather than recorded as passing.

## Status

The model is implemented, tested, and **is what the observatory draws**. `ArchieBridge`
fetches the three projections, hands them to `Observatory` through `Payload.cpp`, and
publishes the `Frame` it gets back; `display/native/qml/Main.qml` renders that and decides
nothing of its own.

It did not start that way. The bridge derived node positions from an FNV hash scattered
over a fixed shell, colours from a table of state strings, and marker scale from a
magnitude — three inventions sitting beside a tested model that says something different
about each of them. Nothing could see the disagreement, because nothing tested the half
that was actually on screen.

What remains unproved is the *rendering*, not the deciding. The frame has been verified
against a live node on Windows: one peer, no modules, clean ingest, tier `normal` because
PSI and thermal are unreadable on that platform and the governor will not spend what it
cannot measure. Nobody has yet watched this on the Arch node it is built for, at a node
count where the budget actually bites, so the caps in `RenderProfile` are still the
unmeasured half of the pair.

## Seeing the cognition engine move

```bash
cmake -S display/visual -B build/visual -G Ninja -DARCHIE_VISUAL_BUILD_TOOLS=ON
cmake --build build/visual --target archie-visual-cognition
build/visual/archie-visual-cognition cerebra-motion.svg
```

Runs a scripted pass — a pause mid-sentence the floor waits out, a question, context from
the vault, local reasoning, a tool call held for approval, execution, verification, and
memory declining to keep it — and writes an animated SVG of exactly the frames the engine
asked for. It prints the cost beside a fixed loop: on that script, 281 frames in the
normal tier, 127 in conserve and 16 with reduced motion, against 937 for a 60 fps timer.

The contract for a renderer is in `cognition.hpp` and is short: call `frame()` when a
projection arrives and at `next_frame_at_ms`, and never on a timer of its own. One
integration requirement follows from the freshness rule: while a pass is running or
held for approval, `archie-displayd` must re-publish the HUD projection within the
lagging window, or the panel will — correctly — dim a pass it can no longer vouch for.
