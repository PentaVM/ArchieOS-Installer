//! What a project looks like on the desk: `stages.json`, the record, and `index.html`, a
//! page that shows every stage newest first with its previews, and reloads itself while
//! Archie keeps delivering.

use serde::{Deserialize, Serialize};
use std::fmt::Write;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub name: String,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Stage {
    pub name: String,
    /// When its first file arrived, milliseconds since 1970.
    pub started_ms: u64,
    /// When its last file arrived.
    pub updated_ms: u64,
    #[serde(default)]
    pub note: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

impl Project {
    /// Records a file that arrived in `stage`.
    pub fn record(&mut self, stage: &str, note: &str, file: FileEntry, now_ms: u64) {
        let entry = match self.stages.iter_mut().position(|s| s.name == stage) {
            Some(i) => &mut self.stages[i],
            None => {
                self.stages.push(Stage {
                    name: stage.to_string(),
                    started_ms: now_ms,
                    updated_ms: now_ms,
                    note: String::new(),
                    files: Vec::new(),
                });
                self.stages.last_mut().expect("just pushed")
            }
        };
        entry.updated_ms = now_ms;
        if !note.is_empty() {
            entry.note = note.to_string();
        }
        entry.files.retain(|f| f.name != file.name);
        entry.files.push(file);
        entry.files.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn is_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// What a person opens each kind of file with, shown beside it.
fn opens_in(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "step" | "stp" | "iges" | "igs" => "Fusion 360, SolidWorks, Onshape, COMSOL, FreeCAD",
        "stl" | "3mf" | "obj" => "slicers, Fusion 360, Blender",
        "fcstd" => "FreeCAD",
        "f3d" => "Fusion 360",
        "dxf" | "dwg" => "AutoCAD, Fusion 360, LibreCAD",
        "gbr" | "gtl" | "gbl" | "gto" | "gbo" | "gts" | "gbs" | "gm1" | "drl" => {
            "the board house, any Gerber viewer"
        }
        "zip" => "archive",
        "kicad_pcb" | "kicad_sch" | "kicad_pro" => "KiCad, Altium (import)",
        "psd" => "Photoshop, GIMP, Affinity",
        "xcf" => "GIMP",
        "svg" | "eps" | "ai" => "Illustrator, Inkscape, Affinity",
        "pdf" => "any PDF reader, Illustrator, Acrobat",
        "docx" | "odt" | "rtf" => "Word, LibreOffice, Google Docs",
        "xlsx" | "ods" | "csv" => "Excel, LibreOffice, Google Sheets",
        "pptx" | "odp" => "PowerPoint, LibreOffice, Keynote",
        "fbx" | "glb" | "gltf" | "usd" | "usdz" | "usdc" | "blend" | "ply" => {
            "Blender, Maya, 3ds Max, Unreal, Unity"
        }
        "gpkg" | "shp" | "geojson" | "kml" | "tif" | "tiff" => "QGIS, ArcGIS Pro, Google Earth",
        "mp4" | "mov" | "mkv" | "webm" => "any player, Premiere, DaVinci Resolve",
        "mlt" | "kdenlive" => "Kdenlive, Shotcut",
        "edl" | "fcpxml" | "otio" => "Premiere, DaVinci Resolve, Final Cut",
        "wav" | "flac" | "mp3" | "m4a" | "ogg" => "any player, Audition, Pro Tools, Audacity",
        "png" | "jpg" | "jpeg" | "webp" | "gif" => "any image viewer",
        _ => "",
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} kB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

const STYLE: &str = r#"
:root { --bg:#f6f6f3; --card:#ffffff; --ink:#1d1d1b; --muted:#6b6b66; --line:#e3e2dc; --accent:#2f6fde; }
@media (prefers-color-scheme: dark) {
  :root { --bg:#141414; --card:#1d1d1d; --ink:#ecece8; --muted:#9a9a94; --line:#2c2c2a; --accent:#7aa7ff; }
}
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--ink);
  font:15px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; }
header { padding:28px 32px 8px; }
h1 { margin:0; font-size:26px; letter-spacing:-0.01em; }
.sub { color:var(--muted); margin-top:4px; }
main { padding:8px 32px 48px; display:grid; gap:18px; }
.stage { background:var(--card); border:1px solid var(--line); border-radius:14px; padding:18px 20px; }
.stage.latest { border-color:var(--accent); }
.head { display:flex; gap:12px; align-items:baseline; flex-wrap:wrap; }
.head h2 { margin:0; font-size:18px; }
.badge { font-size:12px; color:var(--accent); border:1px solid var(--accent); border-radius:99px; padding:1px 8px; }
.when { color:var(--muted); font-size:13px; margin-left:auto; }
.note { margin:8px 0 0; }
.shots { display:grid; grid-template-columns:repeat(auto-fill, minmax(220px, 1fr)); gap:12px; margin-top:14px; }
.shots a { display:block; border-radius:10px; overflow:hidden; border:1px solid var(--line); background:var(--bg); }
.shots img { display:block; width:100%; height:180px; object-fit:contain; }
ul.files { list-style:none; margin:14px 0 0; padding:0; display:grid; gap:4px; }
ul.files li { display:flex; gap:10px; align-items:baseline; flex-wrap:wrap; }
ul.files a { color:var(--accent); text-decoration:none; font-weight:500; }
.meta { color:var(--muted); font-size:13px; }
.empty { color:var(--muted); padding:40px 0; }
@media (max-width:600px) { header, main { padding-left:16px; padding-right:16px; } }
"#;

/// A time as the page shows it, in UTC: the desk has no business guessing zones.
fn when(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3600,
        (rest % 3600) / 60
    )
}

/// The project's page. Paths in it are relative to the project folder.
pub fn project_page(project: &Project) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"refresh\" content=\"20\">\
         <title>{name} · Archie</title><style>{STYLE}</style></head><body>\
         <header><h1>{name}</h1><div class=\"sub\">{count} stage{s} delivered by Archie · \
         the newest files of each name are also in <a href=\"latest/\">latest/</a></div></header><main>",
        name = escape(&project.name),
        count = project.stages.len(),
        s = if project.stages.len() == 1 { "" } else { "s" },
    );
    if project.stages.is_empty() {
        out.push_str("<p class=\"empty\">Nothing yet.</p>");
    }
    let mut stages: Vec<&Stage> = project.stages.iter().collect();
    stages.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
    for (i, stage) in stages.iter().enumerate() {
        let folder = format!("stages/{}", stage.name);
        let _ = write!(
            out,
            "<section class=\"stage{latest}\"><div class=\"head\"><h2>{name}</h2>{badge}\
             <span class=\"when\">{when}</span></div>",
            latest = if i == 0 { " latest" } else { "" },
            name = escape(&stage.name),
            badge = if i == 0 {
                "<span class=\"badge\">latest</span>"
            } else {
                ""
            },
            when = when(stage.updated_ms),
        );
        if !stage.note.is_empty() {
            let _ = write!(out, "<p class=\"note\">{}</p>", escape(&stage.note));
        }
        let images: Vec<&FileEntry> = stage.files.iter().filter(|f| is_image(&f.name)).collect();
        if !images.is_empty() {
            out.push_str("<div class=\"shots\">");
            for image in images {
                let href = format!("{folder}/{}", image.name);
                let _ = write!(
                    out,
                    "<a href=\"{href}\"><img src=\"{href}\" alt=\"{alt}\" loading=\"lazy\"></a>",
                    href = escape(&href),
                    alt = escape(&image.name)
                );
            }
            out.push_str("</div>");
        }
        out.push_str("<ul class=\"files\">");
        for file in &stage.files {
            let opens = opens_in(&file.name);
            let _ = write!(
                out,
                "<li><a href=\"{href}\">{name}</a><span class=\"meta\">{size}{sep}{opens}</span></li>",
                href = escape(&format!("{folder}/{}", file.name)),
                name = escape(&file.name),
                size = size(file.bytes),
                sep = if opens.is_empty() { "" } else { " · opens in " },
                opens = escape(opens),
            );
        }
        out.push_str("</ul></section>");
    }
    out.push_str("</main></body></html>\n");
    out
}

/// The desk's front page: every project, most recently delivered first.
pub fn home_page(projects: &[Project]) -> String {
    let mut out = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"refresh\" content=\"30\">\
         <title>Archie desk</title><style>{STYLE}</style></head><body>\
         <header><h1>Archie desk</h1><div class=\"sub\">What Archie delivered to this computer</div></header><main>"
    );
    if projects.is_empty() {
        out.push_str("<p class=\"empty\">Nothing yet. Deliveries from Archie land here.</p>");
    }
    let mut sorted: Vec<&Project> = projects.iter().collect();
    let last = |p: &Project| p.stages.iter().map(|s| s.updated_ms).max().unwrap_or(0);
    sorted.sort_by_key(|p| std::cmp::Reverse(last(p)));
    for project in sorted {
        let newest = project.stages.iter().max_by_key(|s| s.updated_ms);
        let _ = write!(
            out,
            "<section class=\"stage\"><div class=\"head\"><h2><a href=\"{href}/index.html\">{name}</a></h2>\
             <span class=\"when\">{when}</span></div><p class=\"note\">{count} stages{latest}</p></section>",
            href = escape(&project.name),
            name = escape(&project.name),
            when = when(last(project)),
            count = project.stages.len(),
            latest = newest
                .map(|s| format!(" · latest: {}", escape(&s.name)))
                .unwrap_or_default(),
        );
    }
    out.push_str("</main></body></html>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> FileEntry {
        FileEntry {
            name: name.into(),
            bytes: 2048,
            sha256: "00".repeat(32),
        }
    }

    #[test]
    fn stages_collect_their_files_and_the_page_shows_them() {
        let mut project = Project {
            name: "bracket".into(),
            stages: Vec::new(),
        };
        project.record("iter-0001", "first try", file("bracket.step"), 1_000);
        project.record("iter-0001", "", file("bracket-iso.png"), 2_000);
        project.record("iter-0002", "thinner <web>", file("bracket.step"), 3_000);
        project.record("iter-0002", "", file("bracket.step"), 4_000);
        assert_eq!(project.stages.len(), 2);
        assert_eq!(project.stages[0].files.len(), 2);
        assert_eq!(
            project.stages[0].note, "first try",
            "an empty note keeps the old one"
        );
        assert_eq!(
            project.stages[1].files.len(),
            1,
            "a file sent again replaces its entry"
        );

        let page = project_page(&project);
        let newest = page.find("iter-0002").unwrap();
        assert!(newest < page.find("iter-0001").unwrap(), "newest first");
        assert!(page.contains("thinner &lt;web&gt;"), "notes are escaped");
        assert!(page.contains("<img src=\"stages/iter-0001/bracket-iso.png\""));
        assert!(page.contains("Fusion 360"));
    }

    #[test]
    fn times_are_civil_dates() {
        assert_eq!(when(0), "1970-01-01 00:00 UTC");
        assert_eq!(when(1_790_463_840_000), "2026-09-26 23:04 UTC");
    }
}
