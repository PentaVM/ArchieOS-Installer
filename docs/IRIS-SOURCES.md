# IRIS sources

IRIS prefers native structure over pixels. Browser DOM/accessibility text, FreeCAD objects
and Blender scene objects enter as typed snapshots. Desktop pixels are a bounded fallback;
camera/video use a latest-frame queue whose revocation clears buffers; static PPM input is
decoded once. Remote sources accept semantic deltas, never a video stream.

The crate provides real contracts and deterministic adapters. Production AT-SPI/UIA,
compositor damage, WebDriver, camera-device and Blender connectors are **BLOCKED** until
their runtimes and hardware are installed.
