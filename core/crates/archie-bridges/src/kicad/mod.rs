//! KiCad: schematics and boards, through KiCad's own surfaces.
//!
//! * Its files (`.kicad_sch`, `.kicad_pcb`) are S-expressions this bridge reads directly
//!   ([`design`]) and edits one atom at a time ([`edit`]).
//! * `kicad-cli` runs KiCad's own checks and exports: ERC, DRC, netlists, gerbers, drill,
//!   placement, BOM, STEP, PDFs and 3D renders of the board.
//! * The IPC API (protobuf over NNG, KiCad 9 and later) edits an open board live. It is
//!   the next surface, listed as planned in the manifest.

pub mod design;
pub mod edit;

use crate::manifest::Manifest;
use crate::run::{Ran, Run};
use crate::sexpr::parse;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../../bridges/kicad/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../../bridges/kicad/SKILL.md");

const CHECK_LIMIT: Duration = Duration::from_secs(300);
const EXPORT_LIMIT: Duration = Duration::from_secs(900);
/// The most violations one check lists; the counts cover all of them.
const MAX_VIOLATIONS: usize = 80;
/// The most nets `kicad_netlist` lists.
const MAX_NETS: usize = 1500;
/// A render larger than this is returned as a path only.
const MAX_IMAGE: usize = 700 * 1024;
/// The largest design file read.
const MAX_FILE: u64 = 64 * 1024 * 1024;

const EXPORTS: [&str; 10] = [
    "gerbers",
    "drill",
    "positions",
    "step",
    "odb",
    "ipc2581",
    "bom",
    "netlist",
    "schematic_pdf",
    "schematic_svg",
];

pub struct Kicad {
    manifest: Manifest,
    workspace: Workspace,
    cli: PathBuf,
}

impl Kicad {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            cli: programs.get("kicad-cli"),
        }
    }

    fn read(&self, name: &str, extension: &str) -> Result<(PathBuf, String), String> {
        let path = self
            .workspace
            .file_with(name, extension)
            .map_err(|e| e.to_string())?;
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        if size > MAX_FILE {
            return Err(format!("{name} is larger than {MAX_FILE} bytes"));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))?;
        Ok((path, text))
    }

    fn cli(&self) -> Run {
        Run::new(&self.cli)
    }

    fn cli_failed(ran: &Ran, what: &str) -> Outcome {
        Outcome::error(format!(
            "kicad-cli {what} failed (exit {:?}):\n{}",
            ran.code,
            ran.said()
        ))
    }

    fn projects(&self) -> Outcome {
        let (files, truncated) =
            self.workspace
                .find(&["kicad_pro", "kicad_sch", "kicad_pcb"], 5, 2000);
        let mut projects: BTreeMap<String, Value> = BTreeMap::new();
        for file in &files {
            let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
            let entry = projects
                .entry(stem.to_string())
                .or_insert_with(|| json!({"project": null, "root_schematic": null, "board": null}));
            match file.rsplit_once('.').map(|(_, e)| e) {
                Some("kicad_pro") => entry["project"] = json!(file),
                Some("kicad_sch") => entry["root_schematic"] = json!(file),
                Some("kicad_pcb") => entry["board"] = json!(file),
                _ => {}
            }
        }
        // A project is a .kicad_pro; sheets without one are listed on their own.
        let (with_project, loose): (Vec<_>, Vec<_>) = projects
            .into_iter()
            .partition(|(_, entry)| !entry["project"].is_null());
        answer(json!({
            "projects": with_project.into_iter().map(|(_, e)| e).collect::<Vec<_>>(),
            "other_design_files": loose
                .into_iter()
                .flat_map(|(_, e)| [e["root_schematic"].clone(), e["board"].clone()])
                .filter(|v| !v.is_null())
                .collect::<Vec<_>>(),
            "truncated": truncated,
        }))
    }

    fn schematic(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(file) = string(arguments, "file") else {
            return Outcome::error("kicad_schematic needs file, a .kicad_sch in the workspace");
        };
        let (_, text) = match self.read(file, "kicad_sch") {
            Ok(read) => read,
            Err(message) => return Outcome::error(message),
        };
        match parse(&text) {
            Ok(root) if root.head() == Some("kicad_sch") => answer(json!(design::schematic(&root))),
            Ok(_) => Outcome::error(format!("{file} is not a KiCad schematic")),
            Err(error) => Outcome::error(format!("{file} does not parse: {error}")),
        }
    }

    fn board(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(file) = string(arguments, "file") else {
            return Outcome::error("kicad_board needs file, a .kicad_pcb in the workspace");
        };
        let (_, text) = match self.read(file, "kicad_pcb") {
            Ok(read) => read,
            Err(message) => return Outcome::error(message),
        };
        match parse(&text) {
            Ok(root) if root.head() == Some("kicad_pcb") => answer(json!(design::board(&root))),
            Ok(_) => Outcome::error(format!("{file} is not a KiCad board")),
            Err(error) => Outcome::error(format!("{file} does not parse: {error}")),
        }
    }

    fn set_fields(&self, arguments: &Map<String, Value>) -> Outcome {
        let (Some(file), Some(reference)) =
            (string(arguments, "file"), string(arguments, "reference"))
        else {
            return Outcome::error("kicad_set_fields needs file, reference and fields");
        };
        let fields: BTreeMap<String, String> = match arguments.get("fields") {
            Some(Value::Object(map)) => {
                let mut fields = BTreeMap::new();
                for (name, value) in map {
                    let Some(value) = value.as_str() else {
                        return Outcome::error(format!("field {name} is not a string"));
                    };
                    fields.insert(name.clone(), value.to_string());
                }
                fields
            }
            _ => return Outcome::error("fields is an object of field name to text"),
        };
        let (path, text) = match self.read(file, "kicad_sch") {
            Ok(read) => read,
            Err(message) => return Outcome::error(message),
        };
        if let Some(lock) = open_in_kicad(&path) {
            return Outcome::error(format!(
                "{file} is open in KiCad ({lock} exists). Saving from KiCad would undo this edit: \
                 close the schematic editor first, or make the change in the GUI on the Stage."
            ));
        }
        let (edited, changes, symbols) = match edit::set_fields(&text, reference, &fields) {
            Ok(done) => done,
            Err(error) => return Outcome::error(error.to_string()),
        };
        match self.workspace.replace(file, edited.as_bytes()) {
            Ok(previous) => answer(json!({
                "file": file,
                "reference": reference,
                "symbols_edited": symbols,
                "was": changes,
                "previous_version": previous,
                "next": "Run kicad_erc. A footprint changed here reaches the board only through \
                         KiCad's Update PCB from Schematic (F8 in the board editor).",
            })),
            Err(error) => Outcome::error(error.to_string()),
        }
    }

    fn netlist(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(file) = string(arguments, "file") else {
            return Outcome::error("kicad_netlist needs file, the root .kicad_sch");
        };
        let path = match self.workspace.file_with(file, "kicad_sch") {
            Ok(path) => path,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let out = match self
            .workspace
            .scratch("kicad")
            .map(|d| d.join("netlist.net"))
        {
            Ok(out) => out,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let _ = std::fs::remove_file(&out);
        let ran = match self
            .cli()
            .args([
                "sch",
                "export",
                "netlist",
                "--format",
                "kicadsexpr",
                "--output",
            ])
            .arg(&out)
            .arg(&path)
            .cwd(self.workspace.root())
            .limit(CHECK_LIMIT)
            .run()
        {
            Ok(ran) => ran,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let Ok(text) = std::fs::read_to_string(&out) else {
            return Self::cli_failed(&ran, "sch export netlist");
        };
        let root = match parse(&text) {
            Ok(root) => root,
            Err(error) => return Outcome::error(format!("the netlist does not parse: {error}")),
        };
        let mut nets = Vec::new();
        let mut truncated = false;
        if let Some(list) = root.child("nets") {
            for net in list.children("net") {
                if nets.len() == MAX_NETS {
                    truncated = true;
                    break;
                }
                let nodes: Vec<String> = net
                    .children("node")
                    .map(|n| {
                        format!(
                            "{}.{}",
                            n.value_of("ref").unwrap_or("?"),
                            n.value_of("pin").unwrap_or("?")
                        )
                    })
                    .collect();
                nets.push(json!({"name": net.value_of("name").unwrap_or(""), "nodes": nodes}));
            }
        }
        answer(json!({"nets": nets, "truncated": truncated}))
    }

    fn check(&self, arguments: &Map<String, Value>, board: bool) -> Outcome {
        let (tool, extension) = if board {
            ("kicad_drc", "kicad_pcb")
        } else {
            ("kicad_erc", "kicad_sch")
        };
        let Some(file) = string(arguments, "file") else {
            return Outcome::error(format!(
                "{tool} needs file, a .{extension} in the workspace"
            ));
        };
        let path = match self.workspace.file_with(file, extension) {
            Ok(path) => path,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let report = match self
            .workspace
            .scratch("kicad")
            .map(|d| d.join(format!("{tool}.json")))
        {
            Ok(report) => report,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let _ = std::fs::remove_file(&report);
        let mut run = self
            .cli()
            .args(if board {
                ["pcb", "drc"]
            } else {
                ["sch", "erc"]
            })
            .args([
                "--format",
                "json",
                "--units",
                "mm",
                "--severity-all",
                "--output",
            ])
            .arg(&report);
        if board && arguments.get("schematic_parity").and_then(Value::as_bool) == Some(true) {
            run = run.arg("--schematic-parity");
        }
        let ran = match run
            .arg(&path)
            .cwd(self.workspace.root())
            .limit(CHECK_LIMIT)
            .run()
        {
            Ok(ran) => ran,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let parsed = std::fs::read_to_string(&report)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        match parsed {
            Some(report) => answer(summarise(&report)),
            None => Self::cli_failed(&ran, if board { "pcb drc" } else { "sch erc" }),
        }
    }

    fn export(&self, arguments: &Map<String, Value>) -> Outcome {
        let kinds: Vec<&str> = match arguments.get("kinds") {
            Some(Value::Array(kinds)) if !kinds.is_empty() => {
                let mut out = Vec::new();
                for kind in kinds {
                    match kind.as_str().filter(|k| EXPORTS.contains(k)) {
                        Some(kind) => out.push(kind),
                        None => {
                            return Outcome::error(format!(
                                "kinds are some of {EXPORTS:?}; {kind} is not one"
                            ))
                        }
                    }
                }
                out.into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            }
            _ => return Outcome::error(format!("kicad_export needs kinds, some of {EXPORTS:?}")),
        };
        let board = match string(arguments, "board").map(|b| self.read(b, "kicad_pcb")) {
            Some(Ok((path, text))) => Some((path, text)),
            Some(Err(message)) => return Outcome::error(message),
            None => None,
        };
        let schematic = match string(arguments, "schematic")
            .map(|s| self.workspace.file_with(s, "kicad_sch"))
        {
            Some(Ok(path)) => Some(path),
            Some(Err(error)) => return Outcome::error(error.to_string()),
            None => None,
        };
        for kind in &kinds {
            let needs_board = matches!(
                *kind,
                "gerbers" | "drill" | "positions" | "step" | "odb" | "ipc2581"
            );
            if needs_board && board.is_none() {
                return Outcome::error(format!("{kind} is made from a board: pass board"));
            }
            if !needs_board && schematic.is_none() {
                return Outcome::error(format!("{kind} is made from a schematic: pass schematic"));
            }
        }
        let stem = board
            .as_ref()
            .map(|(p, _)| p.as_path())
            .or(schematic.as_deref())
            .and_then(Path::file_stem)
            .and_then(|s| s.to_str())
            .unwrap_or("design")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let folder = match self.workspace.fresh_folder("exports", &stem) {
            Ok(folder) => folder,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let mut made = BTreeMap::new();
        for kind in kinds {
            let dir = folder.join(kind);
            if let Err(error) = std::fs::create_dir_all(&dir) {
                return Outcome::error(format!("creating {kind}: {error}"));
            }
            let mut dir_arg = dir.clone().into_os_string();
            dir_arg.push(std::path::MAIN_SEPARATOR_STR);
            let (args, input): (Vec<std::ffi::OsString>, &Path) = match kind {
                "gerbers" => {
                    let (path, text) = board.as_ref().unwrap();
                    let layers = match plot_layers(text) {
                        Ok(layers) => layers,
                        Err(message) => return Outcome::error(message),
                    };
                    (
                        os(
                            [
                                "pcb",
                                "export",
                                "gerbers",
                                "--layers",
                                layers.as_str(),
                                "--output",
                            ],
                            &dir_arg,
                        ),
                        path.as_path(),
                    )
                }
                "drill" => (
                    os(
                        [
                            "pcb",
                            "export",
                            "drill",
                            "--format",
                            "excellon",
                            "--excellon-units",
                            "mm",
                            "--output",
                        ],
                        &dir_arg,
                    ),
                    board.as_ref().unwrap().0.as_path(),
                ),
                "positions" => (
                    os(
                        [
                            "pcb", "export", "pos", "--format", "csv", "--units", "mm", "--side",
                            "both", "--output",
                        ],
                        dir.join(format!("{stem}-positions.csv")).as_os_str(),
                    ),
                    board.as_ref().unwrap().0.as_path(),
                ),
                "step" => (
                    os(
                        ["pcb", "export", "step", "--output"],
                        dir.join(format!("{stem}.step")).as_os_str(),
                    ),
                    board.as_ref().unwrap().0.as_path(),
                ),
                // Whole-board manufacturing packages, which fabs and Altium, Cadence and
                // Mentor tools read in one file.
                "odb" => (
                    os(
                        ["pcb", "export", "odb", "--output"],
                        dir.join(format!("{stem}-odb.zip")).as_os_str(),
                    ),
                    board.as_ref().unwrap().0.as_path(),
                ),
                "ipc2581" => (
                    os(
                        ["pcb", "export", "ipc2581", "--units", "mm", "--output"],
                        dir.join(format!("{stem}.xml")).as_os_str(),
                    ),
                    board.as_ref().unwrap().0.as_path(),
                ),
                "bom" => (
                    os(
                        ["sch", "export", "bom", "--output"],
                        dir.join(format!("{stem}-bom.csv")).as_os_str(),
                    ),
                    schematic.as_deref().unwrap(),
                ),
                "netlist" => (
                    os(
                        [
                            "sch",
                            "export",
                            "netlist",
                            "--format",
                            "kicadsexpr",
                            "--output",
                        ],
                        dir.join(format!("{stem}.net")).as_os_str(),
                    ),
                    schematic.as_deref().unwrap(),
                ),
                "schematic_pdf" => (
                    os(
                        ["sch", "export", "pdf", "--output"],
                        dir.join(format!("{stem}.pdf")).as_os_str(),
                    ),
                    schematic.as_deref().unwrap(),
                ),
                _ => (
                    os(["sch", "export", "svg", "--output"], &dir_arg),
                    schematic.as_deref().unwrap(),
                ),
            };
            let ran = match self
                .cli()
                .args(args)
                .arg(input)
                .cwd(self.workspace.root())
                .limit(EXPORT_LIMIT)
                .run()
            {
                Ok(ran) => ran,
                Err(error) => return Outcome::error(error.to_string()),
            };
            let files = listing(&self.workspace, &dir);
            if !ran.success() || files.is_empty() {
                return Self::cli_failed(&ran, &format!("export {kind}"));
            }
            made.insert(kind, files);
        }
        answer(json!({"folder": self.workspace.relative(&folder), "files": made}))
    }

    fn render(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(file) = string(arguments, "file") else {
            return Outcome::error("kicad_render needs file, a .kicad_pcb in the workspace");
        };
        let path = match self.workspace.file_with(file, "kicad_pcb") {
            Ok(path) => path,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let side = string(arguments, "side").unwrap_or("top");
        if !["top", "bottom", "left", "right", "front", "back"].contains(&side) {
            return Outcome::error("side is top, bottom, left, right, front or back");
        }
        let dimension = |key: &str, default: u64| {
            arguments
                .get(key)
                .and_then(Value::as_u64)
                .unwrap_or(default)
                .clamp(200, 3000)
        };
        let (width, height) = (dimension("width", 1200), dimension("height", 800));
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("board");
        let out = match self
            .workspace
            .fresh_file("renders", &format!("{stem}-{side}"), "png")
        {
            Ok(out) => out,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let ran = match self
            .cli()
            .args([
                "pcb",
                "render",
                "--side",
                side,
                "--quality",
                "basic",
                "--background",
                "opaque",
            ])
            .args([
                "--width",
                width.to_string().as_str(),
                "--height",
                height.to_string().as_str(),
                "--output",
            ])
            .arg(&out)
            .arg(&path)
            .cwd(self.workspace.root())
            .limit(EXPORT_LIMIT)
            .run()
        {
            Ok(ran) => ran,
            Err(error) => return Outcome::error(error.to_string()),
        };
        let Ok(bytes) = std::fs::read(&out) else {
            return Self::cli_failed(&ran, "pcb render");
        };
        let outcome = answer(json!({
            "image": self.workspace.relative(&out),
            "side": side,
            "width": width,
            "height": height,
        }));
        if bytes.len() <= MAX_IMAGE && bytes.starts_with(b"\x89PNG") {
            outcome.with_image(Image {
                mime: "image/png",
                data: bytes,
            })
        } else {
            outcome
        }
    }

    fn version(&self) -> Outcome {
        match self
            .cli()
            .arg("version")
            .limit(Duration::from_secs(30))
            .run()
        {
            Ok(ran) if ran.success() => answer(json!({"kicad_cli": ran.stdout.trim()})),
            Ok(ran) => Self::cli_failed(&ran, "version"),
            Err(error) => Outcome::error(error.to_string()),
        }
    }
}

/// The layers a fabrication house wants, from what the board defines: every copper layer,
/// paste, silkscreen, mask and the outline.
fn plot_layers(board: &str) -> Result<String, String> {
    let root = parse(board).map_err(|e| format!("the board does not parse: {e}"))?;
    let board = design::board(&root);
    let wanted = [
        "F.Paste",
        "B.Paste",
        "F.SilkS",
        "B.SilkS",
        "F.Mask",
        "B.Mask",
        "Edge.Cuts",
    ];
    let mut layers = board.copper_layers;
    layers.extend(
        board
            .layers
            .into_iter()
            .filter(|l| wanted.contains(&l.as_str())),
    );
    if layers.is_empty() {
        return Err("the board defines no layers".into());
    }
    Ok(layers.join(","))
}

fn os<const N: usize>(args: [&str; N], last: &std::ffi::OsStr) -> Vec<std::ffi::OsString> {
    let mut out: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
    out.push(last.to_os_string());
    out
}

fn listing(workspace: &Workspace, dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| workspace.relative(&e.path()))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// KiCad marks a file it has open with `~<name>.lck` beside it.
fn open_in_kicad(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let lock = path.with_file_name(format!("~{name}.lck"));
    lock.exists().then(|| format!("~{name}.lck"))
}

fn string<'a>(arguments: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// An ERC or DRC report, which KiCad writes as JSON, cut down to what a planner acts on:
/// counts by severity and type, then the violations themselves with where they are.
pub fn summarise(report: &Value) -> Value {
    let mut all: Vec<(Option<&str>, &Value)> = Vec::new();
    if let Some(sheets) = report["sheets"].as_array() {
        for sheet in sheets {
            for violation in sheet["violations"].as_array().into_iter().flatten() {
                all.push((sheet["path"].as_str(), violation));
            }
        }
    }
    for group in ["violations", "unconnected_items", "schematic_parity"] {
        for violation in report[group].as_array().into_iter().flatten() {
            all.push((None, violation));
        }
    }
    let active: Vec<_> = all
        .into_iter()
        .filter(|(_, v)| v["excluded"].as_bool() != Some(true))
        .collect();
    let mut by_severity: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    for (_, violation) in &active {
        *by_severity
            .entry(violation["severity"].as_str().unwrap_or("?").to_string())
            .or_default() += 1;
        *by_type
            .entry(violation["type"].as_str().unwrap_or("?").to_string())
            .or_default() += 1;
    }
    let listed: Vec<Value> = active
        .iter()
        .take(MAX_VIOLATIONS)
        .map(|(sheet, violation)| {
            let items: Vec<Value> = violation["items"]
                .as_array()
                .into_iter()
                .flatten()
                .take(6)
                .map(|item| json!({"what": item["description"], "at": item["pos"]}))
                .collect();
            let mut entry = json!({
                "severity": violation["severity"],
                "type": violation["type"],
                "description": violation["description"],
                "items": items,
            });
            if let Some(sheet) = sheet {
                entry["sheet"] = json!(sheet);
            }
            entry
        })
        .collect();
    json!({
        "kicad_version": report["kicad_version"],
        "units": report["coordinate_units"],
        "total": active.len(),
        "by_severity": by_severity,
        "by_type": by_type,
        "violations": listed,
        "truncated": active.len() > MAX_VIOLATIONS,
    })
}

fn answer(value: Value) -> Outcome {
    Outcome::json(&value)
}

fn file_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "properties": {"file": {"type": "string", "description": description}},
        "required": ["file"],
        "additionalProperties": false
    })
}

impl Bridge for Kicad {
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
        let internal = PrivacyGrade::Internal;
        vec![
            BridgeTool {
                name: "kicad_projects",
                title: "KiCad projects",
                description: "The KiCad projects in the workspace — each .kicad_pro with its root schematic \
                              and board — and any design files outside a project. Start here.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "kicad_schematic",
                title: "What a schematic sheet holds",
                description: "Reads one .kicad_sch sheet: its parts (reference, value, footprint, library \
                              symbol, units, DNP, every other field such as MPN), power symbols, child \
                              sheets with their files, labels and wire count. Child sheets are read with \
                              their own call.",
                input_schema: file_schema("A .kicad_sch path inside the workspace."),
                grade: internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "kicad_board",
                title: "What a board holds",
                description: "Reads a .kicad_pcb: footprints with reference, value, side and position; \
                              copper and other layers; net, track, via and zone counts; the Edge.Cuts \
                              outline's bounding box in mm.",
                input_schema: file_schema("A .kicad_pcb path inside the workspace."),
                grade: internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "kicad_netlist",
                title: "The nets of a design",
                description: "KiCad's own netlist of a schematic hierarchy: every net by name with the \
                              pins on it as REF.PIN. Pass the root sheet.",
                input_schema: file_schema("The root .kicad_sch of the project."),
                grade: internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "kicad_erc",
                title: "Electrical rules check",
                description: "Runs KiCad's ERC on a schematic hierarchy. Returns counts by severity and \
                              type, then the violations with the pins they concern and where (mm). Pass \
                              the root sheet.",
                input_schema: file_schema("The root .kicad_sch of the project."),
                grade: internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "kicad_drc",
                title: "Design rules check",
                description: "Runs KiCad's DRC on a board. Returns counts by severity and type, the \
                              violations with where they are (mm), and unconnected items. \
                              schematic_parity also reports where the board and schematic disagree.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {"type": "string", "description": "A .kicad_pcb path inside the workspace."},
                        "schematic_parity": {"type": "boolean", "description": "Also compare with the schematic."}
                    },
                    "required": ["file"],
                    "additionalProperties": false
                }),
                grade: internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "kicad_set_fields",
                title: "Set a part's fields",
                description: "Sets fields of the part with this reference in one .kicad_sch sheet — Value, \
                              Footprint, Datasheet, MPN or any other; a field it lacks is added hidden. \
                              Every unit of the part changes. Only these values change in the file; the \
                              old file is kept under .archie/history. Refused while KiCad has the sheet \
                              open, and for Reference, which KiCad's annotation owns.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {"type": "string", "description": "The .kicad_sch sheet the part is placed in."},
                        "reference": {"type": "string", "description": "Such as R12 or U3."},
                        "fields": {"type": "object", "additionalProperties": {"type": "string"},
                                   "description": "Field name to its new text, e.g. {\"Value\": \"4k7\"}."}
                    },
                    "required": ["file", "reference", "fields"],
                    "additionalProperties": false
                }),
                grade: internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "kicad_export",
                title: "Export manufacturing and documentation files",
                description: "Runs KiCad's exporters into a new folder exports/<design>-<n>/, one \
                              subfolder per kind: gerbers (every copper layer, paste, silkscreen, mask, \
                              outline), drill (Excellon, mm), positions (CSV, mm, both sides), step \
                              (Fusion 360, SolidWorks), odb (ODB++) and ipc2581 (for fabs and Altium or \
                              Cadence), bom (CSV), netlist, schematic_pdf, schematic_svg. Board kinds need board; \
                              schematic kinds need schematic. Never replaces a file.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "board": {"type": "string", "description": "A .kicad_pcb in the workspace."},
                        "schematic": {"type": "string", "description": "The root .kicad_sch in the workspace."},
                        "kinds": {"type": "array", "items": {"type": "string", "enum": EXPORTS}}
                    },
                    "required": ["kinds"],
                    "additionalProperties": false
                }),
                grade: internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "kicad_render",
                title: "Render the board in 3D",
                description: "Renders a board with KiCad's 3D viewer into renders/<board>-<side>-<n>.png \
                              and returns the image: the way to see a board without opening KiCad, and \
                              to check work done in the GUI.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file": {"type": "string", "description": "A .kicad_pcb path inside the workspace."},
                        "side": {"type": "string", "enum": ["top", "bottom", "left", "right", "front", "back"]},
                        "width": {"type": "integer", "minimum": 200, "maximum": 3000},
                        "height": {"type": "integer", "minimum": 200, "maximum": 3000}
                    },
                    "required": ["file"],
                    "additionalProperties": false
                }),
                grade: internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "kicad_version",
                title: "KiCad's version",
                description: "The installed kicad-cli's version. A failure here means KiCad is not \
                              installed where the bridge looks; the guide says how to install it.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Public,
                effect: Effect::ReadOnly,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "kicad_projects" => self.projects(),
            "kicad_schematic" => self.schematic(arguments),
            "kicad_board" => self.board(arguments),
            "kicad_netlist" => self.netlist(arguments),
            "kicad_erc" => self.check(arguments, false),
            "kicad_drc" => self.check(arguments, true),
            "kicad_set_fields" => self.set_fields(arguments),
            "kicad_export" => self.export(arguments),
            "kicad_render" => self.render(arguments),
            "kicad_version" => self.version(),
            _ => Outcome::error("unknown tool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_counted_and_excluded_violations_are_left_out() {
        let report = json!({
            "kicad_version": "10.0.1",
            "coordinate_units": "mm",
            "sheets": [{"path": "/", "violations": [
                {"severity": "error", "type": "pin_not_connected", "description": "Pin not connected",
                 "items": [{"description": "Symbol U1 Pin 3", "pos": {"x": 10.0, "y": 20.0}}]},
                {"severity": "warning", "type": "lib_symbol_mismatch", "description": "x", "excluded": true, "items": []}
            ]}]
        });
        let summary = summarise(&report);
        assert_eq!(summary["total"], 1);
        assert_eq!(summary["by_type"]["pin_not_connected"], 1);
        assert_eq!(
            summary["violations"][0]["items"][0]["what"],
            "Symbol U1 Pin 3"
        );
        assert_eq!(summary["violations"][0]["sheet"], "/");
        let drc = json!({"violations": [{"severity": "error", "type": "clearance"}],
                         "unconnected_items": [{"severity": "error", "type": "unconnected_items"}]});
        assert_eq!(summarise(&drc)["total"], 2);
    }

    #[test]
    fn gerbers_plot_every_copper_layer_the_board_has() {
        let board = r#"(kicad_pcb (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal)
            (1 "F.Mask" user) (13 "F.Paste" user) (5 "F.SilkS" user "F.Silkscreen") (25 "Edge.Cuts" user)
            (35 "F.Fab" user)))"#;
        assert_eq!(
            plot_layers(board).unwrap(),
            "F.Cu,In1.Cu,B.Cu,F.Mask,F.Paste,F.SilkS,Edge.Cuts"
        );
    }

    fn workspace(name: &str) -> (Kicad, PathBuf) {
        let dir = std::env::temp_dir().join(format!("archie-kicad-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("blinky")).unwrap();
        std::fs::write(dir.join("blinky/blinky.kicad_pro"), "{}").unwrap();
        std::fs::write(
            dir.join("blinky/blinky.kicad_sch"),
            "(kicad_sch (version 20250114)\n\t(symbol (lib_id \"Device:R\") (at 1 2 0) (unit 1)\n\t\t(property \"Reference\" \"R1\" (at 0 0 0))\n\t\t(property \"Value\" \"10k\" (at 0 0 0))\n\t)\n)\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("loose.kicad_pcb"),
            "(kicad_pcb (version 20241229))",
        )
        .unwrap();
        let kicad = Kicad::new(
            Manifest::parse(MANIFEST).unwrap(),
            Workspace::open(&dir).unwrap(),
            &Programs::default(),
        );
        (kicad, dir)
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn projects_are_found_and_named_relative() {
        let (kicad, dir) = workspace("projects");
        let outcome = kicad.call("kicad_projects", &Map::new());
        let answer: Value = serde_json::from_str(&outcome.text).unwrap();
        assert_eq!(
            answer["projects"][0]["root_schematic"],
            "blinky/blinky.kicad_sch"
        );
        assert_eq!(answer["other_design_files"][0], "loose.kicad_pcb");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_field_edit_writes_the_sheet_and_keeps_the_old_one() {
        let (kicad, dir) = workspace("edit");
        let outcome = kicad.call(
            "kicad_set_fields",
            &args(json!({"file": "blinky/blinky.kicad_sch", "reference": "R1", "fields": {"Value": "4k7"}})),
        );
        assert!(!outcome.is_error, "{}", outcome.text);
        let now = std::fs::read_to_string(dir.join("blinky/blinky.kicad_sch")).unwrap();
        assert!(now.contains("\"4k7\""));
        let answer: Value = serde_json::from_str(&outcome.text).unwrap();
        assert_eq!(answer["was"]["Value"], "10k");
        let kept = answer["previous_version"].as_str().unwrap();
        assert!(std::fs::read_to_string(dir.join(kept))
            .unwrap()
            .contains("\"10k\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_sheet_open_in_kicad_is_not_edited() {
        let (kicad, dir) = workspace("locked");
        std::fs::write(dir.join("blinky/~blinky.kicad_sch.lck"), "{}").unwrap();
        let outcome = kicad.call(
            "kicad_set_fields",
            &args(json!({"file": "blinky/blinky.kicad_sch", "reference": "R1", "fields": {"Value": "1k"}})),
        );
        assert!(outcome.is_error);
        assert!(outcome.text.contains("open in KiCad"), "{}", outcome.text);
        let now = std::fs::read_to_string(dir.join("blinky/blinky.kicad_sch")).unwrap();
        assert!(now.contains("\"10k\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn exports_check_their_inputs_before_running_anything() {
        let (kicad, dir) = workspace("export");
        let wrong = kicad.call(
            "kicad_export",
            &args(json!({"kinds": ["gerbers"], "schematic": "blinky/blinky.kicad_sch"})),
        );
        assert!(wrong.text.contains("pass board"), "{}", wrong.text);
        let unknown = kicad.call("kicad_export", &args(json!({"kinds": ["odb"]})));
        assert!(unknown.is_error);
        let outside = kicad.call(
            "kicad_export",
            &args(json!({"kinds": ["bom"], "schematic": "../x.kicad_sch"})),
        );
        assert!(outside.is_error);
        assert!(!dir.join("exports").exists(), "nothing was made");
        let _ = std::fs::remove_dir_all(dir);
    }
}
