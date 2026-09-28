---
name: bridge-gimp
description: Retouch and convert images through Archie's gimp bridge — resize, crop, levels, colour, sharpen, captions — and export layered PSD for Photoshop, TIFF for print, or PNG/JPEG/WebP. Use when a task edits a photo or picture, or needs a PSD.
---

# Images through Archie

GIMP 3 runs headless in one folder of images. It opens XCF, PSD, PNG, JPEG, TIFF, WebP,
GIF, BMP and HEIC.

## Reading

`gimp_images` lists the images in the workspace.

`gimp_inspect {file}` gives:

- size, colour mode and precision;
- every layer's name, visibility, opacity, size and offset.

## Retouching

`gimp_edit {file, operations, format}` applies operations in order and writes
`edits/<name>-<n>.<format>` with a picture. Operations:

- **Geometry:**
  - `{"resize": {"width": 1600}}` — give one side to keep proportions
  - `{"crop": {"x", "y", "width", "height"}}`
  - `{"rotate": {"degrees": 90}}`
  - `{"flip": {"direction": "horizontal"}}`
- **Tone:**
  - `"auto_levels"`
  - `{"brightness_contrast": {"brightness": 0.1, "contrast": 0.15}}` — values from −1 to 1
  - `{"hue_saturation": {"hue": 0, "lightness": 0, "saturation": 20}}`
  - `"desaturate"`
  - `"invert"`
- **Detail:**
  - `{"sharpen": {"radius": 1.5, "amount": 0.6}}`
  - `{"blur": {"radius": 3}}`
- **Captions:**
  - `{"text": {"text": "…", "x": 40, "y": 40, "size": 64, "color": "#ffffff", "font": "Sans-serif"}}`
- **Layers:**
  - `"flatten"`

Formats `psd`, `xcf` and `tif` keep layers. The others are flattened copies.

A good default for a photo: `auto_levels`, a light `sharpen`, then `resize` to the size it
is needed at.

## Exports

`gimp_export {file, formats}` writes `exports/<name>-<n>/`.

| Format | For |
|---|---|
| psd | Photoshop and Affinity, with layers |
| tif | print |
| png | lossless |
| jpg | photos on the web |
| webp | smaller web images |
| pdf | documents |

## When to use the GUI instead

Use GIMP on the Archie Stage for:

- painting;
- selections by eye;
- healing and cloning;
- masks.

Check the result with `gimp_inspect`.
