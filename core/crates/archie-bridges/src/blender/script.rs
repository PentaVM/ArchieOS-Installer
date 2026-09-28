//! The one script the blender bridge runs inside `blender --background`, kept as a Rust
//! constant.
//!
//! Blender is automated in the Python it embeds; ArchieOS carries none. This text is data
//! to ArchieOS: written out at run time and run by Blender's own interpreter. It reads what
//! to do from the JSON file named by ARCHIE_JOB_ARGS and writes its answer to
//! ARCHIE_JOB_RESULT. Nothing a planner sends is executed: file names are opened, numbers
//! set settings, and nothing is evaluated.

pub const JOB: &str = r####"
import json
import math
import os
import traceback

import bpy
from mathutils import Vector

ARGS = json.load(open(os.environ["ARCHIE_JOB_ARGS"], encoding="utf-8"))
RESULT = os.environ["ARCHIE_JOB_RESULT"]


def answer(value):
    with open(RESULT, "w", encoding="utf-8") as out:
        json.dump(value, out)


IMPORTERS = {
    ".fbx": lambda p: bpy.ops.import_scene.fbx(filepath=p),
    ".glb": lambda p: bpy.ops.import_scene.gltf(filepath=p),
    ".gltf": lambda p: bpy.ops.import_scene.gltf(filepath=p),
    ".obj": lambda p: bpy.ops.wm.obj_import(filepath=p),
    ".stl": lambda p: (bpy.ops.wm.stl_import(filepath=p) if hasattr(bpy.ops.wm, "stl_import") else bpy.ops.import_mesh.stl(filepath=p)),
    ".ply": lambda p: bpy.ops.wm.ply_import(filepath=p),
    ".usd": lambda p: bpy.ops.wm.usd_import(filepath=p),
    ".usda": lambda p: bpy.ops.wm.usd_import(filepath=p),
    ".usdc": lambda p: bpy.ops.wm.usd_import(filepath=p),
    ".usdz": lambda p: bpy.ops.wm.usd_import(filepath=p),
    ".abc": lambda p: bpy.ops.wm.alembic_import(filepath=p),
}


def load(paths):
    first = paths[0]
    if first.lower().endswith(".blend") and len(paths) == 1:
        bpy.ops.wm.open_mainfile(filepath=first)
        return
    bpy.ops.wm.read_homefile(use_empty=True)
    for path in paths:
        ext = os.path.splitext(path)[1].lower()
        if ext == ".blend":
            with bpy.data.libraries.load(path, link=False) as (source, target):
                target.objects = list(source.objects)
            for obj in target.objects:
                if obj is not None:
                    bpy.context.scene.collection.objects.link(obj)
        elif ext in IMPORTERS:
            IMPORTERS[ext](path)
        else:
            raise ValueError("Blender does not import " + os.path.basename(path))


def meshes():
    return [o for o in bpy.context.scene.objects if o.type == "MESH" and o.visible_get()]


def bounds():
    points = [o.matrix_world @ Vector(c) for o in meshes() for c in o.bound_box]
    if not points:
        return Vector((0, 0, 0)), 1.0
    lo = Vector((min(p.x for p in points), min(p.y for p in points), min(p.z for p in points)))
    hi = Vector((max(p.x for p in points), max(p.y for p in points), max(p.z for p in points)))
    return (lo + hi) / 2, max((hi - lo).length / 2, 1e-3)


def inspect():
    scene = bpy.context.scene
    objects = []
    for obj in scene.objects:
        entry = {
            "name": obj.name,
            "type": obj.type,
            "location": [round(v, 4) for v in obj.location],
            "rotation_deg": [round(math.degrees(v), 2) for v in obj.rotation_euler],
            "dimensions": [round(v, 4) for v in obj.dimensions],
            "materials": [s.material.name for s in getattr(obj, "material_slots", []) if s.material],
        }
        if obj.type == "MESH":
            entry["vertices"] = len(obj.data.vertices)
            entry["faces"] = len(obj.data.polygons)
        objects.append(entry)
    centre, radius = bounds()
    return {
        "objects": objects[:2000],
        "object_count": len(objects),
        "frames": [scene.frame_start, scene.frame_end],
        "fps": scene.render.fps,
        "engine": scene.render.engine,
        "resolution": [scene.render.resolution_x, scene.render.resolution_y],
        "camera": scene.camera.name if scene.camera else None,
        "lights": [o.name for o in scene.objects if o.type == "LIGHT"],
        "centre": [round(v, 4) for v in centre],
        "radius": round(radius, 4),
    }


def studio(turntable=False):
    """A camera and lights for a scene that has none, framed on its meshes."""
    scene = bpy.context.scene
    centre, radius = bounds()
    added = []
    if scene.camera is None or turntable:
        data = bpy.data.cameras.new("ArchieCamera")
        data.lens = 50
        data.clip_end = radius * 100
        camera = bpy.data.objects.new("ArchieCamera", data)
        scene.collection.objects.link(camera)
        direction = Vector((1.0, -1.25, 0.8)).normalized()
        distance = radius / math.tan(data.angle / 2) * 1.25
        camera.location = centre + direction * distance
        track = camera.constraints.new("TRACK_TO")
        target = bpy.data.objects.new("ArchieTarget", None)
        scene.collection.objects.link(target)
        target.location = centre
        track.target = target
        track.track_axis = "TRACK_NEGATIVE_Z"
        track.up_axis = "UP_Y"
        scene.camera = camera
        added.append("camera")
        if turntable:
            pivot = bpy.data.objects.new("ArchiePivot", None)
            scene.collection.objects.link(pivot)
            pivot.location = centre
            camera.parent = pivot
            camera.location = direction * distance
            pivot.rotation_euler = (0, 0, 0)
            pivot.keyframe_insert("rotation_euler", index=2, frame=scene.frame_start)
            pivot.rotation_euler = (0, 0, 2 * math.pi)
            pivot.keyframe_insert("rotation_euler", index=2, frame=scene.frame_end + 1)
            for curve in pivot.animation_data.action.fcurves:
                for point in curve.keyframe_points:
                    point.interpolation = "LINEAR"
    if not any(o.type == "LIGHT" for o in scene.objects):
        for name, kind, energy, offset in (
            ("ArchieKey", "AREA", 800.0, Vector((1.2, -1.0, 1.4))),
            ("ArchieFill", "AREA", 250.0, Vector((-1.4, -0.6, 0.6))),
            ("ArchieRim", "AREA", 400.0, Vector((0.0, 1.5, 1.2))),
        ):
            data = bpy.data.lights.new(name, kind)
            data.energy = energy * radius * radius
            data.size = radius * 1.5
            light = bpy.data.objects.new(name, data)
            scene.collection.objects.link(light)
            light.location = centre + offset * radius * 2.5
            track = light.constraints.new("TRACK_TO")
            track.target = scene.objects.get("ArchieTarget") or camera_target(scene, centre)
            track.track_axis = "TRACK_NEGATIVE_Z"
        added.append("lights")
    # A soft studio backdrop only for a scene that brought no lighting of its own.
    if scene.world is None or "lights" in added:
        if scene.world is None:
            scene.world = bpy.data.worlds.new("ArchieWorld")
        scene.world.use_nodes = True
        background = scene.world.node_tree.nodes.get("Background")
        if background is not None:
            background.inputs[0].default_value = (0.92, 0.92, 0.9, 1.0)
            background.inputs[1].default_value = 0.6
    for obj in meshes():
        if not obj.material_slots:
            material = bpy.data.materials.get("ArchieClay") or bpy.data.materials.new("ArchieClay")
            material.diffuse_color = (0.36, 0.52, 0.74, 1.0)
            material.use_nodes = True
            bsdf = material.node_tree.nodes.get("Principled BSDF")
            if bsdf is not None:
                bsdf.inputs["Base Color"].default_value = (0.36, 0.52, 0.74, 1.0)
                bsdf.inputs["Roughness"].default_value = 0.45
            obj.data.materials.append(material)
            if "material" not in added:
                added.append("material")
    return added


def camera_target(scene, centre):
    target = bpy.data.objects.new("ArchieTarget", None)
    scene.collection.objects.link(target)
    target.location = centre
    return target


def configure(settings):
    scene = bpy.context.scene
    engine = settings.get("engine", "cycles")
    if engine == "cycles":
        scene.render.engine = "CYCLES"
        scene.cycles.device = "CPU"
        scene.cycles.samples = int(settings.get("samples", 64))
        scene.cycles.use_denoising = True
    elif engine == "eevee":
        names = [item.identifier for item in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items]
        scene.render.engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in names else "BLENDER_EEVEE"
    else:
        scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x = int(settings.get("width", 1600))
    scene.render.resolution_y = int(settings.get("height", 1000))
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = False
    scene.view_settings.view_transform = "AgX" if "AgX" in [t.identifier for t in bpy.types.ColorManagedViewSettings.bl_rna.properties["view_transform"].enum_items] else "Filmic"
    scene.render.image_settings.file_format = "PNG"


EXPORTERS = {
    "fbx": lambda p: bpy.ops.export_scene.fbx(filepath=p, path_mode="COPY", embed_textures=True),
    "glb": lambda p: bpy.ops.export_scene.gltf(filepath=p, export_format="GLB"),
    "gltf": lambda p: bpy.ops.export_scene.gltf(filepath=p, export_format="GLTF_SEPARATE"),
    "obj": lambda p: bpy.ops.wm.obj_export(filepath=p),
    "stl": lambda p: (bpy.ops.wm.stl_export(filepath=p) if hasattr(bpy.ops.wm, "stl_export") else bpy.ops.export_mesh.stl(filepath=p)),
    "ply": lambda p: bpy.ops.wm.ply_export(filepath=p),
    "usdc": lambda p: bpy.ops.wm.usd_export(filepath=p),
    "usdz": lambda p: bpy.ops.wm.usd_export(filepath=p),
    "abc": lambda p: bpy.ops.wm.alembic_export(filepath=p),
    "blend": lambda p: bpy.ops.wm.save_as_mainfile(filepath=p, copy=True),
}


def main():
    action = ARGS["action"]
    load(ARGS["files"])
    if action == "inspect":
        answer(inspect())
    elif action == "render":
        added = studio() if ARGS.get("studio", True) else []
        configure(ARGS)
        scene = bpy.context.scene
        if ARGS.get("frame") is not None:
            scene.frame_set(int(ARGS["frame"]))
        scene.render.filepath = ARGS["output"]
        bpy.ops.render.render(write_still=True)
        answer({"image": ARGS["output"], "engine": scene.render.engine, "added": added})
    elif action == "frames":
        scene = bpy.context.scene
        if ARGS.get("turntable"):
            scene.frame_start = 1
            scene.frame_end = int(ARGS["total"])
        added = studio(turntable=bool(ARGS.get("turntable"))) if ARGS.get("studio", True) else []
        configure(ARGS)
        scene.frame_start, scene.frame_end = int(ARGS["start"]), int(ARGS["end"])
        scene.render.filepath = os.path.join(ARGS["folder"], "frame_")
        bpy.ops.render.render(animation=True)
        answer({"rendered": [scene.frame_start, scene.frame_end], "added": added, "fps": scene.render.fps})
    elif action == "export":
        written = []
        for fmt in ARGS["formats"]:
            path = os.path.join(ARGS["folder"], ARGS["stem"] + "." + fmt)
            EXPORTERS[fmt](path)
            written.append(path)
        answer({"exported": written})
    elif action == "save":
        bpy.ops.wm.save_as_mainfile(filepath=ARGS["output"])
        answer(inspect())
    else:
        raise ValueError("unknown action " + str(action))


try:
    main()
except Exception as error:  # the bridge reads the error from the answer
    answer({"error": "%s: %s" % (type(error).__name__, error), "trace": traceback.format_exc()[-3000:]})
"####;
