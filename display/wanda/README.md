# Wanda relationship view

Dependency-free secondary interface for Wanda. Voice stays the main interaction;
this surface lets the owner inspect the same people, evidence and proposals.
It does not edit the graph, declare relationships, purchase gifts or send messages.

Serve this directory at the Wanda service origin: `index.html` at `/`, CSS with
`text/css` and `.mjs` modules with a JavaScript MIME type. No external scripts,
fonts or image services are required. Use the service's local paired token in the
connection dialog. It is stored in `sessionStorage` only, never in a URL, console,
localStorage or an analytics service. Browser storage failures fall back to memory.

The view calls protected, same-origin endpoints with `Authorization: Bearer …`:

- `GET /v1/domains`: `{ "domains": ["personal/owner"] }`; object entries
  `{ "domain": "…", "label": "…" }` and a top-level array are also supported.
  An empty configured list remains empty. Only a 404 permits the specified
  `personal/owner` default; no extra personal or work domains are fabricated.
- `GET /v1/view?domain=<encoded domain>`: the version 1 view envelope and version
  1 graph projection described in the Wanda specification. The returned domain
  must match the requested domain.
- `POST /v1/query`: `{ "domain": "…", "question": "…", "deep": false }`.
  Response: `{ "recommendations": [], "model": null, "state": "…" }`.
  Questions are bounded to 2,000 characters. Response text is bounded to 8 MiB.

Node `tags` and `traits` are trait objects with value/basis/redacted fields. Redacted
values do not enter search, tag filters or profile content. Declared relationships
and hypotheses have different line patterns and text badges. Unknown standing
stays unconfirmed. Layout traverses asserted edges only; hypotheses cannot create
a path to the owner. A filtered selection is cleared, and switching domains clears
the previous graph before fetching. Late responses from a previous domain are ignored.
Expired recommendations retain their explanation/evidence but withhold the suggested
action until sources are reviewed. All dynamic HTML text is escaped; SVG uses textContent.

The labelled demo is loaded only after clicking “Explorar una demo”. Its people,
relationships, commitments and cited preferences are explicit fixtures. It disables
real model queries and contains no invented current price or attraction claim.

Map controls: drag to pan; Ctrl/Command + wheel to zoom; buttons to zoom and fit;
Tab then Enter/Space to select a person; arrows to pan, +/- to zoom, 0 to fit.
The `/` shortcut focuses search outside form fields. List mode offers native buttons
for the same profiles. Small screens stack the profile below the map. Reduced-motion
preferences disable transitions. Empty, unconfigured, unsupported, authorization and
missing-model states preserve the actual availability of the service.

Run the focused tests from the repository root:

```powershell
node --test display/wanda/tests/model.test.mjs
node --check display/wanda/app.mjs
node --check display/wanda/model.mjs
node --check display/wanda/demo.mjs
```

Eleven tests cover accent-insensitive search, trait-object filtering and redaction,
selection, C++ trait bases, unknown/inferred standing, malformed optional fields,
hostile recommendation text, expiration, graph placement, configured domains and
cross-domain/version rejection while accepting an unconfigured null graph.
The initial tests were observed failing before implementation. These pure tests do
not prove browser layout, screen-reader behavior, touch gestures or live inference.
Browser QA should cover desktop/mobile screenshots, SVG click/keyboard selection,
pan/zoom, list/profile navigation, pairing, domain changes during pending requests,
server failures, actual DTOs and the opt-in fixture. Root integration owns that pass.

Task 5 UI ruling: assets are isolated under `display/wanda`; the global Quickshell,
Orbe and shell-bridge navigation remain owned by Xavi/Claude. The planned optional
shell bridge extension is deferred to those owners; the API and static surface are
the integration interface.
