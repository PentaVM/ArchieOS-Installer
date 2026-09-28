//! QGIS: geographic data, analysed with QGIS's own processing algorithms and GDAL.
//!
//! `qgis_process --no-python` runs any native, GDAL or PDAL algorithm headless, with its
//! parameters as JSON on standard input. The bridge asks the algorithm for its own
//! parameter list first, then resolves every input layer inside the workspace and sends
//! every output into a fresh folder, so a parameter cannot read or write elsewhere.

pub mod map;

use crate::manifest::Manifest;
use crate::run::Run;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../../bridges/qgis/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../../bridges/qgis/SKILL.md");

const LIMIT: Duration = Duration::from_secs(3600);
const READS: [&str; 16] = [
    "gpkg", "shp", "geojson", "json", "kml", "kmz", "gml", "gpx", "csv", "fgb", "tif", "tiff",
    "vrt", "asc", "las", "laz",
];
const PROVIDERS: [&str; 5] = ["native", "qgis", "gdal", "3d", "pdal"];
/// Algorithms that reach databases or the network, which this bridge does not run.
const REFUSED: [&str; 6] = [
    "download",
    "postgis",
    "executesql",
    "database",
    "httprequest",
    "oracle",
];
const INPUT_TYPES: [&str; 9] = [
    "layer",
    "source",
    "vector",
    "raster",
    "mesh",
    "pointcloud",
    "file",
    "multilayer",
    "vectortile",
];
const OUTPUT_TYPES: [&str; 7] = [
    "sink",
    "vectorDestination",
    "rasterDestination",
    "fileDestination",
    "folderDestination",
    "pointCloudDestination",
    "vectorTileDestination",
];
const MAX_LISTED: usize = 80;
const MAX_IMAGE: usize = 700 * 1024;

pub struct Qgis {
    manifest: Manifest,
    workspace: Workspace,
    programs: Programs,
}

fn algorithm_allowed(id: &str) -> Result<(), String> {
    let (provider, name) = id
        .split_once(':')
        .ok_or("an algorithm id is provider:name, such as native:buffer")?;
    let plain =
        |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if !plain(provider) || !plain(name) {
        return Err(format!("{id} is not an algorithm id"));
    }
    if !PROVIDERS.contains(&provider) {
        return Err(format!(
            "this bridge runs algorithms from {PROVIDERS:?}, not {provider}"
        ));
    }
    let lower = name.to_ascii_lowercase();
    if let Some(word) = REFUSED.iter().find(|w| lower.contains(*w)) {
        return Err(format!(
            "{id} reaches a database or the network ({word}); this bridge does not run it"
        ));
    }
    Ok(())
}

impl Qgis {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            programs: programs.clone(),
        }
    }

    fn qgis(&self, args: &[&str], input: Option<Value>) -> Result<Value, String> {
        let mut run = Run::new(self.programs.get("qgis_process"))
            .args(["--no-python", "--json"])
            .args(args)
            .env("QT_QPA_PLATFORM", "offscreen")
            .cwd(self.workspace.root())
            .limit(LIMIT);
        if let Some(input) = input {
            run = run.input(serde_json::to_vec(&input).map_err(|e| e.to_string())?);
        }
        let ran = run.run().map_err(|e| e.to_string())?;
        // qgis_process can print notices before its JSON.
        let text = ran
            .stdout
            .find('{')
            .map(|at| &ran.stdout[at..])
            .unwrap_or("");
        match serde_json::from_str::<Value>(text) {
            Ok(value) if ran.success() => Ok(value),
            _ => Err(format!(
                "qgis_process failed (exit {:?}): {}",
                ran.code,
                ran.said()
            )),
        }
    }

    fn layer(&self, value: &str) -> Result<String, String> {
        // A layer may name one table of a file: roads.gpkg|layername=roads.
        let (file, suffix) = match value.split_once('|') {
            Some((file, suffix)) => (file, Some(suffix)),
            None => (value, None),
        };
        let path = self.workspace.file(file).map_err(|e| e.to_string())?;
        let mut out = path.display().to_string();
        if let Some(suffix) = suffix {
            if !suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b'_' | b'-' | b'.' | b'|'))
            {
                return Err(format!("{value}: the part after | is layername=… only"));
            }
            out.push('|');
            out.push_str(suffix);
        }
        Ok(out)
    }

    fn info(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(file) = arguments.get("file").and_then(Value::as_str) else {
            return Outcome::error("qgis_info needs file");
        };
        let path = match self.workspace.file(file) {
            Ok(path) => path,
            Err(e) => return Outcome::error(e.to_string()),
        };
        // gdalinfo reads rasters; ogrinfo reads vectors. Try the raster reading first.
        let raster = Run::new(self.programs.get("gdalinfo"))
            .args(["-json", "-nomd"])
            .arg(&path)
            .limit(Duration::from_secs(120))
            .run();
        if let Ok(ran) = raster {
            if ran.success() {
                if let Ok(info) = serde_json::from_str::<Value>(&ran.stdout) {
                    let crs = info["coordinateSystem"]["wkt"].as_str().and_then(crs_name);
                    return Outcome::json(&json!({
                        "kind": "raster",
                        "driver": info["driverShortName"],
                        "size": info["size"],
                        "bands": info["bands"].as_array().map(|b| b.iter().map(|band| json!({"type": band["type"], "nodata": band["noDataValue"], "colour": band["colorInterpretation"]})).collect::<Vec<_>>()),
                        "crs": crs,
                        "corners": info["cornerCoordinates"],
                        "wgs84_extent": info["wgs84Extent"],
                    }));
                }
            }
        }
        let vector = Run::new(self.programs.get("ogrinfo"))
            .args(["-json", "-so", "-al", "-ro"])
            .arg(&path)
            .limit(Duration::from_secs(120))
            .run();
        match vector {
            Ok(ran) if ran.success() => match serde_json::from_str::<Value>(&ran.stdout) {
                Ok(info) => {
                    let layers: Vec<Value> = info["layers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|layer| {
                            let geometry = &layer["geometryFields"][0];
                            json!({
                                "name": layer["name"],
                                "features": layer["featureCount"],
                                "geometry": geometry["type"],
                                "crs": geometry["coordinateSystem"]["wkt"].as_str().and_then(crs_name),
                                "extent": geometry["extent"],
                                "fields": layer["fields"].as_array().map(|f| f.iter().map(|field| json!({"name": field["name"], "type": field["type"]})).collect::<Vec<_>>()),
                            })
                        })
                        .collect();
                    Outcome::json(
                        &json!({"kind": "vector", "driver": info["driverShortName"], "layers": layers}),
                    )
                }
                Err(_) => Outcome::error("ogrinfo gave no JSON; GDAL 3.7 or later is needed"),
            },
            Ok(ran) => Outcome::error(format!("neither GDAL nor OGR reads {file}: {}", ran.said())),
            Err(e) => Outcome::error(e.to_string()),
        }
    }

    fn algorithms(&self, arguments: &Map<String, Value>) -> Outcome {
        let search = arguments
            .get("search")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let list = match self.qgis(&["list"], None) {
            Ok(list) => list,
            Err(why) => return Outcome::error(why),
        };
        let mut found = Vec::new();
        let mut total = 0;
        for (provider, details) in list["providers"].as_object().into_iter().flatten() {
            if !PROVIDERS.contains(&provider.as_str()) {
                continue;
            }
            for (id, algorithm) in details["algorithms"].as_object().into_iter().flatten() {
                let name = algorithm["name"].as_str().unwrap_or("");
                let group = algorithm["group"].as_str().unwrap_or("");
                let haystack = format!("{id} {name} {group}").to_ascii_lowercase();
                if algorithm_allowed(id).is_ok()
                    && search.split_whitespace().all(|w| haystack.contains(w))
                {
                    total += 1;
                    if found.len() < MAX_LISTED {
                        found.push(json!({"id": id, "name": name, "group": group}));
                    }
                }
            }
        }
        Outcome::json(
            &json!({"algorithms": found, "matching": total, "truncated": total > MAX_LISTED}),
        )
    }

    fn help(&self, id: &str) -> Result<Value, String> {
        algorithm_allowed(id)?;
        self.qgis(&["help", id], None)
    }

    fn algorithm_help(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(id) = arguments.get("algorithm").and_then(Value::as_str) else {
            return Outcome::error("qgis_algorithm_help needs algorithm");
        };
        match self.help(id) {
            Ok(help) => {
                let parameters: Vec<Value> = help["parameters"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(name, p)| {
                        json!({
                            "name": name,
                            "type": p["type"],
                            "description": p["description"],
                            "optional": p["optional"],
                            "default": p["default_value"],
                            "options": p["options"],
                        })
                    })
                    .collect();
                Outcome::json(&json!({
                    "algorithm": id,
                    "name": help["algorithm_details"]["name"],
                    "summary": help["algorithm_details"]["short_description"],
                    "parameters": parameters,
                }))
            }
            Err(why) => Outcome::error(why),
        }
    }

    /// Runs `id` with `inputs`, every layer resolved inside the workspace and every output
    /// placed in a fresh `exports/<algorithm>-<n>/`. Returns the results and the folder.
    fn run_algorithm(
        &self,
        id: &str,
        inputs: &Map<String, Value>,
    ) -> Result<(Value, PathBuf), String> {
        let help = self.help(id)?;
        let declared = help["parameters"].as_object().cloned().unwrap_or_default();
        let folder = self
            .workspace
            .fresh_folder("exports", &id.replace(':', "-"))
            .map_err(|e| e.to_string())?;
        let mut resolved = Map::new();
        for (name, value) in inputs {
            let Some(parameter) = declared.get(name) else {
                return Err(format!(
                    "{id} has no parameter {name}; qgis_algorithm_help lists them"
                ));
            };
            let kind = kind(parameter);
            let value = if INPUT_TYPES.contains(&kind) {
                match value {
                    Value::String(layer) => json!(self.layer(layer)?),
                    Value::Array(layers) => Value::Array(
                        layers
                            .iter()
                            .map(|l| {
                                l.as_str()
                                    .ok_or("layers are paths".to_string())
                                    .and_then(|l| self.layer(l))
                                    .map(Value::from)
                            })
                            .collect::<Result<_, _>>()?,
                    ),
                    _ => return Err(format!("{name} is a layer path in the workspace")),
                }
            } else if is_output(parameter) {
                let file = value
                    .as_str()
                    .ok_or_else(|| format!("{name} is an output file name"))?;
                if !crate::delivery::is_name(file) {
                    return Err(format!(
                        "{name}: an output is one plain file name such as buffered.gpkg"
                    ));
                }
                json!(folder.join(file))
            } else {
                match value {
                    Value::Object(_) => return Err(format!("{name} takes a plain value")),
                    Value::String(text) if looks_like_a_path(text) => {
                        return Err(format!(
                            "{name} is not a layer or output parameter; a path there is refused"
                        ))
                    }
                    other => other.clone(),
                }
            };
            resolved.insert(name.clone(), value);
        }
        // Outputs the planner did not name still go to the folder, never to a temporary
        // place outside the workspace.
        for (name, parameter) in &declared {
            let kind = kind(parameter);
            if is_output(parameter)
                && !resolved.contains_key(name)
                && parameter["optional"] != json!(true)
            {
                let extension = match kind {
                    "rasterDestination" => "tif",
                    "folderDestination" => "",
                    "fileDestination" => "html",
                    _ => "gpkg",
                };
                let file = if extension.is_empty() {
                    name.to_ascii_lowercase()
                } else {
                    format!("{}.{extension}", name.to_ascii_lowercase())
                };
                resolved.insert(name.clone(), json!(folder.join(file)));
            }
        }
        let answer = self.qgis(&["run", id, "-"], Some(json!({"inputs": resolved})))?;
        Ok((self.relative_paths(answer["results"].clone()), folder))
    }

    fn relative_paths(&self, value: Value) -> Value {
        match value {
            Value::String(text)
                if Path::new(text.split('|').next().unwrap_or("")).is_absolute() =>
            {
                Value::String(
                    self.workspace
                        .relative(Path::new(text.split('|').next().unwrap_or(""))),
                )
            }
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|v| self.relative_paths(v)).collect())
            }
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, self.relative_paths(v)))
                    .collect(),
            ),
            other => other,
        }
    }

    fn run_tool(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(id) = arguments.get("algorithm").and_then(Value::as_str) else {
            return Outcome::error("qgis_run needs algorithm");
        };
        let inputs = arguments
            .get("inputs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        match self.run_algorithm(id, &inputs) {
            Ok((results, folder)) => Outcome::json(
                &json!({"folder": self.workspace.relative(&folder), "results": results}),
            ),
            Err(why) => Outcome::error(why),
        }
    }

    fn convert(&self, arguments: &Map<String, Value>) -> Outcome {
        let (Some(file), Some(to)) = (
            arguments.get("file").and_then(Value::as_str),
            arguments.get("to").and_then(Value::as_str),
        ) else {
            return Outcome::error("qgis_convert needs file and to");
        };
        let raster = ["tif", "png", "jpg", "vrt", "asc"];
        let vector = [
            "gpkg", "shp", "geojson", "kml", "gml", "csv", "dxf", "fgb", "gpx",
        ];
        let stem = crate::delivery::to_name(
            Path::new(file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("layer"),
        );
        let (id, input) = if raster.contains(&to) {
            ("gdal:translate", "INPUT")
        } else if vector.contains(&to) {
            ("native:savefeatures", "INPUT")
        } else {
            return Outcome::error(format!(
                "to is a raster format {raster:?} or a vector one {vector:?}"
            ));
        };
        let mut inputs = Map::new();
        inputs.insert(input.into(), json!(file));
        inputs.insert("OUTPUT".into(), json!(format!("{stem}.{to}")));
        match self.run_algorithm(id, &inputs) {
            Ok((results, folder)) => Outcome::json(
                &json!({"folder": self.workspace.relative(&folder), "results": results}),
            ),
            Err(why) => Outcome::error(why),
        }
    }

    fn map(&self, arguments: &Map<String, Value>) -> Outcome {
        let layers: Vec<String> = match arguments.get("layers").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => match list
                .iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect()
            {
                Some(list) => list,
                None => return Outcome::error("layers are paths"),
            },
            _ => return Outcome::error("qgis_map needs layers, drawn first to last"),
        };
        let Some(extent) = arguments.get("extent").and_then(Value::as_str) else {
            return Outcome::error(
                "extent is \"xmin,xmax,ymin,ymax [EPSG:code]\"; qgis_info gives a layer's",
            );
        };
        if !extent
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" ,.-:[]".contains(&b))
        {
            return Outcome::error("extent is numbers, commas and an [EPSG:code]");
        }
        let numbers: Vec<f64> = extent
            .split('[')
            .next()
            .unwrap_or("")
            .split(',')
            .filter_map(|n| n.trim().parse().ok())
            .collect();
        if numbers.len() != 4 || numbers[1] <= numbers[0] || numbers[3] <= numbers[2] {
            return Outcome::error("extent is xmin,xmax,ymin,ymax with max above min");
        }
        let width = arguments
            .get("width")
            .and_then(Value::as_u64)
            .unwrap_or(1600)
            .clamp(200, 4000) as u32;
        let aspect = (numbers[3] - numbers[2]) / (numbers[1] - numbers[0]);
        let height = ((width as f64 * aspect).round() as u32).clamp(100, 4000);
        let crs: Option<String> = extent
            .split_once('[')
            .map(|(_, rest)| rest.trim_end_matches(']').trim().to_string())
            .filter(|c| !c.is_empty());
        let folder = match self.workspace.fresh_folder("maps", "map") {
            Ok(folder) => folder,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let scratch = folder.join(".layers");
        if let Err(e) = std::fs::create_dir_all(&scratch) {
            return Outcome::error(e.to_string());
        }
        let limit = Duration::from_secs(600);
        let te: Vec<String> = [numbers[0], numbers[2], numbers[1], numbers[3]]
            .iter()
            .map(|n| n.to_string())
            .collect();
        let mut painted = Vec::new();
        for (i, name) in layers.iter().enumerate() {
            let source = match self.layer(name) {
                Ok(source) => source,
                Err(why) => return Outcome::error(why),
            };
            let raster = Run::new(self.programs.get("gdalinfo"))
                .args(["-json", "-nomd", "-norat", "-noct"])
                .arg(&source)
                .limit(limit)
                .run()
                .is_ok_and(|r| r.success());
            if raster {
                // Warped to the map's CRS, extent and size, then made 8-bit for painting.
                let warped = scratch.join(format!("{i}.tif"));
                let mut warp = Run::new(self.programs.get("gdalwarp"))
                    .args(["-q", "-overwrite", "-r", "bilinear", "-dstalpha", "-te"])
                    .args(&te)
                    .args(["-ts", &width.to_string(), &height.to_string()]);
                if let Some(crs) = &crs {
                    warp = warp.args(["-t_srs", crs]);
                }
                let png = scratch.join(format!("{i}.png"));
                let ok = warp
                    .arg(&source)
                    .arg(&warped)
                    .limit(limit)
                    .run()
                    .is_ok_and(|r| r.success())
                    && Run::new(self.programs.get("gdal_translate"))
                        .args(["-q", "-of", "PNG", "-ot", "Byte", "-scale"])
                        .arg(&warped)
                        .arg(&png)
                        .limit(limit)
                        .run()
                        .is_ok_and(|r| r.success());
                let pixels = ok
                    .then(|| std::fs::read(&png).ok())
                    .flatten()
                    .and_then(|bytes| map::rgba(&bytes).ok());
                match pixels {
                    Some((rgba, w, h)) if w == width && h == height => {
                        painted.push(map::Layer::Raster(rgba))
                    }
                    _ => {
                        return Outcome::error(format!("{name}: GDAL could not warp it to the map"))
                    }
                }
            } else {
                // Vectors come as GeoJSON in the map's CRS, clipped to the extent.
                let json = scratch.join(format!("{i}.geojson"));
                let mut convert = Run::new(self.programs.get("ogr2ogr"))
                    .args(["-f", "GeoJSON", "-clipdst"])
                    .args(&te);
                if let Some(crs) = &crs {
                    convert = convert.args(["-t_srs", crs]);
                }
                let ran = convert
                    .arg(&json)
                    .arg(source.split('|').next().unwrap_or(&source))
                    .limit(limit)
                    .run();
                if !ran.as_ref().is_ok_and(|r| r.success()) {
                    return Outcome::error(format!("{name}: neither GDAL nor OGR reads it"));
                }
                let size = std::fs::metadata(&json).map(|m| m.len()).unwrap_or(0);
                if size > 256 * 1024 * 1024 {
                    return Outcome::error(format!(
                        "{name} has more inside the extent than a map picture draws; zoom in"
                    ));
                }
                match std::fs::read_to_string(&json)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                {
                    Some(collection) => painted.push(map::Layer::Vector(collection)),
                    None => {
                        return Outcome::error(format!(
                            "{name}: the reprojected layer does not read"
                        ))
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&scratch);
        let bytes = match map::paint(
            &painted,
            [numbers[0], numbers[1], numbers[2], numbers[3]],
            width,
            height,
        ) {
            Ok(bytes) => bytes,
            Err(why) => return Outcome::error(why),
        };
        let png = folder.join("map.png");
        if let Err(e) = std::fs::write(&png, &bytes) {
            return Outcome::error(e.to_string());
        }
        let outcome = Outcome::json(
            &json!({"image": self.workspace.relative(&png), "size": [width, height], "crs": crs}),
        );
        if bytes.len() <= MAX_IMAGE {
            outcome.with_image(Image {
                mime: "image/png",
                data: bytes,
            })
        } else {
            outcome
        }
    }
}

/// A parameter's type as `qgis_process` describes it: `{"type": {"id": "sink", …}}` in
/// current versions, a plain string in older ones.
fn kind(parameter: &Value) -> &str {
    parameter["type"]["id"]
        .as_str()
        .or_else(|| parameter["type"].as_str())
        .or_else(|| parameter["raw_definition"]["parameter_type"].as_str())
        .unwrap_or("")
}

/// Whether a parameter is somewhere the algorithm writes. Fails closed: QGIS's own flag
/// or any destination type counts.
fn is_output(parameter: &Value) -> bool {
    parameter["is_destination"] == json!(true) || OUTPUT_TYPES.contains(&kind(parameter))
}

/// Text that names somewhere outside the workspace, in a parameter that is not a layer.
fn looks_like_a_path(text: &str) -> bool {
    let body = text.strip_prefix("expression:").unwrap_or(text);
    text == body
        && (text.starts_with('/')
            || text.starts_with('~')
            || text.contains(":\\")
            || text.contains(":/"))
        || body.contains("..")
}

/// A CRS's name from its WKT: the first quoted string.
fn crs_name(wkt: &str) -> Option<String> {
    let start = wkt.find('"')? + 1;
    let end = wkt[start..].find('"')? + start;
    let mut name = wkt[start..end].to_string();
    // The authority code, when the WKT ends with one: ID["EPSG",4326].
    if let Some(at) = wkt.rfind("ID[\"EPSG\",") {
        let code: String = wkt[at + 10..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !code.is_empty() {
            name = format!("{name} (EPSG:{code})");
        }
    }
    Some(name)
}

impl Bridge for Qgis {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    fn guide(&self) -> &'static str {
        GUIDE
    }

    fn tools(&self) -> Vec<BridgeTool> {
        let file = json!({"type": "string", "description": "A layer in the workspace."});
        vec![
            BridgeTool {
                name: "qgis_layers",
                title: "Geographic data",
                description: "The vector, raster and point-cloud files in the workspace.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "qgis_info",
                title: "What a layer holds",
                description: "A vector file's layers (features, geometry type, CRS, extent, fields) or a raster's \
                              size, bands, CRS and extent, read by GDAL.",
                input_schema: json!({"type": "object", "properties": {"file": file}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "qgis_algorithms",
                title: "Find an algorithm",
                description: "QGIS's processing algorithms (native, GDAL, PDAL, 3D) whose id, name or group contain \
                              every word of search, such as \"buffer\", \"clip raster\", \"reproject\".",
                input_schema: json!({"type": "object", "properties": {"search": {"type": "string"}}, "additionalProperties": false}),
                grade: PrivacyGrade::Public,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "qgis_algorithm_help",
                title: "An algorithm's parameters",
                description: "One algorithm's parameters: name, type, description, whether optional, default and \
                              options. Read it before qgis_run.",
                input_schema: json!({"type": "object", "properties": {"algorithm": {"type": "string"}}, "required": ["algorithm"], "additionalProperties": false}),
                grade: PrivacyGrade::Public,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "qgis_run",
                title: "Run an algorithm",
                description: "Runs a processing algorithm. Layer parameters are paths in the workspace (a table of a \
                              file as roads.gpkg|layername=roads); output parameters are plain file names, written \
                              to exports/<algorithm>-<n>/. Returns the algorithm's results.",
                input_schema: json!({
                    "type": "object",
                    "properties": {"algorithm": {"type": "string"}, "inputs": {"type": "object"}},
                    "required": ["algorithm", "inputs"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "qgis_convert",
                title: "Convert a layer",
                description: "Converts a vector layer to gpkg, shp (ArcGIS), geojson, kml (Google Earth), gml, csv, \
                              dxf (CAD), fgb or gpx, or a raster to tif, png, jpg, vrt or asc, into exports/.",
                input_schema: json!({"type": "object", "properties": {"file": file, "to": {"type": "string"}}, "required": ["file", "to"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "qgis_map",
                title: "Draw a map",
                description: "Draws layers over an extent \"xmin,xmax,ymin,ymax [EPSG:code]\": rasters warped \
                              underneath, then each vector layer in its own colour with translucent fills and \
                              ringed points, first at the bottom. Writes maps/map-<n>/map.png and returns it.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "layers": {"type": "array", "items": {"type": "string"}},
                        "extent": {"type": "string"},
                        "width": {"type": "integer", "minimum": 200, "maximum": 6000}
                    },
                    "required": ["layers", "extent"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "qgis_layers" => {
                let (files, truncated) = self.workspace.find(&READS, 5, 2000);
                Outcome::json(&json!({"files": files, "truncated": truncated}))
            }
            "qgis_info" => self.info(arguments),
            "qgis_algorithms" => self.algorithms(arguments),
            "qgis_algorithm_help" => self.algorithm_help(arguments),
            "qgis_run" => self.run_tool(arguments),
            "qgis_convert" => self.convert(arguments),
            "qgis_map" => self.map(arguments),
            _ => Outcome::error("unknown tool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_algorithms_from_known_providers_run() {
        assert!(algorithm_allowed("native:buffer").is_ok());
        assert!(algorithm_allowed("gdal:warpreproject").is_ok());
        for bad in [
            "script:mine",
            "native:filedownloader",
            "gdal:executesql",
            "native:buf fer",
            "buffer",
            "qgis:importintopostgis",
        ] {
            assert!(algorithm_allowed(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parameter_kinds_are_read_as_qgis_process_writes_them() {
        // qgis_process 3.34's help --json: the type is an object with an id.
        let sink = json!({"is_destination": true, "type": {"id": "sink", "name": "Feature Sink"}});
        let source = json!({"is_destination": false, "type": {"id": "source"}});
        let old = json!({"type": "rasterDestination"});
        let flagged = json!({"is_destination": true, "type": {"id": "something_new"}});
        assert_eq!(kind(&sink), "sink");
        assert_eq!(kind(&source), "source");
        assert!(is_output(&sink) && is_output(&old) && is_output(&flagged));
        assert!(!is_output(&source));
        for path in [
            "/etc/passwd",
            "../x.gpkg",
            "C:\\x",
            "http://evil.example/x",
            "~/x",
        ] {
            assert!(looks_like_a_path(path), "{path}");
        }
        for plain in ["expression:\"area\"/2", "EPSG:4326", "10", "roads"] {
            assert!(!looks_like_a_path(plain), "{plain}");
        }
    }

    #[test]
    fn crs_names_come_from_their_wkt() {
        let wkt = r#"GEOGCRS["WGS 84",DATUM["World Geodetic System 1984"],ID["EPSG",4326]]"#;
        assert_eq!(crs_name(wkt).unwrap(), "WGS 84 (EPSG:4326)");
    }

    #[test]
    fn layer_paths_stay_inside_and_may_name_a_table() {
        let dir = std::env::temp_dir().join(format!("archie-qgis-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("roads.gpkg"), b"x").unwrap();
        let qgis = Qgis::new(
            Manifest::parse(MANIFEST).unwrap(),
            Workspace::open(&dir).unwrap(),
            &Programs::default(),
        );
        assert!(qgis
            .layer("roads.gpkg|layername=roads")
            .unwrap()
            .ends_with("roads.gpkg|layername=roads"));
        assert!(qgis.layer("../etc/passwd").is_err());
        assert!(qgis.layer("roads.gpkg|layername=x;rm").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
