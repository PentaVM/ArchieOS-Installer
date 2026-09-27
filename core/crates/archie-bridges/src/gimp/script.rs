//! The one script the gimp bridge runs inside GIMP 3's batch mode, kept as a Rust
//! constant, and the fixed line that starts it.
//!
//! GIMP is automated in the Python it embeds; ArchieOS carries none. This text is data to
//! ArchieOS: written out at run time and run by GIMP's own interpreter. It reads what to
//! do from the JSON file named by ARCHIE_JOB_ARGS and writes its answer to
//! ARCHIE_JOB_RESULT. Nothing a planner sends is executed: operations are looked up by
//! name in a fixed table, and their settings are numbers.

/// GIMP's `-b` argument: always this text, which runs the script file.
pub const BOOT: &str =
    "import os; exec(open(os.environ['ARCHIE_JOB_SCRIPT'], encoding='utf-8').read())";

pub const JOB: &str = r####"
import json
import os
import traceback

import gi
gi.require_version("Gimp", "3.0")
gi.require_version("Gegl", "0.4")
from gi.repository import Gegl, Gimp, Gio

ARGS = json.load(open(os.environ["ARCHIE_JOB_ARGS"], encoding="utf-8"))
RESULT = os.environ["ARCHIE_JOB_RESULT"]
RUN = Gimp.RunMode.NONINTERACTIVE


def answer(value):
    with open(RESULT, "w", encoding="utf-8") as out:
        json.dump(value, out)


def load(path):
    return Gimp.file_load(RUN, Gio.File.new_for_path(path))


def save(image, path, flatten):
    target = image
    if flatten:
        target = image.duplicate()
        target.merge_visible_layers(Gimp.MergeType.CLIP_TO_IMAGE)
    Gimp.file_save(RUN, target, Gio.File.new_for_path(path), None)
    if target is not image:
        target.delete()


def describe(image):
    layers = []
    for layer in image.get_layers():
        layers.append({
            "name": layer.get_name(),
            "visible": layer.get_visible(),
            "opacity": round(layer.get_opacity(), 1),
            "size": [layer.get_width(), layer.get_height()],
            "offset": list(layer.get_offsets()[1:]),
        })
    return {
        "size": [image.get_width(), image.get_height()],
        "mode": str(image.get_base_type()).split(".")[-1].split(" ")[0],
        "precision": str(image.get_precision()).split(".")[-1].split(" ")[0],
        "layers": layers,
    }


def gegl(layer, operation, settings):
    drawable_filter = Gimp.DrawableFilter.new(layer, operation, "")
    config = drawable_filter.get_config()
    for key, value in settings.items():
        config.set_property(key, value)
    drawable_filter.update()
    layer.merge_filter(drawable_filter)


def each_layer(image, fn):
    for layer in image.get_layers():
        fn(layer)


def apply(image, op):
    name, s = (op, {}) if isinstance(op, str) else next(iter(op.items()))
    if name == "resize":
        width = int(s.get("width", 0))
        height = int(s.get("height", 0))
        if width and not height:
            height = round(image.get_height() * width / image.get_width())
        if height and not width:
            width = round(image.get_width() * height / image.get_height())
        image.scale(width, height)
    elif name == "crop":
        image.crop(int(s["width"]), int(s["height"]), int(s.get("x", 0)), int(s.get("y", 0)))
    elif name == "rotate":
        turns = {90: Gimp.RotationType.DEGREES90, 180: Gimp.RotationType.DEGREES180, 270: Gimp.RotationType.DEGREES270}
        image.rotate(turns[int(s["degrees"])])
    elif name == "flip":
        image.flip(Gimp.OrientationType.HORIZONTAL if s.get("direction", "horizontal") == "horizontal" else Gimp.OrientationType.VERTICAL)
    elif name == "brightness_contrast":
        each_layer(image, lambda l: l.brightness_contrast(float(s.get("brightness", 0)), float(s.get("contrast", 0))))
    elif name == "hue_saturation":
        each_layer(image, lambda l: l.hue_saturation(Gimp.HueRange.ALL, float(s.get("hue", 0)), float(s.get("lightness", 0)), float(s.get("saturation", 0)), 0.0))
    elif name == "desaturate":
        each_layer(image, lambda l: l.desaturate(Gimp.DesaturateMode.LUMINANCE))
    elif name == "invert":
        each_layer(image, lambda l: l.invert(False))
    elif name == "auto_levels":
        each_layer(image, lambda l: l.levels_stretch())
    elif name == "blur":
        r = float(s.get("radius", 2))
        each_layer(image, lambda l: gegl(l, "gegl:gaussian-blur", {"std-dev-x": r, "std-dev-y": r}))
    elif name == "sharpen":
        each_layer(image, lambda l: gegl(l, "gegl:unsharp-mask", {"std-dev": float(s.get("radius", 1.5)), "scale": float(s.get("amount", 0.6))}))
    elif name == "text":
        font = Gimp.Font.get_by_name(s.get("font", "Sans-serif")) or Gimp.fonts_get_list("")[0]
        layer = Gimp.TextLayer.new(image, s["text"], font, float(s.get("size", 48)), Gimp.Unit.pixel())
        image.insert_layer(layer, None, 0)
        layer.set_offsets(int(s.get("x", 0)), int(s.get("y", 0)))
        color = Gegl.Color.new(s.get("color", "#ffffff"))
        layer.set_color(color)
    elif name == "flatten":
        image.flatten()
    else:
        raise ValueError("unknown operation " + str(name))


def main():
    action = ARGS["action"]
    image = load(ARGS["file"])
    if action == "inspect":
        answer(describe(image))
    elif action == "edit":
        for op in ARGS["operations"]:
            apply(image, op)
        save(image, ARGS["output"], ARGS["flatten"])
        if ARGS.get("preview"):
            save(image, ARGS["preview"], True)
        answer(describe(image))
    elif action == "export":
        written = []
        for path, flatten in ARGS["targets"]:
            save(image, path, flatten)
            written.append(path)
        answer({"exported": written, "image": describe(image)})
    else:
        raise ValueError("unknown action " + str(action))
    image.delete()


try:
    main()
except Exception as error:  # the bridge reads the error from the answer
    answer({"error": "%s: %s" % (type(error).__name__, error), "trace": traceback.format_exc()[-3000:]})
"####;
