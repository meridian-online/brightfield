//! Markdown to Typst markup.
//!
//! Every character of the report's own text is escaped, so nothing an agent
//! writes can call a Typst function, import a package or read a file: what the
//! Typst source does is only what this module writes around that text.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::{ChartImage, ListedChart};

/// The heading of the section arc writes each chart's list item into.
const CHARTS_HEADING: &str = "Charts";

/// Where a chart list item's panel file lives, relative to the Protocol.
const PANELS_PREFIX: &str = "panels/";

/// The word arc starts a panel's finding with, under the panel's list item.
const CHECK_PREFIX: &str = "Check:";

/// A table with this many columns or more is set at 8pt rather than 9pt.
const SMALL_TABLE_COLUMNS: usize = 7;

/// A table with this many columns or more is set on a landscape page of its own.
const LANDSCAPE_TABLE_COLUMNS: usize = 10;

/// The text width of a portrait page, in millimetres: A4's 210 less the two
/// 20mm side margins the preamble sets.
const TEXT_WIDTH_MM: f64 = 170.0;

/// The tallest a chart is drawn, in millimetres: A4's 297 less the 20mm top and
/// 22mm bottom margins, less room for the chart's caption. A chart that would be
/// taller at the full text width is drawn this tall and narrower, so its
/// non-breakable block still fits on one page.
const MAX_CHART_HEIGHT_MM: f64 = 225.0;

fn events(markdown: &str) -> Vec<Event<'_>> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    Parser::new_ext(markdown, options).collect()
}

/// The index of the event that ends the element starting at `start`.
fn matching_end(events: &[Event], start: usize) -> usize {
    let mut depth = 0usize;
    for (i, event) in events.iter().enumerate().skip(start) {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    events.len().saturating_sub(1)
}

/// The plain text of `events`: their text and code, with breaks as spaces.
fn plain_text(events: &[Event]) -> String {
    let mut text = String::new();
    for event in events {
        match event {
            Event::Text(t) | Event::Code(t) | Event::InlineHtml(t) | Event::Html(t) => {
                text.push_str(t);
            }
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }
    text
}

/// The heading text when `events[i]` starts a level-2 heading.
fn heading_at(events: &[Event], i: usize) -> Option<(HeadingLevel, String)> {
    if let Event::Start(Tag::Heading { level, .. }) = &events[i] {
        let end = matching_end(events, i);
        return Some((*level, plain_text(&events[i + 1..end]).trim().to_string()));
    }
    None
}

/// Whether `events` hold the level-2 Charts heading.
fn has_charts_heading(events: &[Event]) -> bool {
    (0..events.len()).any(|i| {
        heading_at(events, i)
            .is_some_and(|(level, text)| level == HeadingLevel::H2 && text == CHARTS_HEADING)
    })
}

/// The top-level items of the list whose events, start and end included, are
/// `list`, each as the range of its events inside the `Item` tags.
fn list_items(list: &[Event]) -> Vec<std::ops::Range<usize>> {
    let mut items = Vec::new();
    let mut i = 1;
    while i + 1 < list.len() {
        if matches!(list[i], Event::Start(Tag::Item)) {
            let end = matching_end(list, i);
            items.push(i + 1..end);
            i = end + 1;
        } else {
            i += 1;
        }
    }
    items
}

/// The chart an item of the Charts list names, when it is one: its first
/// inline is a code span holding `panels/<file>`, and the text after it,
/// less a leading colon, is the title. The `Check:` lines of any list nested
/// in the item are its findings.
fn chart_item(item: &[Event]) -> Option<ListedChart> {
    let mut i = 0;
    while matches!(item.get(i), Some(Event::Start(Tag::Paragraph))) {
        i += 1;
    }
    let Some(Event::Code(path)) = item.get(i) else {
        return None;
    };
    if !path.starts_with(PANELS_PREFIX) {
        return None;
    }
    let rest = &item[i + 1..];
    let title_end = rest
        .iter()
        .position(|e| {
            matches!(
                e,
                Event::Start(Tag::List(_)) | Event::End(TagEnd::Paragraph)
            )
        })
        .unwrap_or(rest.len());
    let title = plain_text(&rest[..title_end]);
    let title = title.trim().trim_start_matches(':').trim().to_string();
    let mut checks = Vec::new();
    for (j, event) in rest.iter().enumerate() {
        if matches!(event, Event::Start(Tag::List(_))) {
            let end = matching_end(rest, j);
            for nested in list_items(&rest[j..=end]) {
                let text = plain_text(&rest[j..=end][nested]);
                if let Some(check) = text.trim().strip_prefix(CHECK_PREFIX) {
                    checks.push(check.trim().to_string());
                }
            }
        }
    }
    Some(ListedChart {
        path: path.to_string(),
        title,
        checks,
    })
}

/// The charts the `## Charts` section of `markdown` lists, in order.
pub(crate) fn listed_charts(markdown: &str) -> Vec<ListedChart> {
    let events = events(markdown);
    let mut charts = Vec::new();
    let mut in_charts = false;
    let mut i = 0;
    while i < events.len() {
        if let Some((level, text)) = heading_at(&events, i) {
            if level <= HeadingLevel::H2 {
                in_charts = level == HeadingLevel::H2 && text == CHARTS_HEADING;
            }
            i = matching_end(&events, i) + 1;
            continue;
        }
        if in_charts && matches!(events[i], Event::Start(Tag::List(_))) {
            let end = matching_end(&events, i);
            let list = &events[i..=end];
            charts.extend(
                list_items(list)
                    .into_iter()
                    .filter_map(|item| chart_item(&list[item])),
            );
            i = end + 1;
            continue;
        }
        i += 1;
    }
    charts
}

/// `text` escaped for Typst markup, so each character prints as itself.
pub(crate) fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut previous = ' ';
    for c in text.chars() {
        match c {
            '\\' | '#' | '$' | '*' | '_' | '@' | '<' | '>' | '[' | ']' | '`' | '~' | '/' | '='
            | '+' | '-' => {
                out.push('\\');
                out.push(c);
            }
            // `1.` at the start of a line is a numbered list in Typst markup.
            '.' if previous.is_ascii_digit() => out.push_str("\\."),
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
        previous = c;
    }
    out
}

/// `text` as a Typst string literal, quotes included.
pub(crate) fn string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether a table cell reads as a number: digits with an optional sign,
/// currency symbol, thousands separators, decimal point and trailing percent.
fn is_numeric(cell: &str) -> bool {
    let s = cell.trim();
    let s = s.trim_start_matches(['+', '-', '\u{2212}', '±']);
    let s = s.trim_start_matches(['$', '£', '€']);
    let s = s.strip_suffix('%').unwrap_or(s).trim_end();
    let digits: String = s.chars().filter(|c| !matches!(c, ',' | '_')).collect();
    digits.chars().any(|c| c.is_ascii_digit()) && digits.parse::<f64>().is_ok()
}

/// One table cell: its markup and its plain text.
struct Cell {
    markup: String,
    text: String,
}

/// The writer: walks the events and appends Typst markup.
struct Writer<'c> {
    out: String,
    charts: &'c [ChartImage],
    notice: Option<String>,
}

impl Writer<'_> {
    /// The markup for `events`, written by a writer with the same charts.
    fn nested(&self, events: &[Event]) -> String {
        let mut inner = Writer {
            out: String::new(),
            charts: self.charts,
            notice: None,
        };
        inner.write(events, false);
        inner.out.trim().to_string()
    }

    fn write(&mut self, events: &[Event], top_level: bool) {
        // A report with no Charts section still says the tables changed, first.
        if top_level && !has_charts_heading(events) {
            self.write_notice();
        }
        let mut in_charts = false;
        let mut i = 0;
        while i < events.len() {
            if top_level {
                if let Some((level, text)) = heading_at(events, i) {
                    if level <= HeadingLevel::H2 {
                        in_charts = level == HeadingLevel::H2 && text == CHARTS_HEADING;
                        if in_charts {
                            self.write_notice();
                        }
                    }
                }
            }
            match &events[i] {
                Event::Start(Tag::List(first)) if in_charts => {
                    let end = matching_end(events, i);
                    self.write_chart_list(&events[i..=end], *first);
                    i = end + 1;
                    continue;
                }
                Event::Start(Tag::Table(alignments)) => {
                    let end = matching_end(events, i);
                    self.write_table(&events[i..=end], alignments);
                    i = end + 1;
                    continue;
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let end = matching_end(events, i);
                    let code = plain_text(&events[i + 1..end]);
                    let lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.split_whitespace().next().unwrap_or(""),
                        CodeBlockKind::Indented => "",
                    };
                    self.out.push_str("#raw(block: true, ");
                    if !lang.is_empty() {
                        self.out
                            .push_str(&format!("lang: {}, ", string_literal(lang)));
                    }
                    self.out
                        .push_str(&string_literal(code.trim_end_matches('\n')));
                    self.out.push_str(");\n\n");
                    i = end + 1;
                    continue;
                }
                Event::Start(Tag::Image { .. }) => {
                    // A picture the report links to is not fetched: its alt
                    // text stands in for it.
                    let end = matching_end(events, i);
                    self.out
                        .push_str(&escape_markup(&plain_text(&events[i + 1..end])));
                    i = end + 1;
                    continue;
                }
                event => self.write_event(event),
            }
            i += 1;
        }
    }

    fn write_notice(&mut self) {
        let Some(notice) = self.notice.take() else {
            return;
        };
        let body = self.nested(&events(&notice));
        self.out.push_str(
            "#block(breakable: false, width: 100%, fill: rgb(\"#f1f3f5\"), inset: (x: 10pt, y: 8pt), radius: 3pt)[",
        );
        self.out.push_str(&body);
        self.out.push_str("];\n\n");
    }

    fn write_event(&mut self, event: &Event) {
        let out = &mut self.out;
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    out.push_str(&format!("#heading(level: {})[", *level as usize));
                }
                Tag::BlockQuote(_) => out.push_str("#quote(block: true)["),
                Tag::List(None) => out.push_str("#list(\n"),
                Tag::List(Some(start)) => out.push_str(&format!("#enum(start: {start},\n")),
                Tag::Item => out.push('['),
                Tag::Emphasis => out.push_str("#text(weight: \"medium\")["),
                Tag::Strong => out.push_str("#text(weight: \"bold\")["),
                Tag::Strikethrough => out.push_str("#strike["),
                Tag::Link { dest_url, .. } => {
                    out.push_str(&format!("#link({})[", string_literal(dest_url)));
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => out.push_str("\n\n"),
                TagEnd::Heading(_) | TagEnd::BlockQuote(_) => out.push_str("];\n\n"),
                TagEnd::List(_) => out.push_str(");\n\n"),
                TagEnd::Item => {
                    while out.ends_with(['\n', ' ']) {
                        out.pop();
                    }
                    out.push_str("],\n");
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    out.push_str("];");
                }
                _ => {}
            },
            Event::Text(t) | Event::InlineHtml(t) | Event::Html(t) => {
                out.push_str(&escape_markup(t));
            }
            Event::Code(code) => {
                out.push_str(&format!("#raw({});", string_literal(code)));
            }
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push_str("#linebreak();"),
            Event::Rule => {
                out.push_str("#line(length: 100%, stroke: 0.5pt + rgb(\"#c8ccd0\"));\n\n");
            }
            Event::TaskListMarker(done) => {
                out.push_str(if *done { "\\[x\\] " } else { "\\[ \\] " })
            }
            _ => {}
        }
    }

    /// A list in the Charts section: each chart item becomes its chart, and the
    /// items between charts stay a list.
    fn write_chart_list(&mut self, list: &[Event], first: Option<u64>) {
        let mut plain: Vec<String> = Vec::new();
        for item in list_items(list) {
            let events = &list[item];
            let chart = chart_item(events)
                .and_then(|listed| self.charts.iter().position(|c| c.path == listed.path));
            if let Some(index) = chart {
                self.flush_items(&mut plain, first);
                self.write_chart(index);
            } else {
                plain.push(self.nested(events));
            }
        }
        self.flush_items(&mut plain, first);
    }

    fn flush_items(&mut self, items: &mut Vec<String>, first: Option<u64>) {
        if items.is_empty() {
            return;
        }
        self.out.push_str(&match first {
            Some(start) => format!("#enum(start: {start},\n"),
            None => "#list(\n".to_string(),
        });
        for item in items.drain(..) {
            self.out.push_str(&format!("[{item}],\n"));
        }
        self.out.push_str(");\n\n");
    }

    /// The chart at `index` with its title above it as a caption, in a block
    /// that does not break, on one line of the source.
    fn write_chart(&mut self, index: usize) {
        let chart = &self.charts[index];
        let height_at_width =
            TEXT_WIDTH_MM * f64::from(chart.height_px) / f64::from(chart.width_px.max(1));
        let size = if height_at_width > MAX_CHART_HEIGHT_MM {
            format!("height: {MAX_CHART_HEIGHT_MM}mm")
        } else {
            "width: 100%".to_string()
        };
        self.out.push_str(&format!(
            "#block(breakable: false, width: 100%, above: 1.6em, below: 1.6em)[#text(weight: \"semibold\", size: 10.5pt)[{}] #v(0.5em, weak: true) #align(center, image({}, {size}))];\n\n",
            escape_markup(&chart.title),
            string_literal(&crate::chart_file(index)),
        ));
    }

    fn write_table(&mut self, table: &[Event], alignments: &[Alignment]) {
        // Rows of cells, the header first.
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut i = 1;
        while i + 1 < table.len() {
            match &table[i] {
                Event::Start(Tag::TableHead | Tag::TableRow) => rows.push(Vec::new()),
                Event::Start(Tag::TableCell) => {
                    let end = matching_end(table, i);
                    let cell = &table[i + 1..end];
                    let markup = self.nested(cell);
                    if let Some(row) = rows.last_mut() {
                        row.push(Cell {
                            markup,
                            text: plain_text(cell),
                        });
                    }
                    i = end;
                }
                _ => {}
            }
            i += 1;
        }
        let columns = alignments
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0));
        if columns == 0 {
            return;
        }
        // A column is right-aligned when Markdown says so, or when every cell
        // under its header that holds anything is a number.
        let numeric_column: Vec<bool> = (0..columns)
            .map(|c| {
                let mut body = rows
                    .iter()
                    .skip(1)
                    .filter_map(|r| r.get(c))
                    .filter(|cell| !cell.text.trim().is_empty())
                    .peekable();
                body.peek().is_some() && body.all(|cell| is_numeric(&cell.text))
            })
            .collect();
        let align: Vec<&str> = (0..columns)
            .map(|c| match alignments.get(c) {
                Some(Alignment::Left) => "left",
                Some(Alignment::Center) => "center",
                Some(Alignment::Right) => "right",
                _ if numeric_column[c] => "right",
                _ => "left",
            })
            .collect();
        let size = if columns >= SMALL_TABLE_COLUMNS {
            "8pt"
        } else {
            "9pt"
        };
        let landscape = columns >= LANDSCAPE_TABLE_COLUMNS;

        let mut t = String::new();
        if landscape {
            t.push_str("#page(flipped: true)[\n");
        }
        t.push_str(&format!(
            "#block(breakable: false, above: 1.2em, below: 1.4em)[#set text(size: {size})\n#table(\n  columns: {columns},\n  align: ({}),\n  stroke: (x, y) => (bottom: if y == 0 {{ 0.7pt + rgb(\"#5f6870\") }} else {{ 0.4pt + rgb(\"#dde0e3\") }}),\n  inset: (x: 5pt, y: 3.5pt),\n",
            align.join(", ")
        ));
        for (r, row) in rows.iter().enumerate() {
            let cells: Vec<String> = (0..columns)
                .map(|c| {
                    let Some(cell) = row.get(c) else {
                        return "[]".to_string();
                    };
                    let markup = if r == 0 {
                        format!("#text(weight: \"semibold\")[{}]", cell.markup)
                    } else {
                        cell.markup.clone()
                    };
                    // A number in a column of words is right-aligned on its own.
                    if r > 0 && align[c] == "left" && is_numeric(&cell.text) {
                        format!("table.cell(align: right)[{markup}]")
                    } else {
                        format!("[{markup}]")
                    }
                })
                .collect();
            if r == 0 {
                t.push_str(&format!("  table.header({}),\n", cells.join(", ")));
            } else {
                t.push_str(&format!("  {},\n", cells.join(", ")));
            }
        }
        t.push_str(")];\n");
        if landscape {
            t.push_str("];\n");
        }
        t.push('\n');
        self.out.push_str(&t);
    }
}

/// The body of the report: `markdown` as Typst markup, with each listed chart
/// that `charts` holds drawn in its list item's place and `notice`, when there
/// is one, above the Charts heading.
pub(crate) fn body(markdown: &str, charts: &[ChartImage], notice: Option<&str>) -> String {
    let mut writer = Writer {
        out: String::new(),
        charts,
        notice: notice.map(str::to_string),
    };
    writer.write(&events(markdown), true);
    writer.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_recognised() {
        for yes in ["1", "-56.9", "+302.1%", "1,234", "$5", "−3", "0.67"] {
            assert!(is_numeric(yes), "{yes}");
        }
        for no in ["", "CAN", "2024-10-01", "n/a", "1990 to 2024", "-"] {
            assert!(!is_numeric(no), "{no}");
        }
    }

    #[test]
    fn the_co2_charts_are_listed() {
        let markdown = "## Charts\n\nFindings are checked.\n\n\
                        - `panels/a.yaml`: Change in CO2 (percent)\n\
                        - `panels/b.yaml`: Per-capita CO2\n  - Check: a mark reads the column z\n\n\
                        ## Run\n\n- Run: `r`\n";
        let listed = listed_charts(markdown);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].path, "panels/a.yaml");
        assert_eq!(listed[0].title, "Change in CO2 (percent)");
        assert!(listed[0].checks.is_empty());
        assert_eq!(
            listed[1].checks,
            vec!["a mark reads the column z".to_string()]
        );
    }
}
