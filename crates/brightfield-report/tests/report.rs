//! A Protocol's report.md, set as Typst and compiled to a PDF: prose is printed
//! as written, each chart replaces its list item, wide tables turn the page and
//! no chart is split across two pages.

use std::io::Cursor;

use brightfield_report::{compile_pdf, preview_pages, to_typst, ChartImage, ReportInput};

/// A PNG of `w` by `h` pixels, a flat mid-grey.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([128, 128, 128, 255]));
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("the PNG encodes");
    bytes
}

fn chart(path: &str, title: &str, w: u32, h: u32) -> ChartImage {
    ChartImage {
        path: path.to_string(),
        title: title.to_string(),
        png: png(w, h),
        width_px: w,
        height_px: h,
        note: None,
    }
}

fn input<'a>(markdown: &'a str, charts: &'a [ChartImage]) -> ReportInput<'a> {
    ReportInput {
        markdown,
        charts,
        notice: None,
        title: None,
        subtitle: None,
    }
}

/// The text a reader of the PDF would copy out of it, with runs of white space
/// made one space, so a line break the page made does not split a phrase.
fn pdf_text(pdf: &[u8]) -> String {
    let text = pdf_extract::extract_text_from_mem(pdf).expect("the PDF's text reads");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn typst_markup_in_prose_is_printed_literally() {
    // The Markdown escapes keep `*a*` and `_b_` from being emphasis, and the
    // code span holds the same characters again; neither may be read as Typst.
    let markdown = "Literal: # $5 \\*a\\* \\_b\\_ @c <d> [e] \\f ~ a // b\n\n\
                    In code: `# $5 *a* _b_ @c <d> [e] \\f`.\n";
    let source = to_typst(&input(markdown, &[]));
    for escaped in [
        "\\#", "\\$5", "\\*a\\*", "\\_b\\_", "\\@c", "\\<d\\>", "\\[e\\]", "\\\\f", "\\~", "\\/\\/",
    ] {
        assert!(
            source.contains(escaped),
            "{escaped} is not escaped:\n{source}"
        );
    }
    let pdf = compile_pdf(&input(markdown, &[])).expect("the report compiles");
    let text = pdf_text(&pdf);
    assert!(
        text.contains("Literal: # $5 *a* _b_ @c <d> [e] \\f ~ a // b"),
        "{text}"
    );
    assert!(text.contains("# $5 *a* _b_ @c <d> [e] \\f"), "{text}");
}

#[test]
fn charts_replace_their_list_items() {
    let markdown = "Some prose.\n\n## Charts\n\n\
                    Findings are checked against the Protocol's current database.\n\n\
                    - `panels/first.yaml`: The first chart\n\
                    - `panels/second.yaml`: The second chart\n\n\
                    ## Run\n\n- Run: `r1`\n";
    let charts = [
        chart("panels/first.yaml", "The first chart", 40, 20),
        chart("panels/second.yaml", "The second chart", 40, 20),
    ];
    let source = to_typst(&input(markdown, &charts));
    assert_eq!(source.matches("#bf-chart(").count(), 2, "{source}");
    assert!(!source.contains("first.yaml"), "{source}");
    assert!(!source.contains("second.yaml"), "{source}");
    assert!(source.contains("The first chart"), "{source}");
    let pdf = compile_pdf(&input(markdown, &charts)).expect("the report compiles");
    let text = pdf_text(&pdf);
    assert!(
        text.contains("The first chart") && text.contains("The second chart"),
        "{text}"
    );
    assert!(!text.contains("panels/first.yaml"), "{text}");
}

fn table(columns: usize) -> String {
    let header: Vec<String> = (0..columns).map(|c| format!("col {c}")).collect();
    let rule: Vec<&str> = (0..columns).map(|_| "---").collect();
    let row: Vec<String> = (0..columns).map(|c| format!("{}", c * 10)).collect();
    format!(
        "A table.\n\n| {} |\n| {} |\n| {} |\n",
        header.join(" | "),
        rule.join(" | "),
        row.join(" | ")
    )
}

#[test]
fn a_wide_table_goes_landscape() {
    let wide = table(10);
    let source = to_typst(&input(&wide, &[]));
    assert!(source.contains("flipped: true"), "{source}");
    compile_pdf(&input(&wide, &[])).expect("the wide table compiles");

    let narrow = table(6);
    let source = to_typst(&input(&narrow, &[]));
    assert!(!source.contains("flipped: true"), "{source}");
    assert!(source.contains("table("), "{source}");
}

#[test]
fn no_chart_is_split() {
    let mut markdown = String::from("Prose before the charts.\n\n## Charts\n\n");
    let mut charts = Vec::new();
    for i in 0..5 {
        let path = format!("panels/tall_{i}.yaml");
        markdown.push_str(&format!("- `{path}`: Tall chart {i}\n"));
        charts.push(chart(&path, &format!("Tall chart {i}"), 400, 900));
    }
    let source = to_typst(&input(&markdown, &charts));
    // Every chart is drawn by `bf-chart`, and its one picture is inside a
    // block that does not break.
    assert_eq!(source.matches("#bf-chart(").count(), 5, "{source}");
    let image_lines: Vec<&str> = source.lines().filter(|l| l.contains("image(")).collect();
    assert_eq!(image_lines.len(), 1, "{source}");
    assert!(
        image_lines[0].contains("block(breakable: false"),
        "a chart outside a non-breakable block: {}",
        image_lines[0]
    );
    let pdf = compile_pdf(&input(&markdown, &charts)).expect("the report compiles");
    assert!(pdf.starts_with(b"%PDF"));
    // Each is a page tall, so each has a page.
    let pages = preview_pages(&input(&markdown, &charts), 10.0).expect("the pages draw");
    assert_eq!(pages.len(), 5);
}

/// The number of pages `markdown` with `charts` sets on.
fn page_count(markdown: &str, charts: &[ChartImage]) -> usize {
    preview_pages(&input(markdown, charts), 10.0)
        .expect("the pages draw")
        .len()
}

#[test]
fn a_chart_shrinks_to_fit_the_room_left_before_it_moves() {
    // Prose that fills about half the page, then a chart whose full width
    // would be about 150mm tall: at 70% of its width it fits.
    let prose: String = (0..12)
        .map(|i| format!("Paragraph {i} of prose that takes up a line or two of the page before the chart.\n\n"))
        .collect();
    let markdown = format!("{prose}## Charts\n\n- `panels/a.yaml`: A chart\n");
    let fits = [chart("panels/a.yaml", "A chart", 600, 530)];
    assert_eq!(
        page_count(&markdown, &fits),
        1,
        "a chart that fits at 70% stays"
    );
    // One that would need to be narrower than 70% of its width moves on.
    let tall = [chart("panels/a.yaml", "A chart", 400, 900)];
    assert_eq!(
        page_count(&markdown, &tall),
        2,
        "a chart too tall for the room moves"
    );
}

#[test]
fn a_long_table_breaks_across_pages_and_keeps_its_rows() {
    let mut markdown = String::from("| country | year | value |\n| --- | --- | --- |\n");
    for i in 0..80 {
        markdown.push_str(&format!("| row {i} | {} | {}.5 |\n", 1990 + i, i * 3));
    }
    let source = to_typst(&input(&markdown, &[]));
    assert!(source.contains("table.header("), "{source}");
    assert!(!source.contains("breakable: false)[#set text"), "{source}");
    let text = pdf_text(&compile_pdf(&input(&markdown, &[])).expect("compiles"));
    assert!(text.contains("row 79"), "the last row is missing:\n{text}");
    assert!(page_count(&markdown, &[]) >= 2);
}

#[test]
fn a_wide_table_in_a_list_or_a_quote_stays_portrait() {
    let wide = table(10);
    let in_list: String = wide
        .lines()
        .map(|l| format!("   {l}\n"))
        .collect::<String>();
    let markdown = format!(
        "- An item\n\n{in_list}\n> {}\n",
        wide.lines().collect::<Vec<_>>().join("\n> ")
    );
    let source = to_typst(&input(&markdown, &[]));
    assert!(!source.contains("flipped: true"), "{source}");
    assert_eq!(source.matches("table(").count(), 2, "{source}");
    compile_pdf(&input(&markdown, &[])).expect("a nested wide table compiles");
}

#[test]
fn numbers_in_tables_are_kept_whole() {
    let markdown = "| a | b |\n| --- | --- |\n| x | -1,234.5 |\n";
    let source = to_typst(&input(markdown, &[]));
    assert!(source.contains("#box[\\-1,234\\.5]"), "{source}");
}

#[test]
fn markup_is_escaped_in_headings_list_items_and_table_cells() {
    let markdown = "## Costs # $5 @c [e]\n\n- item # $5 @c [e]\n\n| h # $5 | v |\n| --- | --- |\n| @c [e] | 1 |\n\n> quote # $5 @c\n";
    let text = pdf_text(&compile_pdf(&input(markdown, &[])).expect("compiles"));
    for wanted in [
        "Costs # $5 @c [e]",
        "item # $5 @c [e]",
        "h # $5",
        "@c [e]",
        "quote # $5 @c",
    ] {
        assert!(text.contains(wanted), "{wanted} is missing:\n{text}");
    }
}

#[test]
fn the_report_opens_with_its_title_and_the_pdf_carries_it() {
    let report = ReportInput {
        markdown: "Prose.\n",
        charts: &[],
        notice: None,
        title: Some("CO2 per capita"),
        subtitle: Some("Run finished 16 October 2026"),
    };
    let source = to_typst(&report);
    assert!(
        source.contains("#set document(title: \"CO2 per capita\")"),
        "{source}"
    );
    let pdf = compile_pdf(&report).expect("compiles");
    let text = pdf_text(&pdf);
    assert!(
        text.starts_with("CO2 per capita Run finished 16 October 2026"),
        "{text}"
    );
    let raw = String::from_utf8_lossy(&pdf);
    assert!(raw.contains("CO2 per capita"), "the title metadata");
}

#[test]
fn a_chart_note_is_set_under_its_caption() {
    let markdown = "## Charts\n\n- `panels/a.yaml`: A chart\n";
    let mut charts = [chart("panels/a.yaml", "A chart", 40, 20)];
    charts[0].note = Some("9 series share 8 colours, so colours repeat".to_string());
    let text = pdf_text(&compile_pdf(&input(markdown, &charts)).expect("compiles"));
    assert!(
        text.contains("A chart 9 series share 8 colours, so colours repeat"),
        "{text}"
    );
}

#[test]
fn the_run_section_sets_ids_and_hashes_small() {
    let markdown = "## Run\n\n- Run: `r1`\n- Assets, by content hash:\n  - `table:a`: 4922e56afe92c6acace11ca5305fd72e4e5c86eb6418d2faabb6cd8d58fe2f92\n";
    let source = to_typst(&input(markdown, &[]));
    assert!(source.contains("#bf-id(\"table:a\")"), "{source}");
    assert!(
        source.contains(
            "#bf-id(\"4922e56afe92c6acace11ca5305fd72e4e5c86eb6418d2faabb6cd8d58fe2f92\")"
        ),
        "{source}"
    );
    compile_pdf(&input(markdown, &[])).expect("compiles");
}

#[test]
fn package_imports_are_refused() {
    let markdown = "#import \"@preview/x:0.1.0\": *\n\nAnd prose after it.\n";
    let source = to_typst(&input(markdown, &[]));
    assert!(source.contains("\\#import"), "{source}");
    let pdf = compile_pdf(&input(markdown, &[])).expect("the report compiles");
    let text = pdf_text(&pdf);
    assert!(
        text.contains("#import") && text.contains("@preview/x:0.1.0"),
        "{text}"
    );
}

#[test]
fn a_check_line_fails_the_export() {
    let markdown =
        "## Charts\n\n- `panels/a.yaml`: A chart\n  - Check: a mark reads the column z\n";
    let charts = [chart("panels/a.yaml", "A chart", 40, 20)];
    let err = compile_pdf(&input(markdown, &charts)).expect_err("a Check line fails");
    assert!(err.contains("panels/a.yaml"), "{err}");
    assert!(err.contains("a mark reads the column z"), "{err}");
}

#[test]
fn the_notice_sits_above_the_charts_heading() {
    let markdown = "Prose.\n\n## Charts\n\n- `panels/a.yaml`: A chart\n";
    let charts = [chart("panels/a.yaml", "A chart", 40, 20)];
    let notice = "These charts read the Protocol's tables as they are now. `t1` changed since the run this report describes (`r1`).";
    let report = ReportInput {
        markdown,
        charts: &charts,
        notice: Some(notice),
        title: None,
        subtitle: None,
    };
    let source = to_typst(&report);
    let notice_at = source.find("changed since the run").expect("the notice");
    let heading_at = source.find("Charts").expect("the heading");
    assert!(notice_at < heading_at, "{source}");
    let text = pdf_text(&compile_pdf(&report).expect("compiles"));
    assert!(text.contains("changed since the run"), "{text}");
}

#[test]
fn previews_are_one_png_per_page() {
    let markdown = "Prose.\n";
    let pages = preview_pages(&input(markdown, &[]), 72.0).expect("the pages render");
    assert_eq!(pages.len(), 1);
    let page = image::load_from_memory(&pages[0]).expect("a PNG");
    // A4 at 72 pixels to the inch is 595 by 842 points, one pixel each.
    assert_eq!((page.width(), page.height()), (595, 842));
}
