//! Flat OpenDocument, written from a description: a text document, a spreadsheet or a
//! presentation as one XML file that LibreOffice opens and converts to DOCX, XLSX, PPTX
//! or PDF. Pictures are embedded, not linked, so the document travels whole.
//!
//! The look is set here once: a clean sans-serif, one accent colour, generous spacing,
//! headings that Word and LibreOffice both know as Heading 1–3, tables with a coloured
//! header row.

use base64::Engine as _;
use serde::Deserialize;
use std::fmt::Write;

const ACCENT: &str = "#2f6fde";
const INK: &str = "#1d1d1b";
const MUTED: &str = "#5c5c57";
const FONT: &str = "Liberation Sans";

const NAMESPACES: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2" xmlns:msoxl="http://schemas.microsoft.com/office/excel/formula" office:version="1.3""#;

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML 1.0 cannot carry at all.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// Text inside a paragraph: line breaks and tabs as ODF writes them.
fn inline(text: &str) -> String {
    escape(text)
        .replace('\t', "<text:tab/>")
        .replace("\r\n", "\n")
        .replace('\n', "<text:line-break/>")
}

/// A picture to embed: its bytes, and its size in pixels.
#[derive(Debug, Clone)]
pub struct Picture {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Picture {
    /// Reads the pixel size of a PNG or JPEG; anything else is refused.
    pub fn new(bytes: Vec<u8>) -> Option<Self> {
        let (width, height) = image_size(&bytes)?;
        Some(Self {
            bytes,
            width,
            height,
        })
    }

    fn frame(&self, width_cm: f64, style: &str, anchor: &str) -> String {
        let height_cm = width_cm * self.height as f64 / self.width.max(1) as f64;
        format!(
            "<draw:frame draw:style-name=\"{style}\" text:anchor-type=\"{anchor}\" svg:width=\"{width_cm:.2}cm\" svg:height=\"{height_cm:.2}cm\"><draw:image><office:binary-data>{}</office:binary-data></draw:image></draw:frame>",
            base64::engine::general_purpose::STANDARD.encode(&self.bytes)
        )
    }
}

pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                return None;
            }
            let marker = bytes[i + 1];
            let length = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + length;
        }
    }
    None
}

// ── Text documents ──────────────────────────────────────────────────────────────────

/// One block of a document, as a planner describes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Block {
    Heading {
        text: String,
        #[serde(default = "one")]
        level: u8,
    },
    Paragraph {
        text: String,
    },
    Bullets {
        items: Vec<String>,
    },
    Numbered {
        items: Vec<String>,
    },
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// A picture from the workspace, `width_cm` wide (default 15).
    Image {
        file: String,
        #[serde(default)]
        width_cm: Option<f64>,
        #[serde(default)]
        caption: Option<String>,
    },
    Quote {
        text: String,
    },
    PageBreak {},
}

fn one() -> u8 {
    1
}

fn list_style(name: &str, numbered: bool) -> String {
    let level = if numbered {
        "<text:list-level-style-number text:level=\"1\" style:num-suffix=\".\" style:num-format=\"1\">"
    } else {
        "<text:list-level-style-bullet text:level=\"1\" text:bullet-char=\"•\">"
    };
    let close = if numbered {
        "</text:list-level-style-number>"
    } else {
        "</text:list-level-style-bullet>"
    };
    format!(
        "<text:list-style style:name=\"{name}\">{level}<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"><style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"0.9cm\" fo:text-indent=\"-0.5cm\" fo:margin-left=\"0.9cm\"/></style:list-level-properties>{close}</text:list-style>"
    )
}

/// A text document with `title` and `blocks`. `pictures` holds, in order, the picture of
/// every image block.
pub fn text_document(title: &str, blocks: &[Block], pictures: &[Picture]) -> String {
    let mut body = String::new();
    if !title.is_empty() {
        let _ = write!(
            body,
            "<text:p text:style-name=\"Title\">{}</text:p>",
            inline(title)
        );
    }
    let mut picture = pictures.iter();
    for block in blocks {
        match block {
            Block::Heading { text, level } => {
                let level = (*level).clamp(1, 3);
                let _ = write!(
                    body,
                    "<text:h text:style-name=\"Heading_20_{level}\" text:outline-level=\"{level}\">{}</text:h>",
                    inline(text)
                );
            }
            Block::Paragraph { text } => {
                for paragraph in text.split("\n\n") {
                    let _ = write!(
                        body,
                        "<text:p text:style-name=\"Text_20_body\">{}</text:p>",
                        inline(paragraph)
                    );
                }
            }
            Block::Quote { text } => {
                let _ = write!(
                    body,
                    "<text:p text:style-name=\"Quotations\">{}</text:p>",
                    inline(text)
                );
            }
            Block::Bullets { items } | Block::Numbered { items } => {
                let style = if matches!(block, Block::Numbered { .. }) {
                    "Numbers"
                } else {
                    "Bullets"
                };
                let _ = write!(body, "<text:list text:style-name=\"{style}\">");
                for item in items {
                    let _ = write!(body, "<text:list-item><text:p text:style-name=\"List_20_Item\">{}</text:p></text:list-item>", inline(item));
                }
                body.push_str("</text:list>");
            }
            Block::Table { header, rows } => {
                let columns = header
                    .len()
                    .max(rows.iter().map(Vec::len).max().unwrap_or(0))
                    .max(1);
                let _ = write!(
                    body,
                    "<table:table table:style-name=\"Tbl\"><table:table-column table:number-columns-repeated=\"{columns}\"/>"
                );
                if !header.is_empty() {
                    body.push_str("<table:table-header-rows><table:table-row>");
                    for c in 0..columns {
                        let text = header.get(c).map(String::as_str).unwrap_or("");
                        let _ = write!(body, "<table:table-cell table:style-name=\"TblHead\" office:value-type=\"string\"><text:p text:style-name=\"Table_20_Heading\">{}</text:p></table:table-cell>", inline(text));
                    }
                    body.push_str("</table:table-row></table:table-header-rows>");
                }
                for (r, row) in rows.iter().enumerate() {
                    body.push_str("<table:table-row>");
                    let style = if r % 2 == 1 { "TblZebra" } else { "TblCell" };
                    for c in 0..columns {
                        let text = row.get(c).map(String::as_str).unwrap_or("");
                        let _ = write!(body, "<table:table-cell table:style-name=\"{style}\" office:value-type=\"string\"><text:p text:style-name=\"Table_20_Contents\">{}</text:p></table:table-cell>", inline(text));
                    }
                    body.push_str("</table:table-row>");
                }
                body.push_str("</table:table><text:p text:style-name=\"Text_20_body\"/>");
            }
            Block::Image {
                width_cm, caption, ..
            } => {
                if let Some(picture) = picture.next() {
                    let width = width_cm.unwrap_or(15.0).clamp(2.0, 17.0);
                    let _ = write!(
                        body,
                        "<text:p text:style-name=\"Figure\">{}</text:p>",
                        picture.frame(width, "Img", "as-char")
                    );
                    if let Some(caption) = caption {
                        let _ = write!(
                            body,
                            "<text:p text:style-name=\"Caption\">{}</text:p>",
                            inline(caption)
                        );
                    }
                }
            }
            Block::PageBreak {} => body.push_str("<text:p text:style-name=\"Break\"/>"),
        }
    }
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document {NAMESPACES} office:mimetype="application/vnd.oasis.opendocument.text">
<office:meta><dc:title>{title}</dc:title></office:meta>
<office:font-face-decls><style:font-face style:name="{FONT}" svg:font-family="'{FONT}'" style:font-family-generic="swiss" style:font-pitch="variable"/></office:font-face-decls>
<office:styles>
<style:default-style style:family="paragraph"><style:paragraph-properties fo:line-height="135%"/><style:text-properties style:font-name="{FONT}" fo:font-size="11pt" fo:color="{INK}" fo:language="en"/></style:default-style>
<style:style style:name="Standard" style:family="paragraph" style:class="text"/>
<style:style style:name="Text_20_body" style:display-name="Text body" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0.25cm"/></style:style>
<style:style style:name="Title" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-bottom="0.5cm" fo:padding-bottom="0.2cm" fo:border-bottom="1.5pt solid {ACCENT}"/><style:text-properties fo:font-size="24pt" fo:font-weight="bold"/></style:style>
<style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="1"><style:paragraph-properties fo:margin-top="0.6cm" fo:margin-bottom="0.2cm" fo:keep-with-next="always"/><style:text-properties fo:font-size="17pt" fo:font-weight="bold" fo:color="{ACCENT}"/></style:style>
<style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="2"><style:paragraph-properties fo:margin-top="0.45cm" fo:margin-bottom="0.15cm" fo:keep-with-next="always"/><style:text-properties fo:font-size="13.5pt" fo:font-weight="bold"/></style:style>
<style:style style:name="Heading_20_3" style:display-name="Heading 3" style:family="paragraph" style:parent-style-name="Standard" style:default-outline-level="3"><style:paragraph-properties fo:margin-top="0.35cm" fo:margin-bottom="0.1cm" fo:keep-with-next="always"/><style:text-properties fo:font-size="11.5pt" fo:font-weight="bold" fo:color="{MUTED}"/></style:style>
<style:style style:name="List_20_Item" style:display-name="List Item" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-bottom="0.1cm"/></style:style>
<style:style style:name="Quotations" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="0.6cm" fo:margin-bottom="0.25cm" fo:padding-left="0.3cm" fo:border-left="2pt solid {ACCENT}"/><style:text-properties fo:font-style="italic" fo:color="{MUTED}"/></style:style>
<style:style style:name="Table_20_Contents" style:display-name="Table Contents" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="10pt"/></style:style>
<style:style style:name="Table_20_Heading" style:display-name="Table Heading" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="10pt" fo:font-weight="bold" fo:color="#ffffff"/></style:style>
<style:style style:name="Figure" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:text-align="center" fo:margin-top="0.2cm" fo:margin-bottom="0.1cm"/></style:style>
<style:style style:name="Caption" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:text-align="center" fo:margin-bottom="0.35cm"/><style:text-properties fo:font-size="9.5pt" fo:font-style="italic" fo:color="{MUTED}"/></style:style>
{bullets}
{numbers}
</office:styles>
<office:automatic-styles>
<style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout>
<style:style style:name="Break" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:break-before="page"/></style:style>
<style:style style:name="Tbl" style:family="table"><style:table-properties style:width="17cm" table:align="margins" fo:margin-top="0.15cm" fo:margin-bottom="0.15cm"/></style:style>
<style:style style:name="TblHead" style:family="table-cell"><style:table-cell-properties fo:background-color="{ACCENT}" fo:padding="0.12cm" fo:border="0.5pt solid {ACCENT}"/></style:style>
<style:style style:name="TblCell" style:family="table-cell"><style:table-cell-properties fo:padding="0.12cm" fo:border="0.5pt solid #d6d5cf"/></style:style>
<style:style style:name="TblZebra" style:family="table-cell"><style:table-cell-properties fo:background-color="#f3f3f0" fo:padding="0.12cm" fo:border="0.5pt solid #d6d5cf"/></style:style>
<style:style style:name="Img" style:family="graphic"><style:graphic-properties style:wrap="none" style:vertical-pos="top" style:vertical-rel="baseline"/></style:style>
</office:automatic-styles>
<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
<office:body><office:text>{body}</office:text></office:body>
</office:document>
"##,
        title = escape(title),
        bullets = list_style("Bullets", false),
        numbers = list_style("Numbers", true),
    )
}

// ── Spreadsheets ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub name: String,
    /// Rows of cells: numbers, booleans, text, or text starting with `=` for a formula
    /// in Excel's A1 syntax, such as `=SUM(B2:B9)`.
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Whether the first row is a header, drawn in the accent colour. Default true.
    #[serde(default = "yes")]
    pub header: bool,
}

fn yes() -> bool {
    true
}

fn cell_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn spreadsheet(sheets: &[Sheet]) -> String {
    let mut styles = String::new();
    let mut tables = String::new();
    for (s, sheet) in sheets.iter().enumerate() {
        let columns = sheet.rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let _ = write!(
            tables,
            "<table:table table:name=\"{}\">",
            escape(&sheet.name)
        );
        for c in 0..columns {
            let longest = sheet
                .rows
                .iter()
                .filter_map(|row| row.get(c))
                .map(|v| cell_text(v).chars().count())
                .max()
                .unwrap_or(4);
            let width = (longest as f64 * 0.21 + 0.7).clamp(1.8, 11.0);
            let name = format!("co{s}_{c}");
            let _ = write!(styles, "<style:style style:name=\"{name}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{width:.2}cm\"/></style:style>");
            let _ = write!(tables, "<table:table-column table:style-name=\"{name}\"/>");
        }
        for (r, row) in sheet.rows.iter().enumerate() {
            tables.push_str("<table:table-row>");
            let head = sheet.header && r == 0;
            let style = if head { " table:style-name=\"hd\"" } else { "" };
            for c in 0..columns {
                match row.get(c).unwrap_or(&serde_json::Value::Null) {
                    serde_json::Value::Number(n) => {
                        let _ = write!(tables, "<table:table-cell{style} office:value-type=\"float\" office:value=\"{n}\"><text:p>{n}</text:p></table:table-cell>");
                    }
                    serde_json::Value::Bool(b) => {
                        let _ = write!(tables, "<table:table-cell{style} office:value-type=\"boolean\" office:boolean-value=\"{b}\"><text:p>{}</text:p></table:table-cell>", if *b { "TRUE" } else { "FALSE" });
                    }
                    serde_json::Value::String(text)
                        if text.starts_with('=') && text.len() > 1 && !head =>
                    {
                        let _ = write!(tables, "<table:table-cell{style} table:formula=\"msoxl:{}\" office:value-type=\"float\" office:value=\"0\"><text:p/></table:table-cell>", escape(text));
                    }
                    serde_json::Value::Null => {
                        let _ = write!(tables, "<table:table-cell{style}/>");
                    }
                    other => {
                        let _ = write!(tables, "<table:table-cell{style} office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>", escape(&cell_text(other)));
                    }
                }
            }
            tables.push_str("</table:table-row>");
        }
        tables.push_str("</table:table>");
    }
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document {NAMESPACES} office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
<office:font-face-decls><style:font-face style:name="{FONT}" svg:font-family="'{FONT}'" style:font-family-generic="swiss"/></office:font-face-decls>
<office:styles><style:default-style style:family="table-cell"><style:text-properties style:font-name="{FONT}" fo:font-size="10.5pt"/></style:default-style></office:styles>
<office:automatic-styles>
<style:style style:name="hd" style:family="table-cell"><style:table-cell-properties fo:background-color="{ACCENT}"/><style:text-properties fo:font-weight="bold" fo:color="#ffffff"/></style:style>
{styles}
</office:automatic-styles>
<office:body><office:spreadsheet>{tables}</office:spreadsheet></office:body>
</office:document>
"##
    )
}

// ── Presentations ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slide {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub bullets: Vec<String>,
    /// A picture from the workspace, shown on the right (or full width without bullets).
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A 16:9 deck. `pictures[i]` is slide i's picture, if it has one.
pub fn presentation(title: &str, slides: &[Slide], pictures: &[Option<Picture>]) -> String {
    let mut pages = String::new();
    for (i, slide) in slides.iter().enumerate() {
        let picture = pictures.get(i).and_then(Option::as_ref);
        let _ = write!(pages, "<draw:page draw:name=\"slide{n}\" draw:master-page-name=\"Archie\" draw:style-name=\"dp1\">", n = i + 1);
        pages.push_str("<draw:rect draw:style-name=\"bar\" svg:x=\"0cm\" svg:y=\"0cm\" svg:width=\"28cm\" svg:height=\"0.35cm\"/>");
        let lone = slide.bullets.is_empty() && picture.is_none();
        let title_y = if lone { "5.2cm" } else { "0.9cm" };
        let _ = write!(
            pages,
            "<draw:frame presentation:class=\"title\" svg:x=\"1.6cm\" svg:y=\"{title_y}\" svg:width=\"24.8cm\" svg:height=\"2.4cm\"><draw:text-box><text:p text:style-name=\"{style}\">{}</text:p></draw:text-box></draw:frame>",
            inline(&slide.title),
            style = if lone { "PCover" } else { "PTitle" },
        );
        if let Some(subtitle) = &slide.subtitle {
            let y = if lone { "7.8cm" } else { "3.2cm" };
            let _ = write!(pages, "<draw:frame presentation:class=\"subtitle\" svg:x=\"1.6cm\" svg:y=\"{y}\" svg:width=\"24.8cm\" svg:height=\"1.4cm\"><draw:text-box><text:p text:style-name=\"PSub\">{}</text:p></draw:text-box></draw:frame>", inline(subtitle));
        }
        let body_top = if slide.subtitle.is_some() { 4.8 } else { 3.6 };
        let body_width = if picture.is_some() { 12.6 } else { 24.8 };
        if !slide.bullets.is_empty() {
            let _ = write!(pages, "<draw:frame presentation:class=\"outline\" svg:x=\"1.6cm\" svg:y=\"{body_top}cm\" svg:width=\"{body_width}cm\" svg:height=\"{h}cm\"><draw:text-box><text:list text:style-name=\"PBullets\">", h = 14.8 - body_top);
            for bullet in &slide.bullets {
                let _ = write!(pages, "<text:list-item><text:p text:style-name=\"PBody\">{}</text:p></text:list-item>", inline(bullet));
            }
            pages.push_str("</text:list></draw:text-box></draw:frame>");
        }
        if let Some(picture) = picture {
            let (x, max_w) = if slide.bullets.is_empty() {
                (1.6, 24.8)
            } else {
                (14.8, 11.6)
            };
            let max_h = 14.6 - body_top;
            let aspect = picture.height as f64 / picture.width.max(1) as f64;
            let (w, h) = if max_w * aspect <= max_h {
                (max_w, max_w * aspect)
            } else {
                (max_h / aspect, max_h)
            };
            let x = x + (max_w - w) / 2.0;
            let _ = write!(
                pages,
                "<draw:frame draw:style-name=\"pic\" svg:x=\"{x:.2}cm\" svg:y=\"{body_top:.2}cm\" svg:width=\"{w:.2}cm\" svg:height=\"{h:.2}cm\"><draw:image><office:binary-data>{}</office:binary-data></draw:image></draw:frame>",
                base64::engine::general_purpose::STANDARD.encode(&picture.bytes)
            );
        }
        if let Some(notes) = &slide.notes {
            let _ = write!(pages, "<presentation:notes><draw:frame presentation:class=\"notes\" svg:x=\"2cm\" svg:y=\"14cm\" svg:width=\"17cm\" svg:height=\"12cm\"><draw:text-box><text:p>{}</text:p></draw:text-box></draw:frame></presentation:notes>", inline(notes));
        }
        pages.push_str("</draw:page>");
    }
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document {NAMESPACES} office:mimetype="application/vnd.oasis.opendocument.presentation">
<office:meta><dc:title>{title}</dc:title></office:meta>
<office:font-face-decls><style:font-face style:name="{FONT}" svg:font-family="'{FONT}'" style:font-family-generic="swiss"/></office:font-face-decls>
<office:styles>
<style:default-style style:family="graphic"><style:graphic-properties draw:stroke="none" draw:fill="none"/><style:text-properties style:font-name="{FONT}" fo:color="{INK}"/></style:default-style>
<style:style style:name="PCover" style:family="paragraph"><style:text-properties fo:font-size="40pt" fo:font-weight="bold"/></style:style>
<style:style style:name="PTitle" style:family="paragraph"><style:text-properties fo:font-size="30pt" fo:font-weight="bold"/></style:style>
<style:style style:name="PSub" style:family="paragraph"><style:text-properties fo:font-size="18pt" fo:color="{MUTED}"/></style:style>
<style:style style:name="PBody" style:family="paragraph"><style:paragraph-properties fo:margin-bottom="0.3cm"/><style:text-properties fo:font-size="20pt"/></style:style>
<text:list-style style:name="PBullets"><text:list-level-style-bullet text:level="1" text:bullet-char="●"><style:list-level-properties text:space-before="0cm" text:min-label-width="0.9cm"/><style:text-properties fo:color="{ACCENT}" fo:font-size="70%"/></text:list-level-style-bullet></text:list-style>
</office:styles>
<office:automatic-styles>
<style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="28cm" fo:page-height="15.75cm" style:print-orientation="landscape"/></style:page-layout>
<style:style style:name="dp1" style:family="drawing-page"><style:drawing-page-properties draw:fill="solid" draw:fill-color="#ffffff" presentation:background-visible="true"/></style:style>
<style:style style:name="bar" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="{ACCENT}" draw:stroke="none"/></style:style>
<style:style style:name="pic" style:family="graphic"><style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>
</office:automatic-styles>
<office:master-styles><style:master-page style:name="Archie" style:page-layout-name="pm1" draw:style-name="dp1"/></office:master-styles>
<office:body><office:presentation>{pages}</office:presentation></office:body>
</office:document>
"##,
        title = escape(title),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, w, h);
            encoder.set_color(png::ColorType::Rgb);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![200; (w * h * 3) as usize])
                .unwrap();
        }
        out
    }

    #[test]
    fn picture_sizes_are_read_from_the_file() {
        assert_eq!(image_size(&png(64, 32)), Some((64, 32)));
        // A minimal JPEG header: SOI, then SOF0 with height 20 and width 40.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x14, 0x00, 0x28, 0x03, 0, 0, 0, 0,
        ];
        assert_eq!(image_size(&jpeg), Some((40, 20)));
        assert_eq!(image_size(b"GIF89a"), None);
    }

    #[test]
    fn a_document_escapes_text_and_embeds_pictures() {
        let blocks: Vec<Block> = serde_json::from_value(serde_json::json!([
            {"heading": {"text": "Results & <next> steps", "level": 1}},
            {"paragraph": {"text": "Line one\nline two\n\nSecond paragraph"}},
            {"bullets": {"items": ["a", "b"]}},
            {"table": {"header": ["Part", "Mass"], "rows": [["Bracket", "41 g"], ["Plate", "12 g"]]}},
            {"image": {"file": "p.png", "width_cm": 10}},
            {"page_break": {}}
        ]))
        .unwrap();
        let xml = text_document("Report", &blocks, &[Picture::new(png(4, 2)).unwrap()]);
        assert!(xml.contains("Results &amp; &lt;next&gt; steps"));
        assert!(xml.contains("<text:line-break/>"));
        assert_eq!(xml.matches("text:style-name=\"Text_20_body\">").count(), 2);
        assert!(xml.contains("svg:width=\"10.00cm\" svg:height=\"5.00cm\""));
        assert!(xml.contains("<office:binary-data>"));
        assert!(xml.contains("TblZebra"));
        assert!(serde_json::from_value::<Vec<Block>>(
            serde_json::json!([{"script": {"text": "x"}}])
        )
        .is_err());
    }

    #[test]
    fn spreadsheets_type_their_cells() {
        let sheets: Vec<Sheet> = serde_json::from_value(serde_json::json!([
            {"name": "Parts & mass", "rows": [["Part", "Mass g"], ["Bracket", 41.5], ["Total", "=SUM(B2:B2)"], ["ok", true]]}
        ]))
        .unwrap();
        let xml = spreadsheet(&sheets);
        assert!(xml.contains("table:name=\"Parts &amp; mass\""));
        assert!(xml.contains("office:value=\"41.5\""));
        assert!(xml.contains("table:formula=\"msoxl:=SUM(B2:B2)\""));
        assert!(xml.contains("office:boolean-value=\"true\""));
        assert!(xml.contains("table:style-name=\"hd\""));
    }

    #[test]
    fn a_deck_places_bullets_and_pictures() {
        let slides: Vec<Slide> = serde_json::from_value(serde_json::json!([
            {"title": "Bracket study", "subtitle": "200 variants"},
            {"title": "Findings", "bullets": ["3 mm wall is enough", "mass −18 %"], "image": "iso.png", "notes": "say why"}
        ]))
        .unwrap();
        let xml = presentation(
            "Study",
            &slides,
            &[None, Some(Picture::new(png(40, 30)).unwrap())],
        );
        assert!(
            xml.contains("PCover"),
            "a slide with only a title is a cover"
        );
        assert_eq!(xml.matches("<text:list-item>").count(), 2);
        assert!(xml.contains("<presentation:notes>"));
        assert_eq!(xml.matches("<draw:page ").count(), 2);
    }
}
