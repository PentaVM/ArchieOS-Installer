//! LibreOffice: documents, spreadsheets and presentations, written well and converted to
//! whatever the person's own office suite opens.
//!
//! New documents are described in JSON and written as flat OpenDocument ([`odf`]), then
//! converted by `soffice --headless`. Existing ones are converted, read as text and
//! pictured through the same program, with its own profile under `.archie/` so it never
//! touches the person's LibreOffice settings or collides with a copy they have open.

pub mod odf;

use crate::manifest::Manifest;
use crate::run::Run;
use crate::workspace::Workspace;
use crate::{Bridge, BridgeTool, Effect, Image, Outcome, Programs};
use archie_types::PrivacyGrade;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../../../../bridges/office/bridge.toml");
pub const GUIDE: &str = include_str!("../../../../../bridges/office/SKILL.md");

const LIMIT: Duration = Duration::from_secs(300);
const MAX_TEXT: usize = 200_000;
const MAX_IMAGE: usize = 700 * 1024;

const WRITER: [&str; 8] = ["odt", "fodt", "docx", "doc", "rtf", "txt", "html", "htm"];
const CALC: [&str; 6] = ["ods", "fods", "xlsx", "xls", "csv", "tsv"];
const IMPRESS: [&str; 5] = ["odp", "fodp", "pptx", "ppt", "key"];
const OTHER: [&str; 2] = ["pdf", "odg"];

/// What each kind of document may be converted to.
fn targets(extension: &str) -> &'static [&'static str] {
    if WRITER.contains(&extension) {
        &["pdf", "docx", "odt", "rtf", "txt", "html", "epub", "png"]
    } else if CALC.contains(&extension) {
        &["pdf", "xlsx", "ods", "csv", "html", "png"]
    } else if IMPRESS.contains(&extension) {
        &["pdf", "pptx", "odp", "png", "html"]
    } else {
        &["pdf", "png"]
    }
}

pub struct Office {
    manifest: Manifest,
    workspace: Workspace,
    soffice: PathBuf,
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

impl Office {
    pub fn new(manifest: Manifest, workspace: Workspace, programs: &Programs) -> Self {
        Self {
            manifest,
            workspace,
            soffice: programs.get("soffice"),
        }
    }

    fn document(&self, arguments: &Map<String, Value>) -> Result<(PathBuf, String), String> {
        let file = arguments
            .get("file")
            .and_then(Value::as_str)
            .ok_or("file is a document in the workspace")?;
        let path = self.workspace.file(file).map_err(|e| e.to_string())?;
        let ext = extension(&path);
        if ![&WRITER[..], &CALC[..], &IMPRESS[..], &OTHER[..]]
            .concat()
            .contains(&ext.as_str())
        {
            return Err(format!("{file} is not a document LibreOffice reads"));
        }
        Ok((path, ext))
    }

    /// `soffice --convert-to <filter>` of `input` into `out_dir`. Returns the files made.
    fn convert(&self, input: &Path, filter: &str, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
        std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
        let profile = self
            .workspace
            .scratch("office-profile")
            .map_err(|e| e.to_string())?;
        let profile_url = format!(
            "file:///{}",
            profile
                .display()
                .to_string()
                .replace('\\', "/")
                .trim_start_matches("//?/")
                .trim_start_matches('/')
        );
        let before: Vec<PathBuf> = listing(out_dir);
        let ran = Run::new(&self.soffice)
            .arg(format!("-env:UserInstallation={profile_url}"))
            .args([
                "--headless",
                "--norestore",
                "--nolockcheck",
                "--nodefault",
                "--convert-to",
                filter,
                "--outdir",
            ])
            .arg(out_dir)
            .arg(input)
            .cwd(self.workspace.root())
            .limit(LIMIT)
            .run()
            .map_err(|e| e.to_string())?;
        let made: Vec<PathBuf> = listing(out_dir)
            .into_iter()
            .filter(|p| !before.contains(p))
            .collect();
        if made.is_empty() {
            return Err(format!(
                "LibreOffice made nothing (exit {:?}): {}",
                ran.code,
                if ran.said().is_empty() {
                    "no message".into()
                } else {
                    ran.said()
                }
            ));
        }
        Ok(made)
    }

    fn picture_of(&self, made: &[PathBuf]) -> Option<Image> {
        let png = made.iter().find(|p| extension(p) == "png")?;
        let data = std::fs::read(png).ok()?;
        (data.len() <= MAX_IMAGE).then_some(Image {
            mime: "image/png",
            data,
        })
    }

    fn documents(&self) -> Outcome {
        let kinds = [&WRITER[..], &CALC[..], &IMPRESS[..], &OTHER[..]].concat();
        let (files, truncated) = self.workspace.find(&kinds, 5, 2000);
        Outcome::json(&json!({"files": files, "truncated": truncated}))
    }

    fn convert_tool(&self, arguments: &Map<String, Value>) -> Outcome {
        let (path, ext) = match self.document(arguments) {
            Ok(found) => found,
            Err(why) => return Outcome::error(why),
        };
        let Some(to) = arguments.get("to").and_then(Value::as_str) else {
            return Outcome::error(format!("to is one of {:?}", targets(&ext)));
        };
        if !targets(&ext).contains(&to) {
            return Outcome::error(format!(
                "a .{ext} converts to {:?}, not {to}",
                targets(&ext)
            ));
        }
        let stem = crate::delivery::to_name(
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("document"),
        );
        let folder = match self.workspace.fresh_folder("exports", &stem) {
            Ok(folder) => folder,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let filter = match (to, CALC.contains(&ext.as_str())) {
            // Every sheet, UTF-8, comma-separated, quoted text.
            ("csv", true) => {
                "csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,false,false,false,-1"
            }
            ("txt", _) => "txt:Text (encoded):UTF8",
            _ => to,
        };
        match self.convert(&path, filter, &folder) {
            Ok(made) => {
                let files: Vec<String> = made.iter().map(|p| self.workspace.relative(p)).collect();
                let outcome = Outcome::json(
                    &json!({"folder": self.workspace.relative(&folder), "files": files}),
                );
                match self.picture_of(&made) {
                    Some(image) => outcome.with_image(image),
                    None => outcome,
                }
            }
            Err(why) => Outcome::error(why),
        }
    }

    fn text(&self, arguments: &Map<String, Value>) -> Outcome {
        let (path, ext) = match self.document(arguments) {
            Ok(found) => found,
            Err(why) => return Outcome::error(why),
        };
        if OTHER.contains(&ext.as_str()) {
            return Outcome::error("LibreOffice does not read text out of PDFs well; convert the source document instead");
        }
        let scratch = match self.workspace.scratch("office-text") {
            Ok(dir) => dir,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let _ = std::fs::remove_dir_all(&scratch);
        let (filter, kind) = if CALC.contains(&ext.as_str()) {
            (
                "csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,false,false,false,-1",
                "sheets",
            )
        } else if IMPRESS.contains(&ext.as_str()) {
            ("fodp", "slides")
        } else {
            ("txt:Text (encoded):UTF8", "text")
        };
        let made = match self.convert(&path, filter, &scratch) {
            Ok(made) => made,
            Err(why) => return Outcome::error(why),
        };
        let mut answer = json!({"kind": kind});
        let mut total = 0usize;
        let mut cut = |text: String| {
            let room = MAX_TEXT.saturating_sub(total);
            total += text.len().min(room);
            if text.len() > room {
                let mut end = room;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                (text[..end].to_string(), true)
            } else {
                (text, false)
            }
        };
        let mut truncated = false;
        match kind {
            "sheets" => {
                let mut sheets = Map::new();
                let mut made = made;
                made.sort();
                for file in made {
                    let name = file
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("sheet")
                        .to_string();
                    let (text, t) = cut(std::fs::read_to_string(&file).unwrap_or_default());
                    truncated |= t;
                    sheets.insert(name, json!(text));
                }
                answer["sheets"] = Value::Object(sheets);
            }
            "slides" => {
                let xml = std::fs::read_to_string(&made[0]).unwrap_or_default();
                let slides: Vec<Value> = xml
                    .split("<draw:page ")
                    .skip(1)
                    .map(|page| {
                        let (text, t) = cut(slide_text(page));
                        truncated |= t;
                        json!(text)
                    })
                    .collect();
                answer["slides"] = json!(slides);
            }
            _ => {
                let (text, t) = cut(std::fs::read_to_string(&made[0]).unwrap_or_default());
                truncated = t;
                answer["text"] = json!(text);
            }
        }
        answer["truncated"] = json!(truncated);
        let _ = std::fs::remove_dir_all(&scratch);
        Outcome::json(&answer)
    }

    fn preview(&self, arguments: &Map<String, Value>) -> Outcome {
        let (path, _) = match self.document(arguments) {
            Ok(found) => found,
            Err(why) => return Outcome::error(why),
        };
        let stem = crate::delivery::to_name(
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("document"),
        );
        let folder = match self.workspace.fresh_folder("renders", &stem) {
            Ok(folder) => folder,
            Err(e) => return Outcome::error(e.to_string()),
        };
        match self.convert(&path, "png", &folder) {
            Ok(made) => {
                let files: Vec<String> = made.iter().map(|p| self.workspace.relative(p)).collect();
                let outcome = Outcome::json(&json!({"image": files.first(), "page": 1}));
                match self.picture_of(&made) {
                    Some(image) => outcome.with_image(image),
                    None => outcome,
                }
            }
            Err(why) => Outcome::error(why),
        }
    }

    /// Writes a flat ODF file into scratch and converts it to each requested format in a
    /// new folder `documents/<name>-<n>/`.
    fn write(&self, name: &str, flat: &str, flat_ext: &str, formats: &[String]) -> Outcome {
        let stem = crate::delivery::to_name(name);
        let folder = match self.workspace.fresh_folder("documents", &stem) {
            Ok(folder) => folder,
            Err(e) => return Outcome::error(e.to_string()),
        };
        let source = folder.join(format!("{stem}.{flat_ext}"));
        if let Err(e) = std::fs::write(&source, flat) {
            return Outcome::error(e.to_string());
        }
        let mut files = Vec::new();
        let mut made_all = Vec::new();
        for format in formats {
            match self.convert(&source, format, &folder) {
                Ok(made) => {
                    files.extend(made.iter().map(|p| self.workspace.relative(p)));
                    made_all.extend(made);
                }
                Err(why) => return Outcome::error(format!("{format}: {why}")),
            }
        }
        // A first-page picture, so the planner and the desk can see the result.
        if let Ok(made) = self.convert(&source, "png", &folder) {
            files.extend(made.iter().map(|p| self.workspace.relative(p)));
            made_all.extend(made);
        }
        let _ = std::fs::remove_file(&source);
        let outcome =
            Outcome::json(&json!({"folder": self.workspace.relative(&folder), "files": files}));
        match self.picture_of(&made_all) {
            Some(image) => outcome.with_image(image),
            None => outcome,
        }
    }

    fn formats(
        arguments: &Map<String, Value>,
        allowed: &[&str],
        default: &str,
    ) -> Result<Vec<String>, String> {
        let formats: Vec<String> = match arguments.get("formats") {
            None => vec![default.into()],
            Some(Value::Array(list)) => list
                .iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<_>>()
                .ok_or("formats are names")?,
            Some(_) => return Err("formats is a list".into()),
        };
        if formats.is_empty() {
            return Err("formats is empty".into());
        }
        for format in &formats {
            if !allowed.contains(&format.as_str()) {
                return Err(format!("{format} is not one of {allowed:?}"));
            }
        }
        Ok(formats)
    }

    fn picture(&self, file: &str) -> Result<odf::Picture, String> {
        let path = self.workspace.file(file).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err(format!("{file} is larger than 20 MB"));
        }
        odf::Picture::new(bytes).ok_or_else(|| format!("{file} is not a PNG or JPEG"))
    }

    fn write_document(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(name) = arguments.get("name").and_then(Value::as_str) else {
            return Outcome::error("office_write_document needs name");
        };
        let blocks: Vec<odf::Block> =
            match arguments.get("blocks").cloned().map(serde_json::from_value) {
                Some(Ok(blocks)) => blocks,
                Some(Err(e)) => return Outcome::error(format!("blocks: {e}")),
                None => return Outcome::error("office_write_document needs blocks"),
            };
        let formats = match Self::formats(
            arguments,
            &["docx", "odt", "pdf", "rtf", "html", "epub"],
            "docx",
        ) {
            Ok(formats) => formats,
            Err(why) => return Outcome::error(why),
        };
        let mut pictures = Vec::new();
        for block in &blocks {
            if let odf::Block::Image { file, .. } = block {
                match self.picture(file) {
                    Ok(picture) => pictures.push(picture),
                    Err(why) => return Outcome::error(why),
                }
            }
        }
        let title = arguments.get("title").and_then(Value::as_str).unwrap_or("");
        self.write(
            name,
            &odf::text_document(title, &blocks, &pictures),
            "fodt",
            &formats,
        )
    }

    fn write_spreadsheet(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(name) = arguments.get("name").and_then(Value::as_str) else {
            return Outcome::error("office_write_spreadsheet needs name");
        };
        let sheets: Vec<odf::Sheet> =
            match arguments.get("sheets").cloned().map(serde_json::from_value) {
                Some(Ok(sheets)) => sheets,
                Some(Err(e)) => return Outcome::error(format!("sheets: {e}")),
                None => return Outcome::error("office_write_spreadsheet needs sheets"),
            };
        if sheets.is_empty()
            || sheets
                .iter()
                .any(|s| s.name.is_empty() || s.name.len() > 31)
        {
            return Outcome::error("each sheet has a name of 1 to 31 characters");
        }
        let formats = match Self::formats(arguments, &["xlsx", "ods", "pdf", "csv", "html"], "xlsx")
        {
            Ok(formats) => formats,
            Err(why) => return Outcome::error(why),
        };
        self.write(name, &odf::spreadsheet(&sheets), "fods", &formats)
    }

    fn write_presentation(&self, arguments: &Map<String, Value>) -> Outcome {
        let Some(name) = arguments.get("name").and_then(Value::as_str) else {
            return Outcome::error("office_write_presentation needs name");
        };
        let slides: Vec<odf::Slide> =
            match arguments.get("slides").cloned().map(serde_json::from_value) {
                Some(Ok(slides)) => slides,
                Some(Err(e)) => return Outcome::error(format!("slides: {e}")),
                None => return Outcome::error("office_write_presentation needs slides"),
            };
        let formats = match Self::formats(arguments, &["pptx", "odp", "pdf"], "pptx") {
            Ok(formats) => formats,
            Err(why) => return Outcome::error(why),
        };
        let mut pictures = Vec::new();
        for slide in &slides {
            pictures.push(match &slide.image {
                Some(file) => match self.picture(file) {
                    Ok(picture) => Some(picture),
                    Err(why) => return Outcome::error(why),
                },
                None => None,
            });
        }
        let title = arguments
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(name);
        self.write(
            name,
            &odf::presentation(title, &slides, &pictures),
            "fodp",
            &formats,
        )
    }
}

fn listing(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

/// The text of one slide: what follows `<draw:page ` up to its end.
fn slide_text(page: &str) -> String {
    let inside = page.split_once('>').map(|(_, rest)| rest).unwrap_or("");
    strip_tags(inside.split("</draw:page>").next().unwrap_or(""))
}

/// The text of an XML fragment: paragraphs on their own lines, tags gone, entities read.
fn strip_tags(xml: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in xml.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                if tag.starts_with("/text:p")
                    || tag.starts_with("/text:h")
                    || tag.starts_with("text:line-break")
                {
                    out.push('\n');
                }
            }
            c if in_tag => tag.push(c),
            c => out.push(c),
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

impl Bridge for Office {
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
        let file = json!({"type": "string", "description": "A document in the workspace."});
        let formats =
            |list: &[&str]| json!({"type": "array", "items": {"type": "string", "enum": list}});
        vec![
            BridgeTool {
                name: "office_documents",
                title: "Documents",
                description: "The documents, spreadsheets and presentations in the workspace.",
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "office_text",
                title: "Read a document",
                description: "The text of a document, every sheet of a spreadsheet as CSV, or every slide of a \
                              presentation, read by LibreOffice. Works for Word, Excel and PowerPoint files too.",
                input_schema: json!({"type": "object", "properties": {"file": file}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::ReadOnly,
            },
            BridgeTool {
                name: "office_convert",
                title: "Convert a document",
                description: "Converts a document into exports/<name>-<n>/: text documents to pdf, docx, odt, rtf, \
                              txt, html, epub; spreadsheets to pdf, xlsx, ods, csv (one file per sheet), html; \
                              presentations to pdf, pptx, odp, html; anything to png (first page).",
                input_schema: json!({"type": "object", "properties": {"file": file, "to": {"type": "string"}}, "required": ["file", "to"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "office_preview",
                title: "Picture of the first page",
                description: "The first page, sheet or slide of a document as a picture in renders/, returned as an image.",
                input_schema: json!({"type": "object", "properties": {"file": file}, "required": ["file"], "additionalProperties": false}),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "office_write_document",
                title: "Write a document",
                description: "Writes a well-set document into documents/<name>-<n>/ from blocks: heading (level 1-3), \
                              paragraph, bullets, numbered, table (header and rows), image (a PNG or JPEG from the \
                              workspace, embedded, with caption), quote, page_break. Formats docx (default), odt, \
                              pdf, rtf, html, epub. Returns the files and a picture of the first page.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "title": {"type": "string"},
                        "blocks": {"type": "array", "items": {"type": "object"},
                                   "description": "Each is one of {\"heading\": {\"text\", \"level\"}}, {\"paragraph\": {\"text\"}}, {\"bullets\": {\"items\"}}, {\"numbered\": {\"items\"}}, {\"table\": {\"header\", \"rows\"}}, {\"image\": {\"file\", \"width_cm\", \"caption\"}}, {\"quote\": {\"text\"}}, {\"page_break\": {}}."},
                        "formats": formats(&["docx", "odt", "pdf", "rtf", "html", "epub"])
                    },
                    "required": ["name", "blocks"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "office_write_spreadsheet",
                title: "Write a spreadsheet",
                description: "Writes a spreadsheet into documents/<name>-<n>/ from sheets of rows. A cell is a number, \
                              a boolean, text, or a formula in Excel's syntax such as \"=SUM(B2:B9)\"; the first row \
                              is a coloured header unless header is false; columns are sized to their content. \
                              Formats xlsx (default), ods, pdf, csv, html.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "sheets": {"type": "array", "items": {"type": "object", "properties": {
                            "name": {"type": "string"}, "rows": {"type": "array", "items": {"type": "array"}}, "header": {"type": "boolean"}},
                            "required": ["name", "rows"]}},
                        "formats": formats(&["xlsx", "ods", "pdf", "csv", "html"])
                    },
                    "required": ["name", "sheets"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
            BridgeTool {
                name: "office_write_presentation",
                title: "Write a presentation",
                description: "Writes a 16:9 deck into documents/<name>-<n>/ from slides: title, optional subtitle, \
                              bullets, an optional picture from the workspace (right of the bullets, or full width) \
                              and speaker notes. A slide with only a title and subtitle is a cover. Formats pptx \
                              (default), odp, pdf.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "title": {"type": "string"},
                        "slides": {"type": "array", "items": {"type": "object", "properties": {
                            "title": {"type": "string"}, "subtitle": {"type": "string"},
                            "bullets": {"type": "array", "items": {"type": "string"}},
                            "image": {"type": "string"}, "notes": {"type": "string"}},
                            "required": ["title"]}},
                        "formats": formats(&["pptx", "odp", "pdf"])
                    },
                    "required": ["name", "slides"],
                    "additionalProperties": false
                }),
                grade: PrivacyGrade::Internal,
                effect: Effect::Writes,
            },
        ]
    }

    fn call(&self, tool: &str, arguments: &Map<String, Value>) -> Outcome {
        match tool {
            "office_documents" => self.documents(),
            "office_text" => self.text(arguments),
            "office_convert" => self.convert_tool(arguments),
            "office_preview" => self.preview(arguments),
            "office_write_document" => self.write_document(arguments),
            "office_write_spreadsheet" => self.write_spreadsheet(arguments),
            "office_write_presentation" => self.write_presentation(arguments),
            _ => Outcome::error("unknown tool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slides_read_as_text() {
        let xml = "<draw:page a=\"1\"><draw:frame><text:p>Title &amp; more</text:p></draw:frame><text:p>point</text:p></draw:page>";
        assert_eq!(
            slide_text(xml.split("<draw:page ").nth(1).unwrap()),
            "Title & more\npoint"
        );
    }

    #[test]
    fn conversions_are_checked_before_libreoffice_runs() {
        let dir = std::env::temp_dir().join(format!("archie-office-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plan.xlsx"), b"PK").unwrap();
        std::fs::write(dir.join("a.exe"), b"MZ").unwrap();
        let office = Office::new(
            Manifest::parse(MANIFEST).unwrap(),
            Workspace::open(&dir).unwrap(),
            &Programs::default(),
        );
        let args = |v: Value| v.as_object().unwrap().clone();
        assert!(office
            .call(
                "office_convert",
                &args(json!({"file": "plan.xlsx", "to": "pptx"}))
            )
            .text
            .contains("converts to"));
        assert!(
            office
                .call("office_text", &args(json!({"file": "a.exe"})))
                .is_error
        );
        let bad = office.call(
            "office_write_document",
            &args(json!({"name": "r", "blocks": [{"script": {}}]})),
        );
        assert!(bad.text.starts_with("blocks:"), "{}", bad.text);
        let missing = office.call(
            "office_write_document",
            &args(json!({"name": "r", "blocks": [{"image": {"file": "none.png"}}]})),
        );
        assert!(missing.is_error);
        assert!(!dir.join("exports").exists() && !dir.join("documents").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
