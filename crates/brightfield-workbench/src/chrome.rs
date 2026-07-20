//! The one drawing file.
//!
//! Every pixel of chrome in the workbench is painted here: the pane frame and
//! its header band, the breadcrumb, the toolbar row, the status rail, the
//! empty state, the focus ring and the selection wash. Nowhere else in the
//! workspace may draw any of those, and the framework helps as far as it can:
//! an [`crate::Item`] is handed a `Ui` whose `max_rect` *and* clip rect both
//! start below the header band, so anything the item paints through that `Ui`
//! is clipped away. That is a clip, not a capability boundary — see
//! [`pane_frame`] for what it does and does not reach.
//!
//! # The rules this file encodes
//!
//! - **One colour boundary.** [`colour`] is the only place a design token
//!   becomes an `egui::Color32`. Everything else composes it. There are no
//!   literal colours in this crate.
//! - **One geometry ladder.** Row heights, control heights, icon sizes, text
//!   size and padding all come from `meridian_design::control::binding(row)`
//!   as a set, so a row's contents cannot drift apart from its height.
//!   Nothing here computes a dimension from a number typed at a call site.
//! - **One name per pane.** A pane whose parent is a tab container has its
//!   title drawn by the strip directly above it, so its header band is
//!   suppressed; anywhere else the header band is the only name the pane has.
//!   That rule is mechanical, and it replaces three hand-written headers that
//!   had drifted into three different treatments of the same idea.
//! - **One accent.** The interaction accent is reserved for interaction —
//!   focus and selection. A pane that wants to say "good" or "warning" says
//!   so with a [`Tone`], and this file decides what that looks like.
//! - **No headings.** All chrome text is the 12px UI size. The two top bars
//!   this crate replaces differed by four pixels because one used a heading
//!   and the other did not, and nobody had decided that.
//!
//! # Not here yet
//!
//! Icon glyphs. [`Icon`] is a name, and the Meridian icon set has not landed
//! in this workspace; [`icon_slot`] therefore reserves the icon's box from
//! the row's binding without painting into it, so layout is already correct
//! and adding the set is a paint change rather than a reflow. The shell-level
//! `top_bar`, the modal card, and the list/grid row primitives land with
//! their first callers rather than being written speculatively here.

use meridian_design::{control, focus, radius, semantic, spacing, typography, Elevation, Rgba};

use crate::item::PaneKey;
use crate::subject::{
    Affordance, Crumb, EmptyState, HideAffordance, Icon, StatusEntry, StatusSide, Subject, Tone,
    ToolbarEntry, ToolbarLocation, Verb,
};
use crate::Mode;

/// The header band's rung. The grid rung rather than the dense one because a
/// header carries an inline control (the dirty marker, and shortly a pane
/// menu), and the dense rung's 18px control misses the pointer-target floor.
const HEADER_ROW: f32 = spacing::ROW_GRID;

// ---------------------------------------------------------------------------
// The colour boundary
// ---------------------------------------------------------------------------

/// The **one** conversion from a Meridian token to an egui colour.
///
/// Tokens are sRGB with straight alpha in 0–1; `Color32` is gamma sRGB with
/// straight alpha in 0–255. Keeping this in one function is what makes "is
/// this colour a token?" answerable by grep.
#[must_use]
pub fn colour(c: Rgba) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(q(c.r), q(c.g), q(c.b), q(c.a))
}

/// What a [`Tone`] looks like.
///
/// [`Tone::Accent`] resolves to the focus border and nothing else, which is
/// the mechanism behind "one accent, reserved for interaction": a pane that
/// wants to draw attention has `Good`, `Warning` and `Critical` available and
/// cannot quietly appropriate the interaction colour for emphasis.
#[must_use]
pub fn tone_colour(tone: Tone, mode: Mode) -> egui::Color32 {
    let sem = semantic(mode.is_dark());
    match tone {
        Tone::Neutral => colour(sem.text.secondary),
        Tone::Accent => colour(sem.borders.focus),
        Tone::Good => colour(meridian_design::viz::STATUS.good),
        Tone::Warning => colour(meridian_design::viz::STATUS.warning),
        Tone::Critical => colour(meridian_design::viz::STATUS.critical),
    }
}

/// The label ink a toggled-on toolbar entry takes.
///
/// Split out because it is the whole of the disabled-toggle rule and a rule
/// worth naming: the on-state is drawn *after* the button, so it is the last
/// thing to touch those pixels and it alone decides whether the control still
/// reads as unavailable.
fn toggle_ink(enabled: bool, mode: Mode) -> Rgba {
    let sem = semantic(mode.is_dark());
    if enabled {
        sem.text.primary
    } else {
        sem.text.disabled
    }
}

fn ui_font() -> egui::FontId {
    egui::FontId::proportional(typography::UI_SIZE)
}

// ---------------------------------------------------------------------------
// The pane frame
// ---------------------------------------------------------------------------

/// Draw a pane's frame and return the `Ui` its item may draw into.
///
/// `header` is the de-duplication rule already decided by the caller: `false`
/// when the pane's parent is a tab container, because the strip above it is
/// already showing the title.
///
/// The returned `Ui`'s `max_rect` **and** its clip rect are both the content
/// rect *below* the header band and inside the panel padding. Two different
/// strengths of guarantee follow, and it is worth being precise about which
/// is which:
///
/// - **Enforced.** Anything painted through the returned `Ui` — `ui.painter()`,
///   every widget added to it, every nested `Ui` derived from it — is clipped
///   to the content rect and cannot land in the header band. `max_rect` alone
///   would not do this: `Ui::new_child` clones the parent's painter and leaves
///   its clip rect untouched (`egui::Ui::new_child`), so `max_rect` seeds
///   layout only. The [`Ui::shrink_clip_rect`](egui::Ui::shrink_clip_rect)
///   below is what makes the statement true.
/// - **Not enforced.** An item that reaches around its `Ui` — `egui::Area`,
///   `egui::Window`, `ctx.layer_painter`, anything that takes a fresh layer
///   from the `Context` — is not clipped by this and can paint anywhere on
///   screen. egui offers no way to withhold that from a `&mut Ui` holder.
///   Against *that* the rule is a review rule, backed by the fact that a pane
///   has no reason to want one and by there being no [`Subject`] field that
///   could ask for it.
///
/// So: a pane cannot draw a header by accident, and the contract tests pin
/// that. A pane that sets out to draw one through a new layer can, and the
/// answer to that is code review, not the type system.
pub fn pane_frame(ui: &mut egui::Ui, subject: &Subject, header: bool, mode: Mode) -> egui::Ui {
    let sem = semantic(mode.is_dark());
    let outer = ui.max_rect();

    ui.painter()
        .rect_filled(outer, radius::PANEL, colour(sem.surfaces.raised));

    let mut content = outer;
    if header {
        let band = egui::Rect::from_min_size(
            outer.min,
            egui::vec2(outer.width(), control::binding(HEADER_ROW).row),
        );
        header_band(ui, band, subject, mode);
        content.min.y += band.height();
    }

    if Elevation::Raised.hairline() {
        ui.painter().rect_stroke(
            outer,
            radius::PANEL,
            egui::Stroke::new(1.0, colour(sem.borders.subtle)),
            egui::StrokeKind::Inside,
        );
    }

    // The focus ring is painted inside the pane, so its bleed is reserved
    // here rather than being clipped away at the moment it matters.
    let pad = spacing::PANEL_PADDING.max(focus::RING_BLEED);
    let content = content.shrink(pad);

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(*ui.layout()),
    );
    // `max_rect` seeds the `Placer` and nothing else — `new_child` clones the
    // parent's painter, clip rect included, so without this line a pane could
    // paint straight over the header band with `ui.painter()`. Shrinking the
    // clip is what turns the doc comment above into a measured fact.
    child.shrink_clip_rect(content);
    child
}

/// The pane header: icon, title, dirty marker.
///
/// Private, with exactly one call site. Three hand-written variants of this —
/// one with a leading space and a separator, one strong inside a frame with a
/// margin the others lacked, one with a trailing explanatory sentence — is
/// what the workbench exists to collapse.
fn header_band(ui: &egui::Ui, rect: egui::Rect, subject: &Subject, mode: Mode) {
    let sem = semantic(mode.is_dark());
    let b = control::binding(HEADER_ROW);
    let painter = ui.painter();

    painter.rect_filled(
        rect,
        radius::outer(radius::PANEL, 0.0),
        colour(sem.surfaces.header),
    );

    let mut x = rect.left() + b.pad_x;
    x += icon_slot(ui, rect, x, subject.icon, b.icon, mode);

    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &subject.title,
        ui_font(),
        colour(sem.text.primary),
    );

    if subject.dirty == crate::subject::Dirty::Edited {
        let r = egui::pos2(rect.right() - b.pad_x - b.icon / 2.0, rect.center().y);
        painter.circle_filled(r, b.icon / 4.0, colour(sem.borders.focus));
    }

    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, colour(sem.borders.divider)),
    );
}

/// Reserve an icon's box and return the horizontal advance it consumed.
///
/// The Meridian icon set has not landed here yet, so this paints nothing and
/// reserves the space the glyph will occupy. Reserving rather than skipping
/// means every surface is already laid out for icons and landing the set is a
/// paint change, not a reflow of every header in the product.
fn icon_slot(
    _ui: &egui::Ui,
    _rect: egui::Rect,
    _x: f32,
    _icon: Icon,
    size: f32,
    _mode: Mode,
) -> f32 {
    size + spacing::ICON_LABEL_GAP
}

/// A pane whose item is missing — a layout referenced something this build
/// cannot construct.
///
/// Visible and named rather than blank: a silent empty pane is
/// indistinguishable from a bug in the pane itself, and this one has a
/// specific cause worth reporting.
pub fn orphan_pane(ui: &mut egui::Ui, key: PaneKey, mode: Mode) {
    let sem = semantic(mode.is_dark());
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, radius::PANEL, colour(sem.surfaces.sunken));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("no item registered for {key}"),
        ui_font(),
        colour(sem.text.muted),
    );
}

// ---------------------------------------------------------------------------
// Focus and selection — two functions, not five treatments
// ---------------------------------------------------------------------------

/// The focus ring.
///
/// egui 0.35 folds `has_focus()` into the same visuals bucket as *pressed*,
/// so a focused-but-not-pressed control is indistinguishable from an idle one
/// unless the ring is painted deliberately. That is why this exists rather
/// than the framework's own treatment being used.
pub fn focus_ring(ui: &egui::Ui, rect: egui::Rect, mode: Mode) {
    let sem = semantic(mode.is_dark());
    ui.painter().rect_stroke(
        rect.shrink(focus::RING_OFFSET),
        focus::ring_radius(radius::CONTROL),
        egui::Stroke::new(focus::RING_WIDTH, colour(sem.borders.focus)),
        egui::StrokeKind::Inside,
    );
}

/// The selection wash.
///
/// One treatment for "this is selected", replacing five: a hand-rolled accent
/// outline at a radius nothing else used, the framework's own wash with an
/// ink swap layered on top, a text marker with an ink swap and no wash at
/// all, and the selection restated as a breadcrumb hop.
pub fn selection_wash(ui: &egui::Ui, rect: egui::Rect, mode: Mode) {
    let sem = semantic(mode.is_dark());
    let painter = ui.painter();
    painter.rect_filled(rect, radius::CHIP, colour(sem.rows.selected_background));
    painter.rect_stroke(
        rect,
        radius::CHIP,
        egui::Stroke::new(1.0, colour(sem.rows.selected_border)),
        egui::StrokeKind::Inside,
    );
}

// ---------------------------------------------------------------------------
// Breadcrumb, toolbar, status rail
// ---------------------------------------------------------------------------

/// The breadcrumb trail, most general hop first.
pub fn breadcrumb(ui: &mut egui::Ui, crumbs: &[Crumb], mode: Mode) {
    if crumbs.is_empty() {
        return;
    }
    let sem = semantic(mode.is_dark());
    let last = crumbs.len() - 1;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing::ICON_LABEL_GAP;
        for (i, crumb) in crumbs.iter().enumerate() {
            let ink = if i == last {
                sem.text.primary
            } else {
                sem.text.secondary
            };
            ui.label(
                egui::RichText::new(&crumb.label)
                    .font(ui_font())
                    .color(colour(ink)),
            );
            if i != last {
                ui.label(
                    egui::RichText::new("›")
                        .font(ui_font())
                        .color(colour(sem.text.muted)),
                );
            }
        }
    });
}

/// What a toolbar row did this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolbarDrawn {
    /// The ids actually drawn, in draw order. The test hook behind the
    /// assertion that a [`ToolbarLocation::Hidden`] entry is declared but
    /// never painted.
    pub drawn: Vec<&'static str>,
    /// The verbs the user activated.
    pub activated: Vec<Verb>,
}

/// The toolbar row: leading group, spacer, trailing group.
///
/// [`ToolbarLocation::Overflow`] entries are collected but not yet drawn —
/// the overflow affordance lands with the first row long enough to need one,
/// and until then they are reported in neither `drawn` nor `activated`, which
/// the contract test pins.
pub fn toolbar_row(ui: &mut egui::Ui, entries: &[ToolbarEntry], mode: Mode) -> ToolbarDrawn {
    let mut out = ToolbarDrawn::default();
    let b = control::binding(HEADER_ROW);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing::CONTROL_GAP;
        for entry in entries
            .iter()
            .filter(|e| e.location == ToolbarLocation::Leading)
        {
            draw_toolbar_entry(ui, entry, b, mode, &mut out);
        }
        let trailing: Vec<&ToolbarEntry> = entries
            .iter()
            .filter(|e| e.location == ToolbarLocation::Trailing)
            .collect();
        if !trailing.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for entry in trailing.iter().rev() {
                    draw_toolbar_entry(ui, entry, b, mode, &mut out);
                }
            });
        }
    });
    out
}

fn draw_toolbar_entry(
    ui: &mut egui::Ui,
    entry: &ToolbarEntry,
    b: control::Binding,
    mode: Mode,
    out: &mut ToolbarDrawn,
) {
    let sem = semantic(mode.is_dark());
    out.drawn.push(entry.id);

    let text = egui::RichText::new(&entry.label).font(ui_font());
    let button = egui::Button::new(text)
        .corner_radius(radius::CONTROL)
        .min_size(egui::vec2(0.0, b.control));
    let response = ui.add_enabled(entry.enabled, button);

    if entry.on {
        // A disabled toggle still has to say it is on, but it must not come
        // back looking available. `add_enabled(false, …)` greys the button and
        // painting the full selection treatment on top put it straight back —
        // a wash at selected strength and a label at primary ink is exactly
        // what an *enabled* toggle looks like. So the on-state steps down with
        // the control: outline instead of wash, disabled ink instead of
        // primary.
        if entry.enabled {
            selection_wash(ui, response.rect, mode);
        } else {
            ui.painter().rect_stroke(
                response.rect,
                radius::CHIP,
                egui::Stroke::new(1.0, colour(sem.borders.subtle)),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            response.rect.center(),
            egui::Align2::CENTER_CENTER,
            &entry.label,
            ui_font(),
            colour(toggle_ink(entry.enabled, mode)),
        );
    }

    let response = match &entry.tooltip {
        // The keystroke is appended here, from the registry, so a rebinding
        // can never leave a tooltip claiming a key that no longer works.
        Some(tip) => match entry.verb.keys() {
            Some(keys) => response.on_hover_text(format!("{tip}  ({keys})")),
            None => response.on_hover_text(tip.clone()),
        },
        None => response,
    };

    if response.clicked() {
        out.activated.push(entry.verb);
    }
}

/// What a status rail did this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusDrawn {
    /// The ids drawn, in draw order.
    pub drawn: Vec<&'static str>,
    /// The verbs the user activated by dismissing an entry.
    pub dismissed: Vec<Verb>,
}

/// The status rail: leading entries at the left, trailing at the right.
pub fn status_rail(ui: &mut egui::Ui, entries: &[StatusEntry], mode: Mode) -> StatusDrawn {
    let sem = semantic(mode.is_dark());
    let mut out = StatusDrawn::default();
    let rect = ui.max_rect();
    ui.painter().rect_filled(
        rect,
        radius::NONE,
        colour(sem.containers.status_bar_background),
    );

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing::CONTROL_GAP;
        for entry in entries.iter().filter(|e| e.side == StatusSide::Leading) {
            draw_status_entry(ui, entry, mode, &mut out);
        }
        let trailing: Vec<&StatusEntry> = entries
            .iter()
            .filter(|e| e.side == StatusSide::Trailing)
            .collect();
        if !trailing.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for entry in trailing.iter().rev() {
                    draw_status_entry(ui, entry, mode, &mut out);
                }
            });
        }
    });
    out
}

fn draw_status_entry(ui: &mut egui::Ui, entry: &StatusEntry, mode: Mode, out: &mut StatusDrawn) {
    out.drawn.push(entry.id);
    let label = ui.label(
        egui::RichText::new(&entry.text)
            .font(ui_font())
            .color(tone_colour(entry.tone, mode)),
    );
    if let HideAffordance::Verb(verb) = entry.hide {
        // A dismissable entry says so by being clickable, and the keystroke
        // that also clears it comes from the registry.
        let hint = verb
            .keys()
            .map_or_else(|| "dismiss".to_string(), |k| format!("dismiss  ({k})"));
        if label
            .interact(egui::Sense::click())
            .on_hover_text(hint)
            .clicked()
        {
            out.dismissed.push(verb);
        }
    }
}

// ---------------------------------------------------------------------------
// The empty state
// ---------------------------------------------------------------------------

/// What an empty state did this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmptyStateDrawn {
    /// Set when the user took the resolving action.
    pub activated: Option<Verb>,
}

/// Paint an empty state, centred in the pane.
///
/// The shell calls this *instead of* the item's own draw, which is what makes
/// an empty state impossible to forget: it is not a branch a pane author has
/// to remember to write.
pub fn empty_state(ui: &mut egui::Ui, empty: &EmptyState, mode: Mode) -> EmptyStateDrawn {
    let sem = semantic(mode.is_dark());
    let mut out = EmptyStateDrawn::default();
    ui.vertical_centered(|ui| {
        ui.add_space(spacing::SECTION_GAP);
        ui.label(
            egui::RichText::new(&empty.headline)
                .font(ui_font())
                .color(colour(sem.text.primary)),
        );
        ui.add_space(spacing::SPACE_3);
        ui.label(
            egui::RichText::new(&empty.body)
                .font(ui_font())
                .color(colour(sem.text.muted)),
        );
        if let Some(next) = &empty.next {
            ui.add_space(spacing::CONTROL_GAP);
            if affordance_button(ui, next).clicked() {
                out.activated = Some(next.verb);
            }
        }
    });
    out
}

fn affordance_button(ui: &mut egui::Ui, next: &Affordance) -> egui::Response {
    let b = control::binding(spacing::ROW_GRID);
    // The keystroke is rendered from the registry, never stored on the
    // affordance — one source of truth, so a rebinding cannot make the label
    // lie.
    let label = next
        .verb
        .keys()
        .map_or_else(|| next.label.clone(), |k| format!("{}  {k}", next.label));
    ui.add(
        egui::Button::new(egui::RichText::new(label).font(ui_font()))
            .corner_radius(radius::CONTROL)
            .min_size(egui::vec2(0.0, b.control)),
    )
}
