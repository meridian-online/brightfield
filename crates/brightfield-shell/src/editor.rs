//! The YAML spec editor: a code surface and an [`Item`] on the shell contract.
//!
//! # What this file is, and is not
//!
//! This is a **spec editor**, not an IDE buffer. Its job is a few hundred
//! lines of YAML: seed from the file, highlight it legibly, take the
//! author's keystrokes, and write the buffer's bytes back atomically. There
//! is no rope, no incremental parse, no multi-cursor and no language server,
//! and none is wanted — the deleted gpui shell carried a whole
//! tree-sitter-yaml grammar to do what [`yaml_layout_job`] does in a
//! line lexer.
//!
//! # The one decision recorded here: `TextEdit` + layouter, not `egui_code_editor`
//!
//! Both candidates were evaluated before a line of this was written.
//! `egui_code_editor 0.3.7` tracks egui 0.35, so version lock was *not* the
//! deciding factor. What decided it:
//!
//! - **Token discipline.** Its `ColorTheme` is a struct of `&'static str`
//!   hex colours; theming it from the Meridian scales would mean leaking
//!   runtime-formatted hex strings — colour laundered through a string. The
//!   layouter closure takes `Color32` straight from the token boundary.
//! - **YAML is structural, not keyword-shaped.** Its `Syntax` type is
//!   keyword-set based (keywords/types/special), which cannot say "the thing
//!   before an unquoted `: ` is a key" — the one distinction YAML
//!   highlighting exists to make. The line lexer here makes it directly.
//! - **The read-only treatment must be ours.** egui models *disabled* as a
//!   global alpha multiply; read-only is a different state (see below) and
//!   needs the surface, not a wrapper, to own its look.
//! - **No new dependency.** The gutter, the status line and the highlight
//!   are this file either way; the crate would have bought only the
//!   `TextEdit` wrapper, plus its `opener` dependency.
//!
//! # Read-only is a treatment, not a disablement
//!
//! A read-only surface (an executed protocol run's YAML; a spec the process
//! cannot write) must stay *legible* and *copyable*: full-contrast ink, live
//! selection, the platform copy. egui's disabled state is a global alpha
//! fade that greys the text and kills interaction — the opposite on both
//! counts. So [`Access::ReadOnly`] renders through an immutable text buffer
//! (`&str` — selection and copy work, typing is a no-op) and takes its own
//! themeable treatment from [`treatment`]: the *reading* surface
//! (`surfaces.raised`) where the editable surface takes the recessed *input
//! well* (`surfaces.sunken`), plus a `read-only` badge in the status line.
//! The syntax ink is identical in both — by construction:
//! [`yaml_layout_job`] does not take an [`Access`].
//!
//! # One copy convention
//!
//! Select, then the platform copy (cmd-c / ctrl-c) — the `TextEdit`'s own.
//! It works identically in read-only, because read-only keeps interaction.
//! There is no bespoke copy button and no second convention; a copy verb in
//! a toolbar would need a registered command, and the keyboard registry is
//! the place that decision gets made, not this file.
//!
//! # Saving writes the buffer's bytes
//!
//! The whole save route is `brightfield_model::spec_save`: the two-writer
//! guard over three texts, then a temp+rename of the buffer **verbatim**.
//! No spec type is constructed anywhere in this file, nothing re-serialises,
//! and no canonicalisation happens as a side effect of saving — a save
//! cannot destroy a comment or reorder a key, because it never marshals.
//! Canonicalisation, if it ever exists, is an explicit command.
//!
//! # What the editor does NOT draw
//!
//! Its tab title, dirty dot, breadcrumb, toolbar, status rail and empty
//! state are all declared on its [`Subject`] and drawn by the shell chrome —
//! zero editor-local chrome. The save acknowledgement is a transient status
//! entry raised the moment the write lands, so it is on the very next
//! painted frame (the speed rule: feedback lands next frame); the reload
//! indicator appears only once a pending reload has been outstanding past
//! the 100 ms honesty line ([`reload_spinner_due`]).

use std::fs;
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use brightfield_keys::BindingContext;
use brightfield_model::spec_save::{self, SaveDecision, SaveOutcome};
use brightfield_workbench::registry::Slot;
use brightfield_workbench::{
    Crumb, Dirty, EmptyState, Handled, HideAffordance, Icon, Item, ItemCtx, ItemId, ItemSpec,
    StatusEntry, StatusSide, Subject, Tone, Verb,
};
use meridian_design::{control, radius, scales, semantic, spacing, Rgba};

use crate::app::ChartDoc;
use crate::design::{to_color32, Mode};

// ---------------------------------------------------------------------------
// Identity and registry glue
// ---------------------------------------------------------------------------

/// The editor pane's stable id.
pub const EDITOR: ItemId = ItemId::new("spec-editor");

/// The editor's icon — a name, resolved to paint when the icon set lands.
const ICON_EDITOR: Icon = Icon("code");

/// The editor's registry entry, appended to the chart view's registry
/// ([`crate::app::chart_registry`]): a centre tab beside the chart canvas,
/// toggled by the canvas↔editor focus verb.
///
/// Kept here rather than inline in the registry so the append is one line
/// and the editor's whole declaration lives with the editor.
#[must_use]
pub fn editor_spec() -> ItemSpec<ChartDoc> {
    ItemSpec {
        id: EDITOR,
        slot: Slot::CentreTab,
        toggle: Some(Verb::new("toggle-focus")),
        make: || Box::new(EditorPane::new()),
    }
}

// ---------------------------------------------------------------------------
// Timing rules — the speed guideline, as data
// ---------------------------------------------------------------------------

/// How long the save acknowledgement stays on the status rail, in ms.
pub const SAVE_ACK_MS: u32 = 2000;

/// The honesty line from the speed guideline: work that can exceed ~100 ms
/// must show its truth. A reload resolved faster than this shows nothing —
/// a spinner for a sub-perceptual read is decoration, not honesty.
pub const RELOAD_SPINNER_HONESTY_MS: u128 = 100;

/// Whether a reload that has been pending for `elapsed_ms` owes the user a
/// progress cue. The one place the honesty line is compared against.
#[must_use]
pub fn reload_spinner_due(elapsed_ms: u128) -> bool {
    elapsed_ms >= RELOAD_SPINNER_HONESTY_MS
}

/// How often the editor stats its file for external changes. The same
/// cadence the app-side mtime watcher used.
const DISK_POLL: Duration = Duration::from_millis(300);

// ---------------------------------------------------------------------------
// Syntax ink — YAML highlighting from the token scales
// ---------------------------------------------------------------------------

/// What a highlighted span *is*. Paint-free, so the lexer is testable
/// without a font or a colour anywhere near it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// Unclassified text, whitespace included.
    Plain,
    /// A mapping key (the segment before an unquoted `: `).
    Key,
    /// A quoted scalar.
    Str,
    /// A number, boolean, or null.
    Literal,
    /// A `#` comment, to end of line.
    Comment,
    /// Structural punctuation: `:` `-` `[` `]` `{` `}` `,` `|` `>`.
    Punct,
    /// An anchor, alias or tag: `&name` `*name` `!tag`.
    Anchor,
}

/// The syntax palette, resolved per mode from the Meridian scales — never
/// from literals. Text-contrast steps (step 11 of the 12-step scales) for
/// the coloured tokens; the semantic text roles for the rest.
///
/// Deliberately independent of [`Access`]: read-only text is full-contrast
/// text, which is most of what distinguishes it from disabled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntaxInks {
    /// Unclassified text.
    pub plain: Rgba,
    /// Mapping keys — the brand accent's text step.
    pub key: Rgba,
    /// Quoted scalars.
    pub string: Rgba,
    /// Numbers, booleans, null.
    pub literal: Rgba,
    /// Comments.
    pub comment: Rgba,
    /// Structural punctuation.
    pub punct: Rgba,
    /// Anchors, aliases, tags.
    pub anchor: Rgba,
}

/// The palette for one mode.
#[must_use]
pub fn syntax_inks(dark: bool) -> SyntaxInks {
    let sem = semantic(dark);
    let (maritime, green, amber, blue) = if dark {
        (
            scales::MARITIME_DARK,
            scales::GREEN_DARK,
            scales::AMBER_DARK,
            scales::BLUE_DARK,
        )
    } else {
        (
            scales::MARITIME_LIGHT,
            scales::GREEN_LIGHT,
            scales::AMBER_LIGHT,
            scales::BLUE_LIGHT,
        )
    };
    SyntaxInks {
        plain: sem.text.primary,
        key: maritime[10],
        string: green[10],
        literal: amber[10],
        comment: sem.text.muted,
        punct: sem.text.secondary,
        anchor: blue[10],
    }
}

impl SyntaxInks {
    /// The ink for one token kind.
    #[must_use]
    pub fn for_kind(&self, kind: TokenKind) -> Rgba {
        match kind {
            TokenKind::Plain => self.plain,
            TokenKind::Key => self.key,
            TokenKind::Str => self.string,
            TokenKind::Literal => self.literal,
            TokenKind::Comment => self.comment,
            TokenKind::Punct => self.punct,
            TokenKind::Anchor => self.anchor,
        }
    }
}

// ---------------------------------------------------------------------------
// The line lexer
// ---------------------------------------------------------------------------

/// Classify one line of YAML into contiguous byte ranges covering the whole
/// line.
///
/// Line-based on purpose: a spec editor needs keys, values, strings and
/// comments to read at a glance, not a conforming YAML parse. The known
/// approximation is block scalars — the lines *under* a `|`/`>` are lexed as
/// YAML rather than as opaque text. Accepted: specs use block scalars for
/// SQL, where the lexer's strings/numbers/punctuation read fine.
#[must_use]
pub fn highlight_line(line: &str) -> Vec<(TokenKind, Range<usize>)> {
    let mut spans: Vec<(TokenKind, Range<usize>)> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;

    // Leading whitespace.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i > 0 {
        spans.push((TokenKind::Plain, 0..i));
    }

    // Sequence markers: any run of `- ` after the indent.
    while i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b' ' {
        spans.push((TokenKind::Punct, i..i + 1));
        spans.push((TokenKind::Plain, i + 1..i + 2));
        i += 2;
    }
    // A bare `-` ending the line is a marker too.
    if i + 1 == bytes.len() && bytes[i] == b'-' {
        spans.push((TokenKind::Punct, i..i + 1));
        return spans;
    }

    if i >= bytes.len() {
        return spans;
    }

    // A whole-line comment.
    if bytes[i] == b'#' {
        spans.push((TokenKind::Comment, i..bytes.len()));
        return spans;
    }

    // Key detection: the segment up to the first unquoted `:` that is
    // followed by whitespace or ends the line. `:` inside a value (a URL,
    // a time) is not followed by a space, so it does not split.
    if let Some(colon) = key_colon(line, i) {
        let key = &line[i..colon];
        if is_plain_key(key) {
            spans.push((TokenKind::Key, i..colon));
            spans.push((TokenKind::Punct, colon..colon + 1));
            i = colon + 1;
        }
    }

    lex_value(line, i, &mut spans);
    spans
}

/// The byte index of the key-separating `:` at nesting depth zero, if this
/// segment of the line has one: an unquoted `:` followed by whitespace or
/// end of line.
fn key_colon(line: &str, from: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = from;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'\'' | b'"' => quote = Some(b),
                b'#' => return None,
                b':' if i + 1 >= bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t' => {
                    return Some(i)
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Whether a candidate key segment reads as a key: non-empty, and free of
/// flow-structure characters (a `{a: 1}` inline mapping is lexed as a
/// value, not half-claimed as a key).
fn is_plain_key(segment: &str) -> bool {
    !segment.is_empty() && !segment.contains(['{', '[', ',', '}', ']'])
}

/// Lex the value region of a line — everything after the key's `:` (or the
/// whole content when the line has no key).
fn lex_value(line: &str, mut i: usize, spans: &mut Vec<(TokenKind, Range<usize>)>) {
    let bytes = line.as_bytes();
    let mut plain_start = i;
    let flush_plain = |spans: &mut Vec<(TokenKind, Range<usize>)>, upto: usize, from: usize| {
        if from < upto {
            spans.push((TokenKind::Plain, from..upto));
        }
    };

    while i < bytes.len() {
        let b = bytes[i];
        let at_boundary = i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t';
        match b {
            b'#' if at_boundary => {
                flush_plain(spans, i, plain_start);
                spans.push((TokenKind::Comment, i..bytes.len()));
                return;
            }
            b'\'' | b'"' => {
                flush_plain(spans, i, plain_start);
                let close = bytes[i + 1..]
                    .iter()
                    .position(|&c| c == b)
                    .map_or(bytes.len(), |p| i + 1 + p + 1);
                spans.push((TokenKind::Str, i..close));
                i = close;
                plain_start = i;
            }
            b'&' | b'*' | b'!' if at_boundary => {
                flush_plain(spans, i, plain_start);
                let mut j = i + 1;
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'-')
                {
                    j += 1;
                }
                spans.push((TokenKind::Anchor, i..j));
                i = j;
                plain_start = i;
            }
            b'[' | b']' | b'{' | b'}' | b',' | b'|' | b'>' | b':' => {
                flush_plain(spans, i, plain_start);
                spans.push((TokenKind::Punct, i..i + 1));
                i += 1;
                plain_start = i;
            }
            _ if at_boundary && starts_number(bytes, i) => {
                flush_plain(spans, i, plain_start);
                let mut j = i + 1;
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric()
                        || bytes[j] == b'.'
                        || bytes[j] == b'_'
                        || bytes[j] == b'+'
                        || bytes[j] == b'-')
                {
                    j += 1;
                }
                spans.push((TokenKind::Literal, i..j));
                i = j;
                plain_start = i;
            }
            _ if at_boundary && b.is_ascii_alphabetic() => {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                    j += 1;
                }
                let word = &line[i..j];
                let is_literal = matches!(
                    word,
                    "true" | "false" | "null" | "True" | "False" | "Null" | "yes" | "no"
                );
                if is_literal {
                    flush_plain(spans, i, plain_start);
                    spans.push((TokenKind::Literal, i..j));
                    plain_start = j;
                }
                i = j;
            }
            b'~' if at_boundary => {
                flush_plain(spans, i, plain_start);
                spans.push((TokenKind::Literal, i..i + 1));
                i += 1;
                plain_start = i;
            }
            _ => i += 1,
        }
    }
    flush_plain(spans, bytes.len(), plain_start);
}

/// Whether a numeric literal starts at `i`.
fn starts_number(bytes: &[u8], i: usize) -> bool {
    match bytes[i] {
        b'0'..=b'9' => true,
        b'-' | b'+' | b'.' => bytes.get(i + 1).is_some_and(u8::is_ascii_digit),
        _ => false,
    }
}

/// The whole buffer as a highlighted `LayoutJob`, wrap disabled — a code
/// surface scrolls sideways, it does not reflow, which is also what keeps
/// the gutter's line numbering exact (one galley row per buffer line).
///
/// Takes the text, the mode and a font — deliberately **not** an
/// [`Access`]: read-only text wears the same ink as editable text.
#[must_use]
pub fn yaml_layout_job(text: &str, dark: bool, font: egui::FontId) -> egui::text::LayoutJob {
    let inks = syntax_inks(dark);
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            append(&mut job, "\n", inks.plain, &font);
        }
        first = false;
        for (kind, range) in highlight_line(line) {
            append(&mut job, &line[range], inks.for_kind(kind), &font);
        }
    }
    job
}

fn append(job: &mut egui::text::LayoutJob, text: &str, ink: Rgba, font: &egui::FontId) {
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: to_color32(ink),
            ..Default::default()
        },
    );
}

// ---------------------------------------------------------------------------
// CodeSurface
// ---------------------------------------------------------------------------

/// Whether the surface takes edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The buffer is the author's to change.
    Editable,
    /// Selection and copy stay live; typing is a no-op. A *treatment*, not
    /// egui's disabled fade — see the module docs.
    ReadOnly,
}

/// The themeable treatment one [`Access`] wears. Resolved from the semantic
/// surface tokens, per mode — never hard-coded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceTreatment {
    /// The surface behind the text: the recessed input well when editable,
    /// the flat reading surface when read-only.
    pub background: Rgba,
    /// The badge the status line shows, when the access needs saying.
    pub badge: Option<&'static str>,
}

/// The treatment for one access in one mode.
#[must_use]
pub fn treatment(access: Access, dark: bool) -> SurfaceTreatment {
    let sem = semantic(dark);
    match access {
        Access::Editable => SurfaceTreatment {
            background: sem.surfaces.sunken,
            badge: None,
        },
        Access::ReadOnly => SurfaceTreatment {
            background: sem.surfaces.raised,
            badge: Some("read-only"),
        },
    }
}

/// What one frame of the surface reported.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeSurfaceOutput {
    /// Whether the buffer changed this frame. Always `false` in read-only.
    pub changed: bool,
    /// The primary cursor as 1-based `(line, column)`, when the surface has
    /// one.
    pub cursor: Option<(usize, usize)>,
}

/// Draw the code surface: gutter with line numbers, the highlighted text
/// edit, and a status line — filling the `Ui` it is given.
///
/// The text edit itself is `egui::TextEdit::multiline` with a layouter over
/// [`yaml_layout_job`]; read-only goes through an immutable buffer so
/// selection and the platform copy keep working (see the module docs for
/// both decisions).
pub fn code_surface(
    ui: &mut egui::Ui,
    id_salt: &str,
    text: &mut String,
    access: Access,
    mode: Mode,
) -> CodeSurfaceOutput {
    let dark = mode.is_dark();
    let treat = treatment(access, dark);
    let status_h = control::binding(spacing::ROW_DENSE).row;

    let avail = ui.available_rect_before_wrap();
    let editor_rect = egui::Rect::from_min_max(
        avail.min,
        egui::pos2(avail.max.x, (avail.max.y - status_h).max(avail.min.y)),
    );
    let status_rect =
        egui::Rect::from_min_max(egui::pos2(avail.min.x, editor_rect.max.y), avail.max);

    ui.painter()
        .rect_filled(editor_rect, radius::CONTROL, to_color32(treat.background));

    let inner_rect = editor_rect.shrink(spacing::SPACE_3);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(*ui.layout()),
    );
    inner.shrink_clip_rect(inner_rect);

    let out = egui::ScrollArea::both()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show(&mut inner, |ui| {
            gutter_and_edit(ui, id_salt, text, access, dark)
        })
        .inner;

    status_line(ui, status_rect, text, &treat, &out, mode);
    ui.advance_cursor_after_rect(avail);
    out
}

/// The gutter and the text edit, side by side inside the scroll area.
fn gutter_and_edit(
    ui: &mut egui::Ui,
    id_salt: &str,
    text: &mut String,
    access: Access,
    dark: bool,
) -> CodeSurfaceOutput {
    let sem = semantic(dark);
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
    let n_lines = text.split('\n').count().max(1);

    let mut result = CodeSurfaceOutput::default();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = spacing::CONTROL_GAP;

        // The gutter: right-aligned line numbers in muted ink, one per
        // buffer line — exact because the layouter disables wrapping, so a
        // galley row IS a buffer line.
        let digits = n_lines.to_string().len();
        let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
        #[allow(clippy::cast_precision_loss)] // line counts are far below f32's mantissa
        let gutter_size = egui::vec2(digits as f32 * char_w, row_h * n_lines as f32);
        let (gutter_rect, _) = ui.allocate_exact_size(gutter_size, egui::Sense::hover());
        let painter = ui.painter();
        for i in 0..n_lines {
            #[allow(clippy::cast_precision_loss)]
            let y = gutter_rect.min.y + (i as f32 + 0.5) * row_h;
            painter.text(
                egui::pos2(gutter_rect.max.x, y),
                egui::Align2::RIGHT_CENTER,
                i + 1,
                font.clone(),
                to_color32(sem.text.muted),
            );
        }

        // The text edit, with the highlight layouter. Zero margin so its
        // first galley row shares the gutter's first row baseline.
        let hl_font = font.clone();
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
            let job = yaml_layout_job(buf.as_str(), dark, hl_font.clone());
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let output = match access {
            Access::Editable => egui::TextEdit::multiline(text)
                .font(font.clone())
                .margin(egui::Margin::ZERO)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .lock_focus(true)
                .id_salt((id_salt, "edit"))
                .layouter(&mut layouter)
                .show(ui),
            Access::ReadOnly => {
                // An immutable `TextBuffer`: interaction stays on (cursor,
                // selection, platform copy), mutation is a structural no-op.
                // Not `add_enabled(false, …)`, which is the alpha fade this
                // treatment exists to not be.
                let mut view: &str = text.as_str();
                egui::TextEdit::multiline(&mut view)
                    .font(font.clone())
                    .margin(egui::Margin::ZERO)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .lock_focus(true)
                    .id_salt((id_salt, "edit"))
                    .layouter(&mut layouter)
                    .show(ui)
            }
        };

        result.changed = access == Access::Editable && output.response.changed();
        result.cursor = output
            .cursor_range
            .map(|range| line_col(text, range.primary.index.0));
    });
    result
}

/// The 1-based `(line, column)` of a char index in `text`.
#[must_use]
pub fn line_col(text: &str, char_index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in text.chars().enumerate() {
        if i >= char_index {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// The status line under the surface: cursor position (or line count) at the
/// left; the language and, when the access needs saying, its badge at the
/// right.
fn status_line(
    ui: &egui::Ui,
    rect: egui::Rect,
    text: &str,
    treat: &SurfaceTreatment,
    out: &CodeSurfaceOutput,
    mode: Mode,
) {
    let sem = semantic(mode.is_dark());
    let b = control::binding(spacing::ROW_DENSE);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        radius::NONE,
        to_color32(sem.containers.status_bar_background),
    );

    let font = egui::FontId::proportional(meridian_design::typography::UI_SIZE);
    let left = match out.cursor {
        Some((line, col)) => format!("Ln {line}, Col {col}"),
        None => {
            let n = text.split('\n').count().max(1);
            format!("{n} lines")
        }
    };
    painter.text(
        egui::pos2(rect.left() + b.pad_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        left,
        font.clone(),
        to_color32(sem.text.muted),
    );

    let mut x = rect.right() - b.pad_x;
    if let Some(badge) = treat.badge {
        let drawn = painter.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            badge,
            font.clone(),
            to_color32(sem.text.secondary),
        );
        x = drawn.left() - spacing::CONTROL_GAP;
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "YAML",
        font,
        to_color32(sem.text.muted),
    );
}

// ---------------------------------------------------------------------------
// The editor's file state
// ---------------------------------------------------------------------------

/// What a save attempt did — the pane's own record, for the wiring and the
/// tests. The write itself is `spec_save`'s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveReport {
    /// The buffer's bytes were written atomically.
    Written,
    /// Buffer and file already agree; nothing was touched.
    Unchanged,
    /// The file changed on disk since the last sync: nothing was written,
    /// and the NEXT save (with no intervening edit) will force the write.
    ConflictArmed,
    /// The armed force fired: the buffer's bytes overwrote the external
    /// change.
    ForcedWrite,
    /// The editor never successfully seeded from this file and refuses to
    /// overwrite contents it never held.
    RefusedUnseeded,
    /// No file is open.
    NoFile,
    /// The write itself failed.
    Failed(String),
}

/// One open spec file: the path, the live buffer, and the last text the
/// buffer was synced with (seed or successful save) — the three texts
/// `spec_save::decide_save` arbitrates.
struct OpenFile {
    path: PathBuf,
    buffer: String,
    last_synced: Option<String>,
    /// The mtime last observed by the poll, for cheap change detection.
    seen_mtime: Option<SystemTime>,
}

// ---------------------------------------------------------------------------
// EditorPane — the Item
// ---------------------------------------------------------------------------

/// The YAML spec editor as an [`Item`] in the chart view.
///
/// Holds only editor-local state, per the workbench's no-document-handle
/// rule. Which file to open comes from the document: [`ChartDoc::spec_path`]
/// carries the file the dashboard was composed from, and the pane's first
/// drawn frame hands it to [`EditorPane::open_file`]. A fresh pane over a
/// document with no file behind it answers a real empty state.
pub struct EditorPane {
    file: Option<OpenFile>,
    /// When the last successful save landed — drives the transient ack.
    ack: Option<Instant>,
    /// A standing warning: an external conflict, a seed failure, or a
    /// failed write. Cleared by the next successful save or buffer edit.
    warning: Option<String>,
    /// Whether the next save overrides an external conflict. Armed by the
    /// first conflicted save, disarmed by any buffer edit or reseed.
    force_armed: bool,
    /// When a detected external change has not yet been adopted — the
    /// honesty-line clock for the reload indicator.
    reload_pending_since: Option<Instant>,
    /// The last disk poll, throttling the stat to [`DISK_POLL`].
    last_poll: Option<Instant>,
    /// The last drawn cursor, echoed while the pane redraws.
    cursor: Option<(usize, usize)>,
}

impl Default for EditorPane {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorPane {
    /// A pane with no file open.
    #[must_use]
    pub fn new() -> Self {
        Self {
            file: None,
            ack: None,
            warning: None,
            force_armed: false,
            reload_pending_since: None,
            last_poll: None,
            cursor: None,
        }
    }

    /// Open `path`, seeding the buffer from disk.
    ///
    /// A failed read still opens the file — with an empty, **unseeded**
    /// buffer that `spec_save::decide_save` will refuse to save, and a
    /// standing warning saying why.
    pub fn open_file(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        let seen_mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        match fs::read_to_string(&path) {
            Ok(text) => {
                self.warning = None;
                self.file = Some(OpenFile {
                    path,
                    buffer: text.clone(),
                    last_synced: Some(text),
                    seen_mtime,
                });
            }
            Err(e) => {
                self.warning = Some(format!("could not read the spec: {e}"));
                self.file = Some(OpenFile {
                    path,
                    buffer: String::new(),
                    last_synced: None,
                    seen_mtime,
                });
            }
        }
        self.ack = None;
        self.force_armed = false;
        self.reload_pending_since = None;
        self.cursor = None;
    }

    /// The open file's path, if any.
    #[must_use]
    pub fn path(&self) -> Option<&std::path::Path> {
        self.file.as_ref().map(|f| f.path.as_path())
    }

    /// The live buffer, if a file is open.
    #[must_use]
    pub fn buffer(&self) -> Option<&str> {
        self.file.as_ref().map(|f| f.buffer.as_str())
    }

    /// The live buffer, mutably — the same string the surface edits, so the
    /// shell (or a test) can stand in for typing.
    pub fn buffer_mut(&mut self) -> Option<&mut String> {
        self.file.as_mut().map(|f| &mut f.buffer)
    }

    /// Record that the buffer was edited: an edit supersedes any armed
    /// conflict force and any stale warning — the conflict decision is
    /// re-made from the new text at the next save, so two saves with an
    /// edit between them are not "consecutive" and do not force.
    ///
    /// The surface's own edits route through here from [`Item::ui`]; a
    /// caller standing in for typing (the wiring, a test) calls it after
    /// mutating [`Self::buffer_mut`].
    pub fn note_buffer_edited(&mut self) {
        self.force_armed = false;
        self.warning = None;
    }

    /// Whether there is anything a save could write: a seeded buffer that
    /// differs from its last sync. An unseeded editor answers `false` — it
    /// refuses to save (see `spec_save::decide_save`), and a Save control
    /// that looks pressable while every press refuses would be a lie.
    #[must_use]
    pub fn can_save(&self) -> bool {
        self.file.as_ref().is_some_and(|f| {
            f.last_synced
                .as_ref()
                .is_some_and(|synced| *synced != f.buffer)
        })
    }

    /// Whether the buffer holds edits its file does not.
    #[must_use]
    fn dirty(&self) -> Dirty {
        let edited = self.file.as_ref().is_some_and(|f| match &f.last_synced {
            Some(synced) => *synced != f.buffer,
            None => !f.buffer.is_empty(),
        });
        if edited {
            Dirty::Edited
        } else {
            Dirty::Clean
        }
    }

    /// Run the save route now: decide, then write the buffer's **bytes**.
    ///
    /// The route is `decide_save` → `save_spec_atomic` and nothing else — no
    /// spec value exists in this function, so nothing can re-serialise or
    /// canonicalise on the way to disk.
    pub fn save_now(&mut self) -> SaveReport {
        let Some(file) = &mut self.file else {
            return SaveReport::NoFile;
        };
        let file_now = fs::read_to_string(&file.path).ok();
        let decision = spec_save::decide_save(
            &file.buffer,
            file_now.as_deref(),
            file.last_synced.as_deref(),
        );
        let report = match decision {
            SaveDecision::Write => match spec_save::save_spec_atomic(&file.buffer, &file.path) {
                Ok(SaveOutcome::Written | SaveOutcome::Unchanged) => SaveReport::Written,
                Err(e) => SaveReport::Failed(e.to_string()),
            },
            SaveDecision::Unchanged => SaveReport::Unchanged,
            SaveDecision::ExternalConflict => {
                if self.force_armed {
                    match spec_save::save_spec_atomic(&file.buffer, &file.path) {
                        Ok(_) => SaveReport::ForcedWrite,
                        Err(e) => SaveReport::Failed(e.to_string()),
                    }
                } else {
                    SaveReport::ConflictArmed
                }
            }
            SaveDecision::RefuseUnseeded => SaveReport::RefusedUnseeded,
        };

        match &report {
            SaveReport::Written | SaveReport::ForcedWrite => {
                file.last_synced = Some(file.buffer.clone());
                file.seen_mtime = fs::metadata(&file.path).and_then(|m| m.modified()).ok();
                self.ack = Some(Instant::now());
                self.warning = None;
                self.force_armed = false;
                self.reload_pending_since = None;
            }
            SaveReport::Unchanged => {
                // Buffer and file agree; adopt the agreement as the sync
                // point so a later edit diffs against what is really there.
                file.last_synced = Some(file.buffer.clone());
                self.warning = None;
                self.force_armed = false;
            }
            SaveReport::ConflictArmed => {
                self.force_armed = true;
                self.warning =
                    Some("file changed on disk — save again to overwrite it".to_string());
            }
            SaveReport::RefusedUnseeded => {
                self.warning = Some(
                    "the spec could not be read at open — refusing to save over it".to_string(),
                );
            }
            SaveReport::Failed(e) => {
                self.warning = Some(format!("save failed: {e}"));
            }
            SaveReport::NoFile => {}
        }
        report
    }

    /// Check the file for external changes, unthrottled — the poll the
    /// frame loop rate-limits through the private `poll_disk`.
    ///
    /// A pristine buffer follows the disk; a dirty buffer is left alone
    /// (the save route's conflict guard owns that case). A change that
    /// cannot be read yet leaves the reload *pending*, which is what the
    /// honesty-line indicator reads.
    pub fn poll_disk_now(&mut self) {
        let Some(file) = &mut self.file else {
            return;
        };
        let mtime = fs::metadata(&file.path).and_then(|m| m.modified()).ok();
        if mtime == file.seen_mtime && self.reload_pending_since.is_none() {
            return;
        }
        file.seen_mtime = mtime;
        if self.reload_pending_since.is_none() {
            self.reload_pending_since = Some(Instant::now());
        }
        match fs::read_to_string(&file.path) {
            Ok(now) => {
                if spec_save::should_reseed(&file.buffer, file.last_synced.as_deref(), &now) {
                    file.buffer = now.clone();
                    file.last_synced = Some(now);
                    self.force_armed = false;
                    self.warning = None;
                    self.cursor = None;
                }
                // Adopted or deliberately kept: either way the reload is
                // resolved, not pending.
                self.reload_pending_since = None;
            }
            Err(_) => {
                // Mid-write or momentarily unreadable: stays pending; the
                // indicator appears only past the honesty line.
            }
        }
    }

    /// The throttled poll the frame loop calls.
    fn poll_disk(&mut self) {
        let due = self.last_poll.is_none_or(|at| at.elapsed() >= DISK_POLL)
            || self.reload_pending_since.is_some();
        if due {
            self.last_poll = Some(Instant::now());
            self.poll_disk_now();
        }
    }

    /// Whether the save acknowledgement is still live.
    fn ack_live(&self) -> bool {
        self.ack
            .is_some_and(|at| at.elapsed() < Duration::from_millis(u64::from(SAVE_ACK_MS)))
    }

    /// Whether the reload indicator is owed — pending past the honesty line.
    fn reload_indicator_due(&self) -> bool {
        self.reload_pending_since
            .is_some_and(|since| reload_spinner_due(since.elapsed().as_millis()))
    }
}

impl Item<ChartDoc> for EditorPane {
    fn item_id(&self) -> ItemId {
        EDITOR
    }

    /// No file open and none to open — nothing to edit.
    ///
    /// Two truths answer it: the pane's own buffer, and the document's
    /// [`ChartDoc::spec_path`] — the file the dashboard was composed from,
    /// which this pane opens on its first drawn frame ([`Self::ui`]). A
    /// composed dashboard with a file behind it is *not* an empty editor,
    /// even before the tab has ever been focused; without the second check
    /// the shell would draw the empty state instead of calling `ui`, and the
    /// open would never happen.
    fn empty_state(&self, doc: &ChartDoc) -> Option<EmptyState> {
        if self.file.is_some() || doc.spec_path.is_some() {
            return None;
        }
        Some(EmptyState::new(
            ICON_EDITOR,
            "No spec open",
            "Name a spec on the command line to edit it here. Edits save \
             back to the file exactly as typed.",
        ))
    }

    fn describe(&self, _doc: &ChartDoc) -> Subject {
        let title = self
            .file
            .as_ref()
            .and_then(|f| f.path.file_name())
            .map_or_else(
                || "Editor".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
        let mut subject =
            Subject::new(title, ICON_EDITOR, BindingContext::Editor).dirty(self.dirty());

        if let Some(file) = &self.file {
            let mut crumbs = Vec::new();
            if let Some(dir) = file.path.parent().and_then(|p| p.file_name()) {
                crumbs.push(Crumb::new(dir.to_string_lossy().into_owned()));
            }
            if let Some(name) = file.path.file_name() {
                crumbs.push(Crumb::new(name.to_string_lossy().into_owned()));
            }
            subject = subject.with_breadcrumb(crumbs);

            let mut save = brightfield_workbench::ToolbarEntry::button(
                "editor-save",
                "Save",
                Verb::new("save-spec"),
            )
            .at(brightfield_workbench::ToolbarLocation::Trailing)
            .enabled(self.can_save());
            save.tooltip = Some("Write the buffer to the spec file".to_string());
            subject = subject.with_toolbar(save);

            if self.ack_live() {
                subject = subject.with_status(StatusEntry {
                    id: "editor-saved",
                    side: StatusSide::Leading,
                    text: "saved".to_string(),
                    tone: Tone::Good,
                    hide: HideAffordance::Transient { ms: SAVE_ACK_MS },
                });
            }
            if let Some(warning) = &self.warning {
                subject = subject.with_status(StatusEntry {
                    id: "editor-warning",
                    side: StatusSide::Leading,
                    text: warning.clone(),
                    tone: Tone::Warning,
                    hide: HideAffordance::WithRail,
                });
            }
            if self.reload_indicator_due() {
                subject = subject.with_status(StatusEntry {
                    id: "editor-reloading",
                    side: StatusSide::Trailing,
                    text: "reloading…".to_string(),
                    tone: Tone::Neutral,
                    hide: HideAffordance::WithRail,
                });
            }
        }
        subject
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // The document carries which spec file composed it; the first drawn
        // frame is where that path becomes an open buffer. Once a file is
        // open the pane keeps it — a start replacing the dashboard clears the
        // document's path, not an editor holding unsaved keystrokes.
        if self.file.is_none() {
            if let Some(path) = &doc.spec_path {
                self.open_file(path.clone());
            }
        }
        self.poll_disk();
        let Some(file) = &mut self.file else {
            // Unreachable through the shell (the empty state draws instead
            // when there is no file and no path to open one from); blank
            // rather than apologetic if reached directly.
            return;
        };

        let out = code_surface(
            ui,
            "spec-editor",
            &mut file.buffer,
            Access::Editable,
            cx.mode,
        );
        if out.changed {
            self.note_buffer_edited();
        }
        if out.cursor.is_some() {
            self.cursor = out.cursor;
        }

        // The save keystroke, editor-scoped exactly as the keyboard registry
        // binds it. Consumed here because the focused text edit owns the
        // keyboard while the author types — the workspace dispatch defers to
        // it — and `perform` stays the verb's route in from everything else.
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            let _ = self.save_now();
        }

        if self.ack_live() || self.reload_pending_since.is_some() {
            // The ack must expire and the pending reload must re-poll
            // without waiting for the next keystroke.
            cx.request_repaint();
        }
    }

    fn perform(&mut self, _doc: &mut ChartDoc, verb: Verb, cx: &mut ItemCtx<'_>) -> Handled {
        if verb.as_str() == "save-spec" {
            let _ = self.save_now();
            cx.request_repaint();
            return Handled::Yes;
        }
        Handled::No
    }
}
