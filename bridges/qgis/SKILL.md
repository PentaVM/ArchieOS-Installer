---
name: bridge-qgis
description: Analyse and convert geographic data through Archie's qgis bridge — buffers, clips, reprojections, joins, zonal statistics, terrain analysis, any of QGIS's processing algorithms — and export Shapefile/KML/GeoPackage/GeoTIFF for ArcGIS or Google Earth. Use when a task involves maps, coordinates or spatial data.
---

# GIS through Archie

QGIS runs headless and without Python. Everything goes through its processing
algorithms and GDAL.

## First, know the data

- `qgis_layers` lists the files.
- `qgis_info {file}` gives:
  - for vectors: each layer's feature count, geometry, CRS, extent and fields;
  - for rasters: size, bands, CRS and extent.

Check CRSs before combining layers. Distances in a geographic CRS (EPSG:4326) are in
degrees, not metres; reproject to a projected CRS first.

## Running algorithms

1. `qgis_algorithms {search: "clip raster"}` finds candidates.
2. `qgis_algorithm_help {algorithm}` lists the parameters, their types and defaults.
3. `qgis_run {algorithm, inputs}`:
   - Layer parameters are paths in the workspace. Name one table of a file as
     `data.gpkg|layername=roads`.
   - Outputs are plain file names (`buffered.gpkg`, `slope.tif`), written to
     `exports/<algorithm>-<n>/`. An output you do not name goes there too.
   - Numbers, booleans, enum indexes, CRSs (`"EPSG:32719"`) and extents pass as given.

Examples:

- Buffer: `native:buffer {INPUT: "roads.gpkg", DISTANCE: 50, OUTPUT: "roads-50m.gpkg"}`.
- Reproject: `native:reprojectlayer {INPUT, TARGET_CRS: "EPSG:32719", OUTPUT}`.
- Clip a raster: `gdal:cliprasterbymasklayer {INPUT, MASK, OUTPUT}`.
- Slope: `native:slope {INPUT: "dem.tif", Z_FACTOR: 1, OUTPUT: "slope.tif"}`.
- Zonal statistics: `native:zonalstatisticsfb {INPUT, INPUT_RASTER, STATISTICS: [2], OUTPUT}`.

## Exports for other software

`qgis_convert {file, to}`:

| Format | For |
|---|---|
| shp | ArcGIS, older tools |
| gpkg | modern everything, one file |
| geojson | the web |
| kml | Google Earth |
| dxf | CAD |
| csv | spreadsheets |
| tif | GeoTIFF |
| png | a picture |

## Seeing the result

`qgis_map {layers, extent, width}` draws layers bottom to top over an extent written as
`"xmin,xmax,ymin,ymax [EPSG:code]"`, into `maps/map-<n>/map.png`:

- rasters are warped underneath;
- each vector layer gets its own colour, with translucent fills and ringed points;
- every layer is reprojected to the extent's CRS.

No QGIS project is needed. Use it to check where things are, and deliver it beside the
data.

## Traps

- An algorithm that reaches a database or the network is not run here.
- Large rasters take minutes. The limit is an hour per run.
- Enum parameters take indexes. `qgis_algorithm_help` shows the options in order.

## When to use the GUI instead

Use QGIS on the Archie Stage for:

- styling;
- labels;
- print layouts.
