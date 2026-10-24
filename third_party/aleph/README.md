# Aleph source provenance

Source: https://github.com/TheLoki2020/aleph
Pinned commit: ad68fc4a3c60fd7af3969597ef0ba886c63b0a40
Original author/copyright: Belluxx, 2026. MIT license retained in LICENSE.

The pinned source was cloned and inspected during implementation. This is a native
Rust adaptation of area/tile geometry, capture planning/checkpoints, atomic files,
paced HTTP/caching, OSM building extraction and terrain/export utilities. The Python
dashboard, its design, and Python runtime are not dependencies of ArchieOS.

Mapping: src/geo.py -> archie-geofabric::exploration; src/common.py and
src/capture.py -> archie-worldd::exploration_runtime; src/terrain.py and the GLB
export convention -> archie-worldd::geo_formats. Rust format readers use maintained
TIFF/earcutr libraries and existing ArchieOS types rather than copying parsers.

Implemented source adapters: public AWS/Tilezen terrain GeoTIFF, OpenStreetMap
closed building ways via bounded Overpass queries, Photon place lookup. Existing
C++ local PBF import remains unchanged. Google satellite/Street View/Google Earth
reverse-engineered mesh downloads are not enabled: the existing GeoFabric storage
and analysis restrictions still apply. This is not a claim of full Aleph provider
or file-format parity. Exported GLB contains terrain/buildings, not photogrammetry.

Data attribution is separate from the MIT code license. OSM requires contributor
credit and ODbL compliance. Terrain is a mosaic of source datasets; see
https://registry.opendata.aws/terrain-tiles/ and
https://github.com/tilezen/joerd/blob/master/docs/attribution.md (reviewed 2026-10-24).
