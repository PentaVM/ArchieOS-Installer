# TFOG Orb rendering budget

The Orb draws field samples and observed graph edges as batched Quick 3D meshes. Parallels and meridians follow the field's sampled angles at a fixed display radius to keep a coherent Dyson shell; field concentration controls the intensity of its intersections. Observed nodes are grouped into stable type sectors for display, while the TFOG model retains its original positions. Short circuit traces start at those nodes. Curved tunnels are selected deterministically across all hierarchy and dependency edges. A forward iris reads equatorial field density and counter-rotates to keep its gaze as the shell turns. Animation changes shader uniforms and root rotation without selecting new points every frame. Broad ribbons and glint halos use the same positions as narrow paths. They are visual layers, not extra entities.

The previous QML Canvas reprojected field samples, drew hundreds of strokes, and blurred every node every 100 ms. On an AMD Radeon Graphics / WSLg DZN path it caused Qt GUI polish stalls up to 3.6 s. With the Canvas removed, polish was normally 0 ms. The measured residual cost is the WSLg Vulkan present path (about 52 ms median swap); that limits this host to roughly 16 displayed frames/s even when mesh rendering itself takes only a few milliseconds. This is a display path bottleneck, not a requirement for an AI workstation.

The local inventory records process identities, installed systemd service unit names, active cgroup membership, device classes, and the existence of fixed public system directories. Installed units with no active member have zero activity; they are not presented as running services. No user file path or content is scanned. The low profile admits up to 256 visible TFOG nodes; all of their positions are batched into a single point mesh, while at most 96 prominent nodes get pickable 3D models. The screen reports registered and visible counts separately. Curved inner tunnels follow only hierarchy, dependency, or runtime edges; they remain inside the shell.

## Per-device controls

- `ARCHIE_ORB_POLL_MS`: semantic Orb fetch interval, default 500 ms, clamped to 200–2000 ms. Lower values react faster; higher values reduce JSON and mesh updates.
- `ARCHIE_ORB_GRID_STEP`: sampled row stride for the Dyson shell, default 1, clamped to 1–4. Larger values draw fewer parallels and meridians.
- `ARCHIE_ORB_MAX_TUNNELS`: cap for curved paths along observed structural relations, default 48 on the `low` profile and 128 otherwise, clamped to 0–192.
- The existing TFOG field `profile` controls sample density. The existing render tier suppresses glint halos under resource pressure.
- The `low` Orb profile disables MSAA in the Orb viewport. Higher profiles continue to use the render tier's sample budget.

For graphics profiling, launch `archie-display` with `QSG_RENDER_TIMING=1` and inspect `Frame prepared` (GUI polish), `syncAndRender` (render and swap), and frame intervals. Keep the timing log off for normal use. These measurements are from the WSLg integration, not native GPU throughput.

The Orb only observes speech, focus, Stage, follow-up, and accessibility state. Policy and Stage retain execution authority.
