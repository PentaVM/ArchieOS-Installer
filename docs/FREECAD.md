# FreeCAD tool layer

The Arch image installs FreeCAD from the official `extra` repository. The package version
recorded by the alpha build is the source of truth; the builder resolved `1.1.3-2` on
2026-09-13. WSLg can launch the GUI, while automation uses `FreeCADCmd`.

The typed surface covers document create/open/inspect, primitive creation, parameter
changes, Python execution, recompute, validation, STEP/STL export and preview rendering.
An `AuthorizedWorkspace` exists only after a live `archie-policy` filesystem lease is
validated and consumed. Paths must be relative, traversal-free and canonicalize below
the workspace. The executor invokes a fixed binary without a shell and has no network
interface.

Run the real headless smoke test inside ArchieOS:

```bash
sudo -u archie /usr/lib/archie/freecad-smoke-test
```

It creates a parameterized 80 x 40 x 5 mm motor bracket with an 8 mm through-hole,
recomputes it, checks that it is one valid solid, and exports FCStd, STEP and STL files to
`/archie/workspaces/freecad-smoke`. These dimensions are an explicit smoke-test fixture;
the user-facing design flow must clarify missing critical dimensions before generation.

