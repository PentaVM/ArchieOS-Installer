#!/usr/bin/env bash
set -euo pipefail

freecad=${FREECAD_CMD:-FreeCADCmd}
workspace=${ARCHIE_FREECAD_WORKSPACE:-/archie/workspaces/freecad-smoke}
case "$(realpath -m "$workspace")" in /archie/workspaces/*) ;; *) echo 'workspace outside /archie/workspaces' >&2; exit 1;; esac
install -d -m 0750 "$workspace"
script=$(mktemp --tmpdir="$workspace" .archie-freecad-smoke.XXXXXX.py)
trap 'rm -f "$script"' EXIT
cat > "$script" <<'PY'
import FreeCAD as App
import Part
import Import
import Mesh

doc = App.newDocument("MotorBracket")
params = doc.addObject("App::FeaturePython", "Parameters")
params.addProperty("App::PropertyLength", "Width").Width = 80
params.addProperty("App::PropertyLength", "Height").Height = 40
params.addProperty("App::PropertyLength", "Depth").Depth = 5
params.addProperty("App::PropertyLength", "HoleDiameter").HoleDiameter = 8
body = doc.addObject("PartDesign::Feature", "Bracket")
base = Part.makeBox(params.Width, params.Depth, params.Height)
hole = Part.makeCylinder(params.HoleDiameter / 2, params.Depth,
                         App.Vector(params.Width / 2, 0, params.Height / 2),
                         App.Vector(0, 1, 0))
body.Shape = base.cut(hole)
doc.recompute()
if body.Shape.isNull() or not body.Shape.isValid() or len(body.Shape.Solids) != 1:
    raise RuntimeError("invalid solid")
doc.saveAs("motor-bracket.FCStd")
Import.export([body], "motor-bracket.step")
Mesh.export([body], "motor-bracket.stl")
print("ARCHIE_FREECAD_OK solid=1 parametric=1")
PY
(cd "$workspace" && "$freecad" "$script")
test -s "$workspace/motor-bracket.FCStd"
test -s "$workspace/motor-bracket.step"
test -s "$workspace/motor-bracket.stl"
echo "FreeCAD smoke test passed: $workspace"

