# ArchieOS Jarvis workflow orb

## Intent and reference

The native observatory must show a translucent, luminous orbital graph with visible points, arcs, and local concentrations like the supplied 1.2-second gold Jarvis clip. The heat map in the TFOG preprint is a visual reference for density, not a source of coordinates. The LinkedIn post is a motivation to prioritize and aggregate large event streams, not evidence that ArchieOS processes the post's claimed volume.

The live screen must show what this ArchieOS instance is actually observing. Wanda, Cortex, Vision, and Iris are named workflow lanes, but a lane without recent source evidence must say waiting or unavailable. No decorative sample may be counted as a real process or event. A separately marked simulation may be used for design previews.

## Current failure

`archie-orb::build_field` includes a degree-zero harmonic and a fixed radius of 0.82, clamps the accumulated field to `[-1,1]`, and outputs a closed triangle mesh. On the target WSL instance the feed contains only the OS and one peer. Two impulses cannot produce the preprint's 360-420-node topology; the opaque mesh hides the few nodes and edges that do exist. `archie-displayd` also re-upserts unchanged entities every second, causing avoidable graph rebuilds.

## Design

1. Preserve TFOG's semantic graph and execution boundary. Compute an angular von Mises-Fisher density over the visible set, center and scale it using its sample mean and variance, and apply a bounded radial deformation. The field is an envelope, not the only primitive. Structural graph updates are recomputed only on actual feature/topology changes.
2. Add a read-only, bounded local inventory in `archie-displayd`: process identity and activity from `/proc`, active service membership from cgroup paths, and a small device summary from `/sys`. Use stable process identity including start ticks. Read no command line, environment, file contents, screen pixels, or private user paths. Missing sources produce no invented entities. Poll independently from `archied` and use explicit caps and LOD.
3. Batch sparse field lines and point glints into Quick3D geometry. Use gold emissive traces, local brightness from real entity positions/activity, a very faint surface, and gentle presentation rotation. A 2D light pass projects the same field samples when WSLg cannot display HDR bloom reliably. Keep actual entity nodes and relation edges visually distinct. All decoration is a sampling of the computed field, not an entity count.
4. Carry bounded workflow observations from existing source components into four visible lanes. An observation contains only component, phase, and count; no transcript, image, social graph, or command content. Stage and Policy retain action authority. Show source age and waiting state. Do not call Wanda, Cortex, Vision, or Iris merely to animate the display.

## Limits and proof

The WSL target currently exposes about 60 processes and 97 service units; Wanda has no runtime call site and the Iris champion file is absent there. The orb must present those limits truthfully. Target profile is a 128 MiB display daemon and low GPU budget. The final review will compare live entity counts, finite geometry, a native render-check PNG, CPU/memory load, and workflow state against source availability. The existing GitHub Actions billing block remains separate from code verification.
