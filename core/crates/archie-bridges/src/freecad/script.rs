//! The one script the freecad bridge runs inside FreeCADCmd, kept as a Rust constant.
//!
//! FreeCAD is automated in the Python it embeds; ArchieOS carries none. This text is data
//! to ArchieOS: written out at run time and run by FreeCAD's own interpreter. It reads what
//! to do from the JSON file named by ARCHIE_JOB_ARGS and writes its answer to
//! ARCHIE_JOB_RESULT. Nothing a planner sends is executed: names become object names,
//! numbers become lengths, and expressions go to FreeCAD's expression engine, which cannot
//! run code.

pub const JOB: &str = r####"

import json
import os
import re
import traceback

import FreeCAD as App

ARGS = json.load(open(os.environ["ARCHIE_JOB_ARGS"], encoding="utf-8"))
RESULT = os.environ["ARCHIE_JOB_RESULT"]
NAME = re.compile(r"^[A-Za-z][A-Za-z0-9_]{0,63}$")


def answer(value):
    with open(RESULT, "w", encoding="utf-8") as out:
        json.dump(value, out)


def load(path):
    lower = path.lower()
    if lower.endswith(".fcstd"):
        return App.openDocument(path)
    doc = App.newDocument("Imported")
    if lower.endswith((".step", ".stp", ".iges", ".igs", ".brep", ".brp")):
        import Import

        Import.insert(path, doc.Name)
    elif lower.endswith((".stl", ".obj", ".3mf", ".ply")):
        import Mesh

        Mesh.insert(path, doc.Name)
    else:
        raise ValueError("FreeCAD does not read " + os.path.basename(path))
    doc.recompute()
    return doc


def results(doc):
    """The shapes a person sees: objects with a shape that nothing else consumes."""
    found = []
    for obj in doc.Objects:
        shape = getattr(obj, "Shape", None)
        if shape is None or shape.isNull():
            continue
        if any(hasattr(parent, "Shape") for parent in obj.InList):
            continue
        found.append(obj)
    return found


def number(value):
    return float(getattr(value, "Value", value))


def describe_shape(obj):
    shape = obj.Shape
    box = shape.BoundBox
    entry = {
        "name": obj.Name,
        "label": obj.Label,
        "type": obj.TypeId,
        "valid": bool(shape.isValid()),
        "solids": len(shape.Solids),
        "volume_mm3": round(shape.Volume, 4),
        "area_mm2": round(shape.Area, 4),
        "bbox_mm": [round(v, 4) for v in (box.XMin, box.YMin, box.ZMin, box.XMax, box.YMax, box.ZMax)],
        "size_mm": [round(box.XLength, 4), round(box.YLength, 4), round(box.ZLength, 4)],
    }
    if shape.Solids:
        centre = shape.Solids[0].CenterOfMass if len(shape.Solids) == 1 else box.Center
        entry["centre_mm"] = [round(centre.x, 4), round(centre.y, 4), round(centre.z, 4)]
    return entry


def parameters(doc):
    found = {}
    for sheet in doc.Objects:
        if sheet.TypeId != "Spreadsheet::Sheet":
            continue
        for cell in sheet.getUsedCells():
            alias = sheet.getAlias(cell)
            if not alias:
                continue
            value = sheet.get(cell)
            entry = {"sheet": sheet.Name, "cell": cell}
            try:
                entry["value"] = number(value)
                unit = str(getattr(value, "Unit", "") or "")
                if unit and unit != "Unit: ()":
                    entry["unit"] = getattr(value, "getUserPreferred", lambda: [str(value)])()[0]
            except (TypeError, ValueError):
                entry["value"] = str(value)
            found[alias] = entry
    return found


def inspect(doc):
    shapes = [describe_shape(obj) for obj in results(doc)]
    return {
        "document": doc.Name,
        "objects": len(doc.Objects),
        "results": shapes,
        "parameters": parameters(doc),
        "valid": all(s["valid"] for s in shapes) and bool(shapes),
        "volume_mm3": round(sum(s["volume_mm3"] for s in shapes), 4),
    }


def set_parameters(doc, values):
    sheets = [o for o in doc.Objects if o.TypeId == "Spreadsheet::Sheet"]
    changed = {}
    for alias, value in values.items():
        for sheet in sheets:
            cell = sheet.getCellFromAlias(alias)
            if cell:
                changed[alias] = {"was": parameters(doc).get(alias, {}).get("value")}
                sheet.set(cell, str(value))
                break
        else:
            raise KeyError("no spreadsheet alias " + alias)
    doc.recompute()
    return changed


# Building from a description: primitives, booleans, fillets, bound to named parameters.

PRIMITIVES = {
    "box": ("Part::Box", ["Length", "Width", "Height"]),
    "cylinder": ("Part::Cylinder", ["Radius", "Height", "Angle"]),
    "sphere": ("Part::Sphere", ["Radius"]),
    "cone": ("Part::Cone", ["Radius1", "Radius2", "Height"]),
    "torus": ("Part::Torus", ["Radius1", "Radius2"]),
    "wedge": ("Part::Wedge", ["Xmin", "Xmax", "Ymin", "Ymax", "Zmin", "Zmax", "X2min", "X2max", "Z2min", "Z2max"]),
}
IDENT = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\b")


def bind(obj, prop, value, names):
    """A number sets the property; text is an expression over the parameters."""
    if isinstance(value, (int, float)):
        if "." in prop:
            obj.setExpression(prop, repr(float(value)))
        else:
            setattr(obj, prop, float(value))
        return
    if not isinstance(value, str) or len(value) > 200:
        raise ValueError(obj.Name + "." + prop + " is a number or an expression")
    expression = IDENT.sub(lambda m: "Params." + m.group(1) if m.group(1) in names else m.group(1), value)
    obj.setExpression(prop, expression)


def build(doc, spec):
    names = set()
    params = spec.get("parameters", {})
    if params:
        sheet = doc.addObject("Spreadsheet::Sheet", "Params")
        for row, (alias, value) in enumerate(params.items(), start=1):
            if not NAME.match(alias):
                raise ValueError("parameter names are letters, digits and _: " + alias)
            sheet.set("A%d" % row, alias)
            sheet.set("B%d" % row, str(value))
            sheet.setAlias("B%d" % row, alias)
            names.add(alias)
        doc.recompute()
    made = {}
    for part in spec["parts"]:
        name = part["name"]
        if not NAME.match(name) or name in made:
            raise ValueError("part names are unique letters, digits and _: " + str(name))
        kind = part["kind"]
        if kind in PRIMITIVES:
            type_id, props = PRIMITIVES[kind]
            obj = doc.addObject(type_id, name)
            for prop in props:
                key = prop.lower()
                if key in part:
                    bind(obj, prop, part[key], names)
        elif kind in ("cut", "fuse", "common"):
            base = made[part["base"]]
            tools = [made[t] for t in part.get("tools", [])]
            if kind == "cut":
                tool = tools[0]
                if len(tools) > 1:
                    tool = doc.addObject("Part::MultiFuse", name + "_tools")
                    tool.Shapes = tools
                obj = doc.addObject("Part::Cut", name)
                obj.Base, obj.Tool = base, tool
            else:
                obj = doc.addObject("Part::MultiFuse" if kind == "fuse" else "Part::MultiCommon", name)
                obj.Shapes = [base] + tools
        elif kind in ("fillet", "chamfer"):
            base = made[part["base"]]
            doc.recompute()
            obj = doc.addObject("Part::Fillet" if kind == "fillet" else "Part::Chamfer", name)
            obj.Base = base
            size = float(part.get("radius", part.get("size", 1.0)))
            edges = part.get("edges", "all")
            count = len(base.Shape.Edges)
            chosen = range(1, count + 1) if edges == "all" else [int(e) for e in edges]
            obj.Edges = [(i, size, size) for i in chosen if 1 <= i <= count]
        else:
            raise ValueError("unknown kind " + str(kind))
        at = part.get("at")
        if at is not None:
            for axis, value in zip("xyz", at):
                bind(obj, "Placement.Base." + axis, value, names)
        rotate = part.get("rotate")
        if rotate is not None:
            axis = rotate.get("axis", [0, 0, 1])
            base_point = obj.Placement.Base
            obj.Placement = App.Placement(base_point, App.Rotation(App.Vector(*axis), float(rotate["angle"])))
        if "label" in part:
            obj.Label = str(part["label"])[:100]
        made[name] = obj
    doc.recompute()
    return made


# Exports.

def mesh_of(objects, deflection=None):
    import Mesh
    import MeshPart

    mesh = Mesh.Mesh()
    for obj in objects:
        shape = obj.Shape
        d = deflection or max(shape.BoundBox.DiagonalLength / 1000.0, 0.005)
        mesh.addMesh(MeshPart.meshFromShape(Shape=shape, LinearDeflection=d, AngularDeflection=0.35))
    return mesh


def export(doc, formats, folder, stem):
    import Import
    import Part

    objects = results(doc)
    if not objects:
        raise ValueError("the document has no shapes to export")
    written = []
    for fmt in formats:
        path = os.path.join(folder, stem + "." + fmt)
        if fmt in ("step", "iges"):
            Import.export(objects, path)
        elif fmt == "brep":
            Part.makeCompound([o.Shape for o in objects]).exportBrep(path)
        elif fmt in ("stl", "obj", "3mf", "ply", "amf"):
            mesh_of(objects).write(path)
        elif fmt == "fcstd":
            doc.saveCopy(path)
        else:
            raise ValueError("unknown format " + fmt)
        written.append(path)
    return written


def main():
    action = ARGS["action"]
    if action == "inspect":
        answer(inspect(load(ARGS["file"])))
    elif action == "build":
        doc = App.newDocument(ARGS.get("document", "Part"))
        build(doc, ARGS["spec"])
        doc.saveAs(ARGS["output"])
        answer(inspect(doc))
    elif action == "set":
        doc = load(ARGS["file"])
        changed = set_parameters(doc, ARGS["parameters"])
        report = inspect(doc)
        report["changed"] = changed
        if ARGS.get("output"):
            doc.saveAs(ARGS["output"])
        if ARGS.get("folder"):
            report["exported"] = export(doc, ARGS["formats"], ARGS["folder"], ARGS["stem"])
        answer(report)
    elif action == "export":
        doc = load(ARGS["file"])
        answer({"exported": export(doc, ARGS["formats"], ARGS["folder"], ARGS["stem"]), "summary": inspect(doc)})
    else:
        raise ValueError("unknown action " + str(action))


try:
    main()
except Exception as error:  # the bridge reads the error from the answer
    answer({"error": "%s: %s" % (type(error).__name__, error), "trace": traceback.format_exc()[-3000:]})
"####;
