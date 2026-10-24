//! The content sphere: a folder read the way Xavi's Lumenfield desktop drew the system.
//!
//! Xavi's desktop sphere showed the files of ArchieOS itself (`2499 ELEMENTOS · 3 GB`),
//! scanned once and laid out by `buildField` of `codigo fuente/src/field.js`, "regions"
//! layout. This module is that half, in Rust: a bounded, read-only listing of one folder
//! (no file is opened, nothing is written, links are not followed) and the exact field
//! layout, serialised as the scene JSON the native display already accepts from
//! `archie-displayd`, so the renderer draws it with every layer it already has.
//!
//! Bounds, as the desktop used them: at most `limit` nodes, at most `files_per_folder`
//! files listed per folder, folders before files, breadth first so the top of the tree
//! is always complete. Hidden entries and build output (`target`, `node_modules`, ...)
//! are skipped, as the desktop's `git archive` copy of the repository never had them.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

/// The design's seven types (field.js `CATEGORIES`), in its order.
pub const CATEGORIES: [(&str, &str, &str); 7] = [
    (
        "code",
        "Código",
        "c cc cpp cs css go h html java js jsx kt lua php py rb rs scss sh sql swift ts tsx vue",
    ),
    (
        "documents",
        "Documentos",
        "csv doc docx epub md odt pdf ppt pptx rtf txt xls xlsx",
    ),
    (
        "media",
        "Medios",
        "avi bmp gif jpeg jpg m4a mkv mov mp3 mp4 ogg png svg wav webm webp",
    ),
    ("data", "Datos", "db ini json toml xml yaml yml"),
    ("archive", "Comprimidos", "7z bz2 gz rar tar tgz zip"),
    (
        "executable",
        "Ejecutables",
        "bat cmd dll exe msi ps1 so app",
    ),
    ("other", "Otros", ""),
];
const KEYS: usize = CATEGORIES.len();
const DIMS: usize = KEYS + 4;
const AXES: [[f64; DIMS]; 3] = [
    [
        1.2, -0.85, 0.52, -0.52, 0.96, -1.06, 0.16, 0.16, 0.28, -0.2, 0.12,
    ],
    [
        -0.4, 0.88, -1.2, 0.46, 0.82, 0.2, -0.62, -0.17, 0.1, 0.26, 0.08,
    ],
    [
        0.8, 0.42, -0.24, -1.14, -0.15, 1.14, -0.58, 0.12, 0.11, -0.12, 0.31,
    ],
];
/// Never read more than this many entries of one folder, whatever it holds.
const MAX_ENTRIES_PER_FOLDER: usize = 20_000;
const SKIPPED: [&str; 7] = [
    "target",
    "node_modules",
    "__pycache__",
    "build",
    "dist-newstyle",
    "venv",
    "site-packages",
];
pub const MAX_LIMIT: usize = 6000;

/// The shape controls of the design's composition panel, as in `LumenShape`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentOptions {
    pub limit: usize,
    pub files_per_folder: usize,
    pub category_pull: f64,
    pub hierarchy_pull: f64,
    pub radial_layers: f64,
    pub size_influence: f64,
    pub scale: f64,
}

impl Default for ContentOptions {
    fn default() -> Self {
        // Xavi's desktop: 2 500 visible nodes and 30 files per folder (LiveOrb.qml).
        Self {
            limit: 2500,
            files_per_folder: 30,
            category_pull: 0.44,
            hierarchy_pull: 0.59,
            radial_layers: 0.63,
            size_influence: 0.09,
            scale: 1.0,
        }
    }
}

impl ContentOptions {
    fn clamped(self) -> Self {
        let unit =
            |v: f64, lo: f64, hi: f64, d: f64| if v.is_finite() { v.clamp(lo, hi) } else { d };
        let d = Self::default();
        Self {
            limit: self.limit.clamp(12, MAX_LIMIT),
            files_per_folder: self.files_per_folder.clamp(1, 200),
            category_pull: unit(self.category_pull, 0.0, 1.0, d.category_pull),
            hierarchy_pull: unit(self.hierarchy_pull, 0.0, 1.0, d.hierarchy_pull),
            radial_layers: unit(self.radial_layers, 0.0, 0.85, d.radial_layers),
            size_influence: unit(self.size_influence, 0.0, 0.4, d.size_influence),
            scale: unit(self.scale, 0.4, 2.0, d.scale),
        }
    }
}

/// One listed entry. `id` is its path relative to the root with `/`, the root is `.`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentNode {
    pub id: String,
    pub name: String,
    pub parent: Option<usize>,
    pub dir: bool,
    pub depth: u32,
    pub size: u64,
    pub category: usize,
    pub subtree: usize,
    pub position: [f64; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContentField {
    pub name: String,
    pub nodes: Vec<ContentNode>,
    pub radius: f64,
    /// Entries seen while listing, including files over the per-folder bound.
    pub elements: u64,
    pub bytes: u64,
    /// Whether the node limit stopped the listing before the whole tree was read.
    pub truncated: bool,
}

/// FNV-1a over UTF-16 code units, as `hash` in field.js (`charCodeAt`).
fn hash(text: &str) -> u32 {
    let mut h: u32 = 2_166_136_261;
    for unit in text.encode_utf16() {
        h ^= u32::from(unit);
        h = h.wrapping_mul(16_777_619);
    }
    h
}

fn unit(x: f64, y: f64, z: f64) -> [f64; 3] {
    let l = (x * x + y * y + z * z).sqrt();
    let l = if l == 0.0 { 1.0 } else { l };
    [x / l, y / l, z / l]
}

fn blend(a: [f64; 3], b: [f64; 3], w: f64) -> [f64; 3] {
    unit(
        a[0] * (1.0 - w) + b[0] * w,
        a[1] * (1.0 - w) + b[1] * w,
        a[2] * (1.0 - w) + b[2] * w,
    )
}

/// field.js `goldenDirection`; its region's share of the field is `fraction`.
fn golden(index: usize, fraction: f64) -> [f64; 3] {
    let z = 1.0 - 2.0 * fraction;
    let angle = index as f64 * 2.399_963_229_728_653;
    let r = (1.0 - z * z).max(0.0).sqrt();
    unit(angle.cos() * r, angle.sin() * r, z)
}

/// field.js `classify`: the extension after the last dot, lower case.
pub fn classify(name: &str) -> usize {
    let extension = name
        .rfind('.')
        .map(|i| name[i + 1..].to_lowercase())
        .unwrap_or_default();
    CATEGORIES
        .iter()
        .position(|(_, _, list)| !extension.is_empty() && list.split(' ').any(|e| e == extension))
        .unwrap_or(KEYS - 1)
}

fn size_term(size: u64) -> f64 {
    ((size as f64).ln_1p() / 24.0).min(1.0)
}

fn token(node: &ContentNode, category: usize, max_depth: u32) -> [f64; DIMS] {
    let mut v = [0.0; DIMS];
    v[category] = 1.0;
    v[KEYS] = size_term(node.size);
    v[KEYS + 1] = f64::from(node.depth) / f64::from(max_depth.max(1));
    v[KEYS + 2] = (f64::from(hash(&node.name.to_lowercase()) & 1023) / 1023.0 - 0.5) * 0.32;
    v[KEYS + 3] = if node.dir { 0.5 } else { 0.0 };
    v
}

fn skipped(name: &str) -> bool {
    name.starts_with('.') || SKIPPED.contains(&name)
}

/// Lists `root` breadth first within the bounds. Reads directory entries and their
/// metadata only; symbolic links are listed as files and never followed.
pub fn scan(root: &Path, options: ContentOptions) -> Option<ContentField> {
    let options = options.clamped();
    let meta = fs::metadata(root).ok()?;
    if !meta.is_dir() {
        return None;
    }
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let mut nodes = vec![ContentNode {
        id: ".".into(),
        name: name.clone(),
        parent: None,
        dir: true,
        depth: 0,
        size: 0,
        category: KEYS - 1,
        subtree: 1,
        position: [0.0; 3],
    }];
    let mut elements = 0_u64;
    let mut bytes = 0_u64;
    let mut truncated = false;
    let mut queue: VecDeque<(usize, PathBuf)> = VecDeque::from([(0, root.to_path_buf())]);
    while let Some((at, path)) = queue.pop_front() {
        let Ok(read) = fs::read_dir(&path) else {
            continue;
        };
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in read.take(MAX_ENTRIES_PER_FOLDER).flatten() {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if skipped(&file_name) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            elements += 1;
            if kind.is_dir() {
                dirs.push((file_name, entry.path()));
            } else {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                bytes = bytes.saturating_add(size);
                files.push((file_name, size));
            }
        }
        dirs.sort();
        files.sort();
        let depth = nodes[at].depth + 1;
        let prefix = if at == 0 {
            String::new()
        } else {
            format!("{}/", nodes[at].id)
        };
        for (file_name, child) in dirs {
            if nodes.len() >= options.limit {
                truncated = true;
                break;
            }
            nodes.push(ContentNode {
                id: format!("{prefix}{file_name}"),
                category: classify(&file_name),
                name: file_name,
                parent: Some(at),
                dir: true,
                depth,
                size: 0,
                subtree: 1,
                position: [0.0; 3],
            });
            queue.push_back((nodes.len() - 1, child));
        }
        if files.len() > options.files_per_folder {
            truncated = true;
        }
        for (file_name, size) in files.into_iter().take(options.files_per_folder) {
            if nodes.len() >= options.limit {
                truncated = true;
                break;
            }
            nodes.push(ContentNode {
                id: format!("{prefix}{file_name}"),
                category: classify(&file_name),
                name: file_name,
                parent: Some(at),
                dir: false,
                depth,
                size,
                subtree: 1,
                position: [0.0; 3],
            });
        }
    }
    if !queue.is_empty() {
        truncated = true;
    }
    let mut field = ContentField {
        name,
        nodes,
        radius: 0.0,
        elements,
        bytes,
        truncated,
    };
    layout(&mut field, options);
    Some(field)
}

/// field.js `buildField`, "regions" layout, without folder cohesion.
pub fn layout(field: &mut ContentField, options: ContentOptions) {
    let options = options.clamped();
    let nodes = &mut field.nodes;
    let n = nodes.len();
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut max_depth = 1;
    for (i, node) in nodes.iter().enumerate() {
        max_depth = max_depth.max(node.depth);
        if let Some(p) = node.parent {
            children.entry(p).or_default().push(i);
        }
    }
    for list in children.values_mut() {
        list.sort_by(|a, b| nodes[*a].id.cmp(&nodes[*b].id));
    }
    let mut signature: Vec<Option<[f64; DIMS]>> = vec![None; n];
    for (i, node) in nodes.iter().enumerate() {
        if !node.dir {
            signature[i] = Some(token(node, node.category, max_depth));
        }
    }
    let mut depth_order: Vec<usize> = (0..n).collect();
    depth_order.sort_by(|a, b| nodes[*b].depth.cmp(&nodes[*a].depth));
    for &i in &depth_order {
        let below: usize = children
            .get(&i)
            .map_or(0, |c| c.iter().map(|&k| nodes[k].subtree).sum());
        nodes[i].subtree = 1 + below;
    }
    for &i in &depth_order {
        if !nodes[i].dir {
            continue;
        }
        let mut aggregate = [0.0; DIMS];
        let mut total = 0.0;
        for &child in children.get(&i).map_or(&[][..], Vec::as_slice) {
            let Some(t) = signature[child] else { continue };
            let weight = if nodes[child].dir { 1.5 } else { 1.0 };
            total += weight;
            for d in 0..DIMS {
                aggregate[d] += t[d] * weight;
            }
        }
        if total > 0.0 {
            for value in &mut aggregate {
                *value /= total;
            }
        } else {
            aggregate = token(&nodes[i], KEYS - 1, max_depth);
        }
        aggregate[KEYS + 3] = 0.5;
        signature[i] = Some(aggregate);
        if total > 0.0 {
            let mut dominant = 0;
            for k in 1..KEYS {
                if aggregate[k] > aggregate[dominant] {
                    dominant = k;
                }
            }
            nodes[i].category = dominant;
        }
    }
    let mut centroid = [0.0; DIMS];
    let mut samples = 0.0;
    for (i, node) in nodes.iter().enumerate() {
        if node.dir {
            continue;
        }
        samples += 1.0;
        if let Some(v) = signature[i] {
            for d in 0..DIMS {
                centroid[d] += v[d];
            }
        }
    }
    if samples > 0.0 {
        for value in &mut centroid {
            *value /= samples;
        }
    }
    let total_items = (field.elements as f64).max(n as f64).max(1.0);
    let radius = (72.0 + total_items.cbrt() * 17.0).min(430.0) * options.scale;
    field.radius = radius;

    let mut top: Vec<usize> = children.get(&0).cloned().unwrap_or_default();
    top.sort_by(|a, b| {
        nodes[*b]
            .subtree
            .cmp(&nodes[*a].subtree)
            .then_with(|| nodes[*a].id.cmp(&nodes[*b].id))
    });
    let region_index: HashMap<usize, usize> =
        top.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let total_mass = top.iter().map(|&i| nodes[i].subtree).sum::<usize>().max(1) as f64;
    let mut region_fraction: HashMap<usize, f64> = HashMap::new();
    let mut allocated = 0.0;
    for &i in &top {
        let mass = nodes[i].subtree as f64;
        region_fraction.insert(i, (allocated + mass * 0.5) / total_mass);
        allocated += mass;
    }

    let signal_weight = 0.2 + options.category_pull * 0.55;
    let spread_weight = 0.42 + (1.0 - options.category_pull) * 0.9;
    let log_n = ((n.max(2)) as f64).ln_1p();
    let mut directions: Vec<[f64; 3]> = vec![[0.0, 0.0, 1.0]; n];
    let mut queue = VecDeque::from([0_usize]);
    while let Some(parent) = queue.pop_front() {
        let parent_direction = directions[parent];
        let siblings = children.get(&parent).cloned().unwrap_or_default();
        for (index, &i) in siblings.iter().enumerate() {
            let value =
                signature[i].unwrap_or_else(|| token(&nodes[i], nodes[i].category, max_depth));
            let mut semantic = [0.0; 3];
            for (axis, coefficients) in AXES.iter().enumerate() {
                semantic[axis] = (0..DIMS)
                    .map(|d| coefficients[d] * (value[d] - centroid[d]))
                    .sum();
            }
            let seed = hash(&nodes[i].id);
            let jitter = unit(
                f64::from(seed & 255) / 127.5 - 1.0,
                f64::from((seed >> 8) & 255) / 127.5 - 1.0,
                f64::from((seed >> 16) & 255) / 127.5 - 1.0,
            );
            let data = unit(
                semantic[0] * signal_weight + jitter[0] * spread_weight,
                semantic[1] * signal_weight + jitter[1] * spread_weight,
                semantic[2] * signal_weight + jitter[2] * spread_weight,
            );
            let direction = if parent == 0 {
                let k = region_index.get(&i).copied().unwrap_or(index);
                let fraction = region_fraction
                    .get(&i)
                    .copied()
                    .unwrap_or((index as f64 + 0.5) / top.len().max(1) as f64);
                blend(golden(k, fraction), data, options.category_pull * 0.26)
            } else {
                let mass = nodes[parent].subtree as f64;
                let region_width = (mass.ln_1p() / log_n).min(1.0);
                let mass_share = (mass / ((n.max(2) - 1) as f64)).min(1.0);
                let cohesion = (options.hierarchy_pull
                    * (1.0 - region_width * 0.45)
                    * (1.0 - mass_share.sqrt()))
                .max(0.12);
                blend(data, parent_direction, cohesion)
            };
            let node = &nodes[i];
            let depth_fraction = f64::from(node.depth) / f64::from(max_depth.max(1));
            let folder_term = if node.dir {
                (node.subtree as f64).ln_1p() / log_n * 0.1
            } else {
                0.0
            };
            let shell = (0.19
                + depth_fraction.sqrt() * options.radial_layers * 0.57
                + size_term(node.size) * options.size_influence * 0.2
                + (f64::from(seed >> 24) / 255.0 - 0.5) * 0.17
                - folder_term)
                .clamp(0.12, 0.9);
            nodes[i].position = [
                direction[0] * radius * shell,
                direction[1] * radius * shell,
                direction[2] * radius * shell,
            ];
            directions[i] = direction;
            queue.push_back(i);
        }
    }
    nodes[0].position = [0.0; 3];
}

fn escape(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.3}")
    } else {
        "0".into()
    }
}

/// The scene JSON `LumenField::accept` reads: the root, folders as modules and files as
/// entities, each with its anchor, plus what the desktop's label said about the corpus.
pub fn scene_json(field: &ContentField, generated_at_ms: i64) -> String {
    let mut out = String::with_capacity(field.nodes.len() * 220 + 1024);
    out.push_str(&format!("{{\"generated_at_ms\":{generated_at_ms},\"lens\":\"content\",\"root_id\":\".\",\"focus_id\":\".\",\"focus_path\":[],\"nodes\":["));
    for (i, node) in field.nodes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        escape(&mut out, &node.id);
        out.push_str(",\"name\":");
        escape(&mut out, &node.name);
        out.push_str(",\"parent\":");
        match node.parent {
            Some(p) => escape(&mut out, &field.nodes[p].id),
            None => out.push_str("null"),
        }
        let kind = if i == 0 {
            "root"
        } else if node.dir {
            "module"
        } else {
            "entity"
        };
        let size = if node.dir {
            0.0
        } else {
            ((node.size as f64).ln_1p() / 26.0).min(1.0)
        };
        out.push_str(&format!(
            ",\"kind\":\"{kind}\",\"category\":\"{}\",\"role\":\"\",\"position\":[{},{},{}],\"size\":{},\"depth\":{},\"member_count\":{},\"bytes\":{},\"expandable\":false,\"expanded\":false}}",
            CATEGORIES[node.category].0,
            number(node.position[0]),
            number(node.position[1]),
            number(node.position[2]),
            number(size),
            node.depth,
            node.subtree,
            node.size,
        ));
    }
    out.push_str("],\"links\":[],\"relations\":[],\"radius\":");
    out.push_str(&number(field.radius));
    out.push_str(",\"counts\":[");
    for (k, (key, label, _)) in CATEGORIES.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let count = field
            .nodes
            .iter()
            .filter(|n| !n.dir && n.category == k)
            .count();
        out.push_str(&format!("{{\"category\":\"{key}\",\"label\":"));
        escape(&mut out, label);
        out.push_str(&format!(",\"count\":{count}}}"));
    }
    let folders = field
        .nodes
        .iter()
        .filter(|n| n.dir)
        .count()
        .saturating_sub(1);
    out.push_str(&format!(
        "],\"source_count\":{},\"collapsed_count\":0,\"entity_count\":{},\"content\":{{\"name\":",
        field.nodes.len(),
        field.nodes.len()
    ));
    escape(&mut out, &field.name);
    out.push_str(&format!(
        ",\"elements\":{},\"bytes\":{},\"folders\":{folders},\"files\":{},\"truncated\":{}}}}}",
        field.elements.max(field.nodes.len() as u64 - 1),
        field.bytes,
        field.nodes.len() - 1 - folders,
        field.truncated
    ));
    out
}
