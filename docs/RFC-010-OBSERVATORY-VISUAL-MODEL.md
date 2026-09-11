# RFC-010 — The observatory's visual model

- **Status:** accepted, implemented as `display/visual`; the renderer that consumes it is
  not written
- **Builds on:** RFC-006, which put an unprivileged bridge between `archied` and the
  renderer. This one is about what the renderer is then told to draw.

## 1. Problem

RFC-006 established that the observatory must be beautiful and must be incapable of
harming the control plane. It settled the second question and left the first as an
aspiration.

The gap it left is that *every decision about what to show lived in QML*. Which node is
which between two polls, how old a reading may be before it stops being animated, how
much the machine can afford to draw, which nodes fit that budget, where a node belongs
and why, what colour it is — all of it was expressed as bindings inside a scene graph.

That has two consequences, and the second is the serious one.

**None of it can be checked.** A decision that can only be observed by looking at a
running window on a machine with a GPU is a decision that will not be checked, and the
observatory ships inside an ISO built by a container that has no screen. There were no
tests under `display/` at all.

**Some of those decisions are claims.** A display that animates a node is saying the
thing is happening now. One that draws a node in the calm colour is saying it is fine.
One that shows four hundred of a machine's four thousand modules without mentioning it
is saying the machine is quieter than it is. ArchieOS is careful about what it asserts
everywhere else — Sentry will not name a face without consent, the scheduler will not
call an unreachable node broken — and the screen was the one surface where a claim could
be made by accident, as a side effect of a binding nobody thought of as an assertion.

## 2. Decision

Split the observatory in two.

```text
archie-displayd / archie-worldd     projections over HTTP
→ archie::visual                    what to show          no Qt, no clock, no socket
→ display/native (Qt Quick 3D)      how to draw it
```

`display/visual` decides what appears, at what level of detail, in what colour, where the
camera goes and what may move. It links no Qt, opens no socket and reads no clock: every
method that needs the time is given it. `Observatory::tick` returns a `Frame` of pure
data, and the renderer applies it without deriving anything of its own.

The boundary is the point. The half of the observatory that makes claims is the half that
can be compiled and tested on a machine with no GPU, which is where CI runs.

## 3. The rules, and the lie each one prevents

Every module exists because there is a specific way a status display misleads. None of
these is a style preference.

**Freshness is a visual channel** (`freshness.hpp`). Every panel is a claim about a
machine somewhere else, and a stale value looks exactly like a fresh one until someone
acts on it. Age costs confidence, and animation stops before the data disappears, because
motion is what a viewer reads as "happening now". A timestamp from the future is reported
as a broken clock rather than rewarded — naive age arithmetic makes a misconfigured node
win every most-recent comparison and animate at full confidence forever.

**The render budget comes from the scheduler's evidence** (`budget.hpp`). Not from a
frame-time counter, which reacts after the damage and makes the observatory part of the
problem it is displaying. Kernel pressure and thermal telemetry decide the tier, so the
display yields for the same reason and at the same instant the scheduler does. A rich
tier requires positive evidence of headroom: an unreadable sensor, an available flag with
no value behind it, and a four-minute-old reading all resolve cautiously. Demotion is
immediate and promotion waits out a dwell. A saturated machine is never blanked —
`Headless` means nobody is looking and must not be reachable as a performance decision.

**Node identity survives refreshes** (`graph.hpp`). A node is its id, not its index; it
holds a layout seat for its whole life; one absent poll starts a grace period rather than
a death. This is what makes any smooth transition possible at all: if identity is
recomputed per payload, every poll is a fresh scene and the only honest way to draw it is
a cut.

**Position means something** (`layout.hpp`, `geo.hpp`). Radius is dependency depth, so
the scene reads outward from its foundations. Angle comes from the node's slot through
the golden angle, so arrivals fill gaps instead of pushing everyone. Height is severity.
Observations go where the provider said they are. Nothing is hashed: a hash gives a scene
that is stable and completely uninformative, which is worse than an obvious grid because
it invites the viewer to read a relationship that is not there.

**Colour carries severity twice** (`palette.hpp`). Hue and luminance both move with it,
so the ordering survives greyscale and colour blindness — "red means trouble" tells
roughly one man in twelve nothing. Age and authority reduce opacity and never touch hue:
a stale critical node is still a critical node, and fading it toward calm would turn "we
have lost contact with something that was on fire" into "everything is fine".

**Authority is separate from age** (`ingest.hpp`). A cached batch from a fallback source
is current and unauthoritative at the same time. It defaults to false, because the
optimistic reading of silence is what the field exists to refuse.

**Events are derived, never invented** (`events.hpp`). Every pulse comes from a graph
delta. The queue is bounded, coalesces per node so a walk from 1 to 5 is one event rather
than three, and counts what it dropped.

**The visible set holds still** (`lod.hpp`). Taking the top N by importance each frame
makes nodes blink along the cutoff several times a second, so the eye goes to whatever is
flickering rather than whatever matters. Incumbents must be beaten by half a severity
level. Whatever does not fit is counted, so the scene can say "and 47 more".

**The operator owns the view** (`scene.hpp`). Nothing switches modes on an alarm or an
arriving observation. A display that reorganises itself because something happened
elsewhere takes the screen away from whoever was reading it, at the moment they were most
likely reading it for a reason.

**A missing field is not a zero** (`ingest.hpp`). Reading an absent severity as 0 draws a
healthy node for something the display knows nothing about, which is worse than showing
nothing because it is indistinguishable from good news. An unknown enum is drawn and
marked rather than refused, since refusing means every control-plane upgrade blanks part
of the observatory. One bad record costs one record.

## 4. The four views

Cerebra is the view of record and shows everything, including kinds this build cannot
classify — a thing missing from Cerebra is missing from the observatory. Brain is the
cognition subsystem, Watcher the world, Fabric the other nodes. Each of the three shows
only what it is about; showing the rest alongside drowns the part it exists to isolate. A
morph consults both ends so nodes fade rather than popping.

What a view excludes is counted separately from what did not fit, because the difference
matters to whoever reads it: an aggregated node was too much for the tier, a filtered one
was never meant to be there.

## 5. Testing

The library is checked the way the trusted crates are: every rule has a test, and every
test has been mutation-tested by deliberately breaking the rule to confirm the test
fails. Where a mutation survived it was because the code was unreachable or the test was
weak, and both were fixed rather than recorded as passing.

Several defects were found that way rather than by writing tests: an angle computed in
single precision that collapsed neighbouring nodes onto the same point at four thousand
slots, a collision test too narrow to notice that 137 degrees repeats exactly every 360
slots, a dependency-cycle report that named two of three tangled modules, and presence
that settled on the next payload rather than on the clock — which left the whole scene
permanently arriving whenever a projection went quiet.

`archie-visual-probe` walks a scripted scenario and prints every frame. Its output is
checked in and CI diffs against it: the model has no clock and no randomness, so any
difference is a decision that changed, which is the class of regression a unit test does
not catch and that would otherwise only be noticed by looking at a screen.

## 6. Cost

The render budget hands the renderer a frame interval, and that budget is for drawing.
Whatever this library spends deciding what to draw comes out of the same 16ms before a
triangle is submitted, so it is measured rather than assumed. `archie-visual-bench`
prints it at the populations the tiers permit.

The first measurement was bad news: 27ms at the Ultra tier's own 4000-node cap, which is
169% of a frame, and 10ms at Normal's 1500. Almost all of it was recomputing pure
functions. Dependency depth and layout depend only on the graph and both ran every frame,
depth twice. Edges are likewise a function of declared dependencies and were rebuilt
every frame for a list that had not changed since the last poll. The timeline entry —
four thousand samples with an id each — was built in full and then discarded by a
`History` that refuses most entries, because a still scene changes nothing.

All three are now keyed on a graph generation counter, and the timeline entry is built
only once the cheap facts say it will be kept. That is 27ms to 9ms at 4000 nodes and 10ms
to 3ms at 1500.

The counter is worth a warning. It first meant "anything that could move a node", which
excluded presence, because a departing node stands in exactly the same place. The history
pre-check used the same counter, so the timeline silently stopped recording the moment
anything began to leave — precisely the moment someone scrubs back to find, and no
existing test noticed. It now means "anything a consumer could notice". The narrow
contract saved one layout walk per departure and cost a class of history nobody would
have known was missing.

The caps in `RenderProfile` are deliberately *not* tuned to fit these numbers. The
measurement comes from one loaded development machine rather than the Arch node this runs
on, and the cost of drawing those nodes has not been measured at all, so a cap chosen to
fit half of an unmeasured budget would be the same guess in a new place. What is
established is that the model's own cost is not free at the top end, and that both halves
need measuring together on hardware.

## 7. What is implemented now

The model, in full: freshness, budget, identity, camera, modes, level of detail, events,
projection ingest for modules, peers, models and world observations, layout, geography,
palette and a seekable timeline, composed behind `Observatory::tick`. Fourteen suites,
green under `-Werror` with no Qt and no GPU, plus the probe and its golden gate.

## 8. What is not

**The renderer.** `display/native/qml` does not consume any of this yet. Nothing in this
RFC reaches a screen until the QML is written against `Frame` and run on real hardware.

That work needs a machine with Qt6 and a display. CI's Arch container can compile the Qt
client but cannot render it, so the screenshot gate RFC-006 left open still needs
physical hardware — and so does any claim that the result looks like anything.
