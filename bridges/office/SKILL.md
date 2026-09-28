---
name: bridge-office
description: Write and read documents, spreadsheets and presentations through Archie's office bridge (LibreOffice) — reports as DOCX/PDF, tables as XLSX with formulas, decks as PPTX — and convert or read Word, Excel and PowerPoint files. Use when a task produces or reads an office document.
---

# Documents through Archie

The bridge runs LibreOffice headless, with its own profile, in one folder of documents.
It never touches the person's LibreOffice settings.

## Reading

- `office_documents` lists the documents.
- `office_text` reads one:
  - a document becomes plain text;
  - a spreadsheet becomes every sheet as CSV;
  - a presentation becomes a list of slides.

  Word, Excel and PowerPoint files work too. PDFs do not.
- `office_preview` gives a picture of the first page, sheet or slide.

## Writing

Describe the content. The bridge sets it well: one clean font, one accent colour, and
headings that Word's navigation pane and table of contents recognise. It returns the
files and a picture of page one.

**`office_write_document {name, title, blocks, formats}`**. Blocks, in order:

- `{"heading": {"text": "...", "level": 1}}` — levels 1 to 3
- `{"paragraph": {"text": "..."}}` — a blank line inside starts a new paragraph
- `{"bullets": {"items": [...]}}`, `{"numbered": {"items": [...]}}`
- `{"table": {"header": [...], "rows": [[...], ...]}}` — a coloured header row, then
  striped rows
- `{"image": {"file": "renders/part.png", "width_cm": 12, "caption": "..."}}` — PNG or
  JPEG from the workspace, embedded in the file
- `{"quote": {"text": "..."}}`, `{"page_break": {}}`

Formats: `docx` (the default), `odt`, `pdf`, `rtf`, `html`, `epub`. Ask for `docx` and
`pdf` together when the person will both edit and share it.

**`office_write_spreadsheet {name, sheets, formats}`**. Each sheet has a `name` (up to 31
characters) and `rows`.

- A cell is a number, `true`/`false`, text, or a formula in Excel syntax: `"=SUM(B2:B9)"`,
  `"=AVERAGE(C:C)"`, `"=B2*C2"`.
- Keep numbers as numbers, not text, so formulas and sorting work.
- The first row is a header unless `header` is false.

Formats: `xlsx` (the default), `ods`, `pdf`, `csv`, `html`.

**`office_write_presentation {name, title, slides, formats}`**. Each slide has:

- `title`;
- optionally `subtitle`, `bullets`, `image` (a picture beside the bullets, or full width
  without them) and `notes`.

A slide with only a title and subtitle is a cover. Keep to 3–6 bullets of one line each.
Formats: `pptx` (the default), `odp`, `pdf`.

## Converting

`office_convert {file, to}` writes into `exports/<name>-<n>/`:

- Word to PDF, and back;
- Excel to CSV, one file per sheet;
- PowerPoint to PDF;
- any document to a PNG of its first page.

The error names the formats each kind converts to.

## Traps

- Formula results appear when the file is opened. The picture of a spreadsheet may show
  them computed, or blank, depending on the format.
- A document with many pictures is large. Scale pictures before embedding them.
- The first conversion after a fresh start takes a few seconds longer while LibreOffice
  sets up its profile.

## When to use the GUI instead

Open the document in LibreOffice on the Archie Stage for:

- fine layout by eye;
- charts;
- tracked changes and comments.

Check the result with `office_preview`.
