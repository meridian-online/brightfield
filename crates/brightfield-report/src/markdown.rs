//! Markdown to Typst markup.
//!
//! Every character of the report's own text is escaped, so nothing an agent
//! writes can call a Typst function, import a package or read a file: what the
//! Typst source does is only what this module writes around that text.

use std::collections::BTreeMap;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::{ChartImage, ListedChart, ReportedRun};

/// The heading of the section arc writes each chart's list item into.
const CHARTS_HEADING: &str = "Charts";

/// The heading of the section arc writes the run's id, time and hashes into.
const RUN_HEADING: &str = "Run";

/// Where a chart list item's panel file lives, relative to the Protocol.
const PANELS_PREFIX: &str = "panels/";

/// The word arc starts a panel's finding with, under the panel's list item.
const CHECK_PREFIX: &str = "Check:";

/// A table with this many columns or more is set at 8pt rather than 9pt.
const SMALL_TABLE_COLUMNS: usize = 7;

/// A table with this many columns or more is set on a landscape page of its
/// own, when it stands at the top level of the report.
const LANDSCAPE_TABLE_COLUMNS: usize = 10;

/// The fewest hexadecimal digits a word in the Run section has to hold to be
/// set as a content hash: arc writes SHA-256 hashes, 64 digits long.
const HASH_DIGITS: usize = 32;

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
            Event::Text(t) | Event::Code(t) => text.push_str(t),
            Event::InlineHtml(t) if !t.trim_start().starts_with("<!--") => text.push_str(t),
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }
    text
}

/// The level and text of the heading `events[i]` starts, when it starts one.
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
///
/// This reads what arc's `charts` function in its `report.rs` writes, at the
/// arcform rev the workspace pins: under `## Charts`, one item per panel as
/// `` - `panels/<file>`: <title> ``, the title left off when the panel has
/// none, and under it one `  - Check: <finding>` line per finding.
/// `arc_writes_what_the_parsers_read` in the shell's `tests/export_pdf.rs`
/// holds this against a report arc writes.
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

/// When the run finished and the tables it made, from the `## Run` section of
/// `markdown`.
///
/// This reads what arc's `run_section` function in its `report.rs` writes, at
/// the arcform rev the workspace pins: `## Run` last, then `` - Run: `<id>` ``,
/// `- Finished: <time>`, and under `- Assets, by content hash:` one
/// `` - `<kind>:<name>`: <hash> `` line per asset, `not measured` standing in
/// for a hash arc could not take. Only `table:` assets are kept. The run id is
/// not read here: arc exports it as `arc::spec::report_run`, and the caller
/// asks arc. Lines are split as arc splits them, so CRLF endings read the same.
/// `arc_writes_what_the_parsers_read` in the shell's `tests/export_pdf.rs`
/// holds this against a report arc writes.
pub(crate) fn reported_run(markdown: &str) -> ReportedRun {
    // The Run section is arc's and comes last, so the last such heading is its
    // own even when the prose quotes one.
    let mut section: Vec<&str> = markdown
        .lines()
        .rev()
        .take_while(|line| *line != "## Run")
        .collect();
    let found = section.len() < markdown.lines().count();
    section.reverse();
    if !found {
        section.clear();
    }
    let mut run = ReportedRun {
        finished: None,
        tables: BTreeMap::new(),
    };
    for line in section {
        let line = line.trim();
        if let Some(at) = line.strip_prefix("- Finished: ") {
            run.finished = Some(at.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("- `table:") {
            if let Some((name, hash)) = rest.split_once("`:") {
                let hash = hash.trim();
                let hash = (!hash.is_empty() && hash.chars().all(|c| c.is_ascii_hexdigit()))
                    .then(|| hash.to_string());
                run.tables.insert(name.to_string(), hash);
            }
        }
    }
    run
}

/// The text of a block of HTML: comments left out whole, each tag left out,
/// and runs of white space made one space.
fn html_text(html: &str) -> String {
    // A `<` opens a tag only before a letter, `/` or `!`; any other is text,
    // as in `x < y`.
    let opens_tag = |s: &str| {
        let mut chars = s.chars();
        chars.next() == Some('<')
            && chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!')
    };
    let mut text = String::new();
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |at| &after[at + 3..]);
            text.push(' ');
        } else if opens_tag(rest) {
            let name: String = rest[1..]
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_ascii_lowercase();
            rest = rest.find('>').map_or("", |at| &rest[at + 1..]);
            // What a script or a style holds is not text to read.
            if name == "script" || name == "style" {
                let close = format!("</{name}");
                rest = rest
                    .to_ascii_lowercase()
                    .find(&close)
                    .map_or("", |at| &rest[at..]);
                rest = rest.find('>').map_or("", |at| &rest[at + 1..]);
            }
            text.push(' ');
        } else {
            let at = rest
                .char_indices()
                .skip(1)
                .find(|&(at, _)| opens_tag(&rest[at..]))
                .map_or(rest.len(), |(at, _)| at);
            text.push_str(&decode_entities(&rest[..at]));
            rest = &rest[at..];
        }
    }
    // ASCII white space only, so a decoded `&nbsp;` stays unbreakable.
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` with the HTML character references a report is likely to hold
/// written as the characters they stand for; any other is left as written.
fn decode_entities(text: &str) -> String {
    const ENTITIES: [(&str, &str); 6] = [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", "\u{a0}"),
        ("&amp;", "&"),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match ENTITIES.iter().find(|(name, _)| rest.starts_with(name)) {
            Some((name, ch)) => {
                out.push_str(ch);
                rest = &rest[name.len()..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `text` escaped for Typst markup, so each character prints as itself.
/// `previous` is the character the markup written so far ends with, so a
/// digit and a full stop split across two pieces of text are still caught.
pub(crate) fn escape_markup(text: &str, previous: Option<char>) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut previous = previous.unwrap_or(' ');
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

/// A word that is a content hash: hexadecimal digits, at least [`HASH_DIGITS`].
fn is_hash(word: &str) -> bool {
    word.len() >= HASH_DIGITS && word.chars().all(|c| c.is_ascii_hexdigit())
}

/// One table cell: its markup and its plain text.
struct Cell {
    markup: String,
    text: String,
}

/// The section of the report a writer is in, by its level-2 heading.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Other,
    Charts,
    Run,
}

/// The writer: walks the events and appends Typst markup.
struct Writer<'c> {
    out: String,
    charts: &'c [ChartImage],
    notice: Option<String>,
    section: Section,
    /// Whether this writer's markup goes inside a container of another's: a
    /// table cell or a list item. A page cannot be configured there.
    contained: bool,
    /// The lists, items and quotes open in this writer.
    depth: usize,
    /// For each link open, whether a `#link` was written for it.
    links: Vec<bool>,
}

impl Writer<'_> {
    /// The markup for `events`, written by a contained writer in this one's
    /// section.
    fn nested(&self, events: &[Event]) -> String {
        let mut inner = Writer {
            out: String::new(),
            charts: self.charts,
            notice: None,
            section: self.section,
            contained: true,
            depth: 0,
            links: Vec::new(),
        };
        inner.write(events, false);
        inner.out.trim().to_string()
    }

    /// Whether a page can be configured where the writer is now.
    fn at_top_level(&self) -> bool {
        !self.contained && self.depth == 0
    }

    fn push_text(&mut self, text: &str) {
        let escaped = escape_markup(text, self.out.chars().last());
        self.out.push_str(&escaped);
    }

    fn write(&mut self, events: &[Event], top_level: bool) {
        // A report with no Charts section still says the tables changed, first.
        if top_level && !has_charts_heading(events) {
            self.write_notice();
        }
        let mut i = 0;
        while i < events.len() {
            if top_level {
                if let Some((level, text)) = heading_at(events, i) {
                    if level <= HeadingLevel::H2 {
                        self.section = match (level, text.as_str()) {
                            (HeadingLevel::H2, CHARTS_HEADING) => Section::Charts,
                            (HeadingLevel::H2, RUN_HEADING) => Section::Run,
                            _ => Section::Other,
                        };
                        if self.section == Section::Charts {
                            self.write_notice();
                        }
                    }
                }
            }
            match &events[i] {
                Event::Start(Tag::List(first)) if self.section == Section::Charts => {
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
                Event::Start(Tag::HtmlBlock) => {
                    // A block of HTML keeps its text and loses its tags, so
                    // `<p align="center">Source: ONS</p>` prints its source;
                    // a comment in it is a note to the file's reader and is
                    // left out whole.
                    let end = matching_end(events, i);
                    let html: String = events[i + 1..end]
                        .iter()
                        .filter_map(|e| match e {
                            Event::Html(t) | Event::Text(t) => Some(t.as_ref()),
                            _ => None,
                        })
                        .collect();
                    let text = html_text(&html);
                    if !text.is_empty() {
                        self.push_text(&text);
                        self.out.push_str("\n\n");
                    }
                    i = end + 1;
                    continue;
                }
                Event::Start(Tag::Image { .. }) => {
                    // A picture the report links to is not fetched: its alt
                    // text stands in for it.
                    let end = matching_end(events, i);
                    self.push_text(&plain_text(&events[i + 1..end]));
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

    /// Code: in the Run section an id, set small; elsewhere inline code.
    fn write_code(&mut self, code: &str) {
        let call = if self.section == Section::Run {
            "bf-id"
        } else {
            "raw"
        };
        self.out
            .push_str(&format!("#{call}({});", string_literal(code)));
    }

    /// Text in the Run section: a content hash at its end is set as an id.
    fn write_run_text(&mut self, text: &str) {
        let trimmed = text.trim_end();
        // Just past the last character that is not a hex digit, which may be
        // several bytes long (an em dash, an accented letter).
        let start = trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| !c.is_ascii_hexdigit())
            .map_or(0, |(at, c)| at + c.len_utf8());
        if is_hash(&trimmed[start..]) {
            self.push_text(&trimmed[..start]);
            self.write_code(&trimmed[start..]);
            self.push_text(&text[trimmed.len()..]);
        } else {
            self.push_text(text);
        }
    }

    fn write_event(&mut self, event: &Event) {
        match event {
            Event::Start(tag) => {
                if matches!(tag, Tag::List(_) | Tag::Item | Tag::BlockQuote(_)) {
                    self.depth += 1;
                }
                let out = &mut self.out;
                match tag {
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
                        // Typst refuses an empty URL, and a link to nowhere is
                        // its text.
                        let linked = !dest_url.trim().is_empty();
                        if linked {
                            out.push_str(&format!("#link({})[", string_literal(dest_url)));
                        }
                        self.links.push(linked);
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                if matches!(tag, TagEnd::List(_) | TagEnd::Item | TagEnd::BlockQuote(_)) {
                    self.depth = self.depth.saturating_sub(1);
                }
                let out = &mut self.out;
                match tag {
                    TagEnd::Paragraph => out.push_str("\n\n"),
                    TagEnd::Heading(_) | TagEnd::BlockQuote(_) => out.push_str("];\n\n"),
                    TagEnd::List(_) => out.push_str(");\n\n"),
                    TagEnd::Item => {
                        while out.ends_with(['\n', ' ']) {
                            out.pop();
                        }
                        out.push_str("],\n");
                    }
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                        out.push_str("];");
                    }
                    TagEnd::Link if self.links.pop().unwrap_or(false) => {
                        out.push_str("];");
                    }
                    _ => {}
                }
            }
            Event::Text(t) if self.section == Section::Run => self.write_run_text(t),
            Event::Text(t) => self.push_text(t),
            // A comment is a note to the file's reader, and a block of HTML
            // such as `<details>` has nothing to draw in a PDF: both are left
            // out. Any other inline HTML is printed as written, since prose
            // like `a <b> c` reads as a tag to CommonMark but is meant as text.
            Event::InlineHtml(t) if !t.trim_start().starts_with("<!--") => self.push_text(t),
            Event::InlineHtml(_) | Event::Html(_) => {}
            Event::Code(code) => self.write_code(code),
            Event::SoftBreak => self.out.push(' '),
            Event::HardBreak => self.out.push_str("#linebreak();"),
            Event::Rule => {
                self.out
                    .push_str("#line(length: 100%, stroke: 0.5pt + rgb(\"#c8ccd0\"));\n\n");
            }
            Event::TaskListMarker(done) => {
                self.out
                    .push_str(if *done { "\\[x\\] " } else { "\\[ \\] " });
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

    /// The chart at `index`, through the preamble's `bf-chart`: its title as a
    /// caption, any note under it, and the picture, in a block that does not
    /// break.
    fn write_chart(&mut self, index: usize) {
        let chart = &self.charts[index];
        let ratio = f64::from(chart.height_px) / f64::from(chart.width_px.max(1));
        let note = chart.note.as_deref().map_or_else(
            || "none".to_string(),
            |note| format!("[{}]", escape_markup(note, None)),
        );
        self.out.push_str(&format!(
            "#v(1.4em, weak: true)\n#bf-chart([{}], {note}, {}, {ratio:.6});\n\n",
            escape_markup(&chart.title, None),
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
        // Typst refuses to configure a page inside a container, so a wide
        // table in a list item or a quote stays on the portrait page.
        let landscape = columns >= LANDSCAPE_TABLE_COLUMNS && self.at_top_level();

        let mut t = String::new();
        if landscape {
            t.push_str("#page(flipped: true)[\n");
        }
        // The table breaks across pages, and `table.header` repeats its header
        // on each.
        t.push_str(&format!(
            "#block(above: 1.2em, below: 1.4em)[#set text(size: {size})\n#table(\n  columns: {columns},\n  align: ({}),\n  stroke: (x, y) => (bottom: if y == 0 {{ 0.7pt + rgb(\"#5f6870\") }} else {{ 0.4pt + rgb(\"#dde0e3\") }}),\n  inset: (x: 5pt, y: 3.5pt),\n",
            align.join(", ")
        ));
        for (r, row) in rows.iter().enumerate() {
            let cells: Vec<String> = (0..columns)
                .map(|c| {
                    let Some(cell) = row.get(c) else {
                        return "[]".to_string();
                    };
                    if r == 0 {
                        return format!("[#text(weight: \"semibold\")[{}]]", cell.markup);
                    }
                    // A number is boxed so it never wraps; a number in a column
                    // of words is right-aligned on its own.
                    if is_numeric(&cell.text) {
                        let boxed = format!("#box[{}]", cell.markup);
                        if align[c] == "left" {
                            format!("table.cell(align: right)[{boxed}]")
                        } else {
                            format!("[{boxed}]")
                        }
                    } else {
                        format!("[{}]", cell.markup)
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
        section: Section::Other,
        contained: false,
        depth: 0,
        links: Vec::new(),
    };
    writer.write(&events(markdown), true);
    writer.out
}

/// `text` escaped for markup with nothing before it.
pub(crate) fn escape(text: &str) -> String {
    escape_markup(text, None)
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

    #[test]
    fn the_run_section_is_read() {
        let report = "Prose naming - `table:fake`: 00\n\n## Run\n\n- Run: `r1`\n- Finished: 2026-10-16T21:15:10Z\n- Assets, by content hash:\n  - `table:a`: 4922e56a\n  - `file:data/x.csv`: 7f78\n  - `table:b`: not measured\n";
        let run = reported_run(report);
        assert_eq!(run.finished.as_deref(), Some("2026-10-16T21:15:10Z"));
        assert_eq!(run.tables.len(), 2, "{:?}", run.tables);
        assert_eq!(run.tables["a"].as_deref(), Some("4922e56a"));
        assert_eq!(run.tables["b"], None);
    }

    #[test]
    fn the_run_section_reads_the_same_with_crlf_endings() {
        let report = "## Charts\r\n\r\n## Run\r\n\r\n- Run: `r1`\r\n- Finished: 2026-10-16T21:15:10Z\r\n- Assets, by content hash:\r\n  - `table:a`: 4922e56a\r\n";
        let run = reported_run(report);
        assert_eq!(run.finished.as_deref(), Some("2026-10-16T21:15:10Z"));
        assert_eq!(run.tables["a"].as_deref(), Some("4922e56a"));
    }

    #[test]
    fn run_text_ending_in_a_multi_byte_character_is_set_as_written() {
        for (markdown, kept) in [
            ("## Run\n\nEach table \u{2014} `x`\n", "Each table \u{2014}"),
            ("## Run\n\ncaf\u{e9}\n", "caf\u{e9}"),
            (
                "## Run\n\nsaid \u{201c}done\u{201d}\n",
                "\u{201c}done\u{201d}",
            ),
        ] {
            let out = body(markdown, &[], None);
            assert!(out.contains(kept), "{kept} in {out}");
        }
    }

    #[test]
    fn an_html_block_keeps_a_bare_less_than_and_its_entities_and_drops_scripts() {
        assert_eq!(html_text("<p>x < y</p>"), "x < y");
        assert_eq!(
            html_text("<p>A &amp; B &lt;5&gt; &quot;q&quot; it&#39;s&nbsp;so &copy;</p>"),
            "A & B <5> \"q\" it's\u{a0}so &copy;"
        );
        assert_eq!(
            html_text(
                "<div>Shown<script>var a = '<b>';</script><STYLE>p { x: 1 }</STYLE> too</div>"
            ),
            "Shown too"
        );
    }

    #[test]
    fn html_comments_and_html_blocks_are_left_out() {
        let out = body(
            "Before <!-- an inline note --> after.\n\n<!--\na block note\n-->\n\n\
             <details>\n<summary>More</summary>\n</details>\n\n\
             <p align=\"center\">Source: ONS <!-- hidden --></p>\n\nLast.\n",
            &[],
            None,
        );
        for gone in ["note", "<", "details", "summary", "align", "hidden"] {
            assert!(!out.contains(gone), "{gone} in {out}");
        }
        for kept in ["Before", "after.", "More", "Source: ONS", "Last."] {
            assert!(out.contains(kept), "{kept} not in {out}");
        }
    }

    #[test]
    fn a_digit_and_a_stop_split_across_text_are_escaped() {
        // `1` then `.`: two text events, as a Markdown escape splits them.
        let out = body("1\\. not a list\n", &[], None);
        assert!(out.starts_with("1\\."), "{out}");
    }

    #[test]
    fn an_empty_link_is_its_text() {
        let out = body(
            "[see the table]() and [a site](https://example.org)\n",
            &[],
            None,
        );
        assert!(!out.contains("#link(\"\")"), "{out}");
        assert!(out.contains("see the table"), "{out}");
        assert!(out.contains("#link(\"https://example.org\")"), "{out}");
    }
}
