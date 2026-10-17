//! A Protocol's `report.md`, typeset to PDF with each chart it lists drawn in
//! its place.
//!
//! arc writes a report as an agent's prose followed by sections of its own,
//! one of which, `## Charts`, lists each panel as `` - `panels/<file>`: <title> ``.
//! This crate turns the Markdown into Typst source ([`to_typst`]), replacing
//! each such item with its title, set as a caption, above the chart's picture,
//! and compiles the source to a PDF ([`compile_pdf`]) or to one PNG per page
//! ([`preview_pages`]).
//!
//! The pictures come from the caller: this crate knows nothing of panels,
//! databases or the GPU. It compiles offline, in a world holding the fonts
//! bundled with `meridian-design` and the pictures, and nothing else.
//!
//! The page is A4 portrait, set in 11pt Inter with Inter SemiBold headings and
//! numbered at the foot. Markdown emphasis is set in Inter Medium, because no
//! italic Inter is bundled, strong emphasis in Inter Bold, and code in
//! JetBrains Mono. A table is set at 9pt, or 8pt from 7 columns, with numbers
//! right-aligned, in a block that does not break; a table of 10 columns or
//! more goes on a landscape page of its own.

mod markdown;
mod world;

use typst::diag::SourceDiagnostic;
use typst::ecow::EcoVec;
use typst::utils::Scalar;
use typst_layout::PagedDocument;

use world::ReportWorld;

/// One chart's picture, for the item in the report's Charts section that
/// names its panel.
#[derive(Clone, Debug)]
pub struct ChartImage {
    /// The panel file the item names, as `panels/x.yaml`.
    pub path: String,
    /// The title set as the chart's caption.
    pub title: String,
    /// The picture, PNG-encoded.
    pub png: Vec<u8>,
    /// The picture's width in pixels.
    pub width_px: u32,
    /// The picture's height in pixels.
    pub height_px: u32,
}

/// What a report is made from.
#[derive(Clone, Copy, Debug)]
pub struct ReportInput<'a> {
    /// The text of `report.md`.
    pub markdown: &'a str,
    /// The charts' pictures. A chart the report lists that is not here stays
    /// the list item it was in [`to_typst`], and fails [`compile_pdf`].
    pub charts: &'a [ChartImage],
    /// A note set in a shaded block above the Charts heading, as Markdown.
    pub notice: Option<&'a str>,
}

/// A chart the report's Charts section lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedChart {
    /// The panel file, as `panels/x.yaml`.
    pub path: String,
    /// The title the list item gives it.
    pub title: String,
    /// The text of each `Check:` line under the item, without the word: the
    /// findings arc's panel check made when it wrote the report.
    pub checks: Vec<String>,
}

/// The charts the `## Charts` section of `markdown` lists, in order, with any
/// `Check:` lines under each.
#[must_use]
pub fn listed_charts(markdown: &str) -> Vec<ListedChart> {
    markdown::listed_charts(markdown)
}

/// The path in the Typst world of the picture of `input.charts[index]`.
fn chart_file(index: usize) -> String {
    format!("/chart-{index}.png")
}

/// The settings every report starts with: the page, the type and the styles
/// of headings, code and quotes.
const PREAMBLE: &str = r##"#set document(title: none)
#set page(
  paper: "a4",
  margin: (x: 20mm, top: 20mm, bottom: 22mm),
  footer: context align(center, text(size: 8.5pt, fill: rgb("#6b737b"), counter(page).display())),
)
#set text(font: "Inter", size: 11pt, lang: "en", region: "gb", fill: rgb("#1d2125"))
#set par(leading: 0.62em, spacing: 1.05em)
#show heading: set text(weight: "semibold")
#show heading: set block(above: 1.5em, below: 0.75em)
#show heading.where(level: 1): set text(size: 17pt)
#show heading.where(level: 2): set text(size: 13.5pt)
#show heading.where(level: 3): set text(size: 11.5pt)
#show raw: set text(font: "JetBrains Mono")
#show raw.where(block: true): set block(fill: rgb("#f4f5f6"), inset: 8pt, radius: 3pt, width: 100%)
#show quote.where(block: true): set block(inset: (left: 10pt), stroke: (left: 2pt + rgb("#c8ccd0")))
#set list(indent: 0.4em, body-indent: 0.6em)
#set enum(indent: 0.4em, body-indent: 0.6em)

"##;

/// The Typst source for the report: the preamble, then the Markdown as Typst
/// markup with every character of its text escaped, each listed chart that
/// `input.charts` holds in place of its list item, and the notice above the
/// Charts heading.
#[must_use]
pub fn to_typst(input: &ReportInput) -> String {
    let mut source = String::from(PREAMBLE);
    source.push_str(&markdown::body(input.markdown, input.charts, input.notice));
    source
}

/// The problems that keep the report from being set: a `Check:` line under a
/// chart, or a listed chart with no picture. Each line names the panel file.
fn listing_problems(input: &ReportInput) -> Vec<String> {
    let mut problems = Vec::new();
    for listed in listed_charts(input.markdown) {
        for check in &listed.checks {
            problems.push(format!(
                "{}: report.md says Check: {check}; fix the panel and write the report again",
                listed.path
            ));
        }
        if listed.checks.is_empty() && !input.charts.iter().any(|c| c.path == listed.path) {
            problems.push(format!(
                "{}: report.md lists the chart, and it was not drawn",
                listed.path
            ));
        }
    }
    problems
}

fn diagnostics(errors: &EcoVec<SourceDiagnostic>) -> String {
    errors
        .iter()
        .map(|e| e.message.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// The report laid out as pages.
fn layout(input: &ReportInput) -> Result<PagedDocument, String> {
    let problems = listing_problems(input);
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let files = input
        .charts
        .iter()
        .enumerate()
        .map(|(i, chart)| (chart_file(i), chart.png.clone()))
        .collect();
    let world = ReportWorld::new(to_typst(input), files);
    typst::compile::<PagedDocument>(&world)
        .output
        .map_err(|errors| format!("the report could not be set: {}", diagnostics(&errors)))
}

/// The report as a PDF.
///
/// # Errors
///
/// A `Check:` line under a chart in the Charts section, a listed chart with no
/// picture, or Typst's own errors; the lines are joined with newlines, and
/// each about a chart starts with its panel file.
pub fn compile_pdf(input: &ReportInput) -> Result<Vec<u8>, String> {
    let document = layout(input)?;
    typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default())
        .map_err(|errors| format!("the PDF could not be written: {}", diagnostics(&errors)))
}

/// Each page of the report as a PNG, at `ppi` pixels to the inch.
///
/// # Errors
///
/// As [`compile_pdf`], or a page that could not be encoded.
pub fn preview_pages(input: &ReportInput, ppi: f32) -> Result<Vec<Vec<u8>>, String> {
    let document = layout(input)?;
    let options = typst_render::RenderOptions {
        pixel_per_pt: Scalar::new(f64::from(ppi) / 72.0),
        render_bleed: false,
    };
    document
        .pages()
        .iter()
        .map(|page| {
            typst_render::render(page, &options)
                .encode_png()
                .map_err(|e| format!("a page could not be encoded as PNG: {e}"))
        })
        .collect()
}
