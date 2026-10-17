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
    }
}

fn input<'a>(markdown: &'a str, charts: &'a [ChartImage]) -> ReportInput<'a> {
    ReportInput {
        markdown,
        charts,
        notice: None,
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
    assert_eq!(source.matches("image(").count(), 2, "{source}");
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
    let image_lines: Vec<&str> = source.lines().filter(|l| l.contains("image(")).collect();
    assert_eq!(image_lines.len(), 5, "{source}");
    for line in image_lines {
        assert!(
            line.contains("block(breakable: false"),
            "a chart outside a non-breakable block: {line}"
        );
    }
    let pdf = compile_pdf(&input(&markdown, &charts)).expect("the report compiles");
    assert!(pdf.starts_with(b"%PDF"));
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
