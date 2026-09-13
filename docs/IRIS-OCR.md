# IRIS OCR

Native document, DOM and accessibility text bypasses OCR. `OcrSpecialist` is used only
when a requested ROI lacks structured text; its descriptor declares locality and outputs
are bounded. No OCR engine is bundled, so an unconfigured specialist returns an explicit
provider error instead of fabricated text.
