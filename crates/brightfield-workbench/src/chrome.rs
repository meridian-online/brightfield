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
//! in this workspace; the private `icon_slot` helper therefore reserves the
//! icon's box from the row's binding without painting into it, so layout is
//! already correct and adding the set is a paint change rather than a reflow.
//! The shell-level `top_bar`, the modal card, and the list/grid row primitives
//! land with their first callers rather than being written speculatively here.

use meridian_design::{control, focus, radius, semantic, spacing, typography, Elevation, Rgba};

use crate::item::PaneKey;
use crate::subject::{
    Action, Affordance, Crumb, EmptyState, HideAffordance, Icon, StatusEntry, StatusSide, Subject,
    Tone, ToolbarEntry, ToolbarLocation, Verb,
};
use crate::Mode;

/// The header band's rung. The grid rung rather than the dense one because a
/// header carries an inline control (the dirty marker, and shortly a pane
/// menu), and the dense rung's 18px control misses the pointer-target floor.
const HEADER_ROW: f32 = spacing::ROW_GRID;

/// The height [`pane_frame`] gives a pane's header band, in logical points.
///
/// Public because a shell that sizes a window around a pane's *content* has to
/// know what the chrome above that content costs, and the alternative — a shell
/// keeping its own copy of this number — is exactly the drift this crate exists
/// to end. [`pane_frame`] reads it too, so there is one declaration.
#[must_use]
pub const fn header_band_height() -> f32 {
    control::binding(HEADER_ROW).row
}

/// The inset [`pane_frame`] leaves between a pane's outer rect and the content
/// rect it hands the item, on **every** side.
///
/// The panel padding, widened to the focus ring's bleed if that were ever the
/// larger of the two — the ring is painted inside the pane, so its bleed is
/// reserved here rather than clipped away at the moment it matters.
///
/// Public for the same reason [`header_band_height`] is: a window sized to fit
/// a fixed-size raster has to budget for it, and budgeting for it by writing
/// `12.0` at the call site is how the two answers drift apart.
#[must_use]
pub fn pane_content_inset() -> f32 {
    spacing::PANEL_PADDING.max(focus::RING_BLEED)
}

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
/// rect *below* the header band and inside the panel padding.
///
/// **This is a default, not an enforcement, and the distinction matters
/// because a lot of code will be written against it.** A pane that simply
/// draws — `ui.painter()`, its widgets, any nested `Ui` derived from it — is
/// clipped to the content rect and cannot reach the header band. `max_rect`
/// alone would not do that: `Ui::new_child` clones the parent's painter and
/// leaves its clip rect untouched, so `max_rect` seeds layout only, and the
/// [`Ui::shrink_clip_rect`](egui::Ui::shrink_clip_rect) below is what makes
/// the default hold.
///
/// A pane that *sets out* to escape can. `Ui::set_clip_rect` widens the clip
/// on the very `&mut Ui` the item is handed; `egui::Area`, `egui::Window` and
/// `ctx.layer_painter` take a fresh layer from the `Context` entirely. egui
/// offers no way to withhold any of that from a `&mut Ui` holder, and no
/// phrasing of this comment will change it.
///
/// So the honest statement is: **a pane cannot draw a header by accident**,
/// and the contract tests pin that. A pane that means to draw one is a code
/// review, and the reason to expect review to be enough is that no [`Subject`]
/// field asks for a header — a pane wanting one is already off the contract.
pub fn pane_frame(ui: &mut egui::Ui, subject: &Subject, header: bool, mode: Mode) -> egui::Ui {
    let sem = semantic(mode.is_dark());
    let outer = ui.max_rect();

    ui.painter()
        .rect_filled(outer, radius::PANEL, colour(sem.surfaces.raised));

    let mut content = outer;
    if header {
        let band =
            egui::Rect::from_min_size(outer.min, egui::vec2(outer.width(), header_band_height()));
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

    let content = content.shrink(pane_content_inset());

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

/// Draw a module's own chrome inside the module's rect, and return the `Ui`
/// the module's body may draw into.
///
/// The pane header band names the pane and belongs to the surface around it.
/// This is the other thing: the controls a **module** declares, which act on
/// that module alone — a log/linear toggle rescales the chart it sits on and
/// nothing else on the canvas — so they are drawn attached to the module,
/// inside the rect the module was given, below whatever chrome the pane
/// already has.
///
/// This *extends* [`pane_frame`]; it does not fork it. A module is drawn
/// inside a pane like any other item, so it has already been through
/// `pane_frame` by the time this runs, and everything here happens within the
/// content rect that call handed over.
///
/// **A module that declares nothing costs nothing.** `entries` empty — or
/// every entry withheld, which [`Toolbar`] treats the same way — takes zero
/// height, paints nothing, and returns a `Ui` over the whole rect with the
/// same clip, so the module draws exactly where an item with no chrome at all
/// draws. That is not a claim about a code path being similar: it is the same
/// rect, and `a_module_that_declares_no_chrome_draws_where_a_plain_item_does`
/// tessellates both and compares the triangles.
pub fn module_frame(
    ui: &mut egui::Ui,
    entries: &[ToolbarEntry],
    mode: Mode,
) -> (egui::Ui, ToolbarDrawn) {
    let outer = ui.max_rect();
    let mut body = outer;
    let toolbar = Toolbar::new(entries);

    let drawn = if toolbar.has_something_to_say() {
        let out = toolbar.show(ui, mode);
        let sem = semantic(mode.is_dark());
        let y = ui.cursor().min.y;
        // A hairline under the strip, the same divider the header band closes
        // with, so a module's own controls read as belonging to the module
        // rather than floating above its ink.
        ui.painter().line_segment(
            [egui::pos2(outer.left(), y), egui::pos2(outer.right(), y)],
            egui::Stroke::new(1.0, colour(sem.borders.divider)),
        );
        body.min.y = y + spacing::SPACE_2;
        out
    } else {
        ToolbarDrawn::default()
    };

    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(*ui.layout()));
    // The same reason `pane_frame` shrinks its child's clip: `new_child` clones
    // the parent's painter, clip rect included, so without this a module could
    // paint over its own control strip.
    child.shrink_clip_rect(body);
    (child, drawn)
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
    ui.painter().add(selection_wash_shape(rect, mode));
}

/// The selection wash as a shape, for a caller that cannot paint it before the
/// content it sits under.
///
/// [`selection_wash`] is this plus a paint, and every caller should prefer it.
/// This exists for one shape of caller: a row whose rect is not known until its
/// cells have been laid out — an `egui::Grid` row is the case in hand — where
/// the wash has to be reserved with `Painter::add` and filled in afterwards
/// with `Painter::set`. Returning the same shape both ways is what keeps that
/// from becoming a sixth treatment of "this is selected".
pub fn selection_wash_shape(rect: egui::Rect, mode: Mode) -> egui::Shape {
    let sem = semantic(mode.is_dark());
    egui::Shape::Vec(vec![
        egui::Shape::rect_filled(rect, radius::CHIP, colour(sem.rows.selected_background)),
        egui::Shape::rect_stroke(
            rect,
            radius::CHIP,
            egui::Stroke::new(1.0, colour(sem.rows.selected_border)),
            egui::StrokeKind::Inside,
        ),
    ])
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

/// A pane's toolbar, with the collapse rule attached: a row in which **every**
/// control is withheld does not exist this frame.
///
/// [`toolbar_row`] draws whatever it is handed — an all-[`Hidden`] list still
/// costs a row of empty air, because `ui.horizontal` allocates its height
/// before any entry is filtered. That is the wrong default for a pane whose
/// controls are all conditional: an item with nothing to say returns `Hidden`
/// for everything, and the honest render of that is **no row at all**, not a
/// blank band above the content. This type is the mechanical source of "quiet
/// when there is nothing to show" — the item goes on *declaring* its controls
/// (so the vocabulary stays greppable and the audit stays able to check the
/// verbs), and the chrome decides existence from the declaration.
///
/// [`Hidden`]: ToolbarLocation::Hidden
#[derive(Clone, Copy, Debug)]
pub struct Toolbar<'a> {
    entries: &'a [ToolbarEntry],
}

impl<'a> Toolbar<'a> {
    /// A toolbar over a pane's declared controls.
    #[must_use]
    pub const fn new(entries: &'a [ToolbarEntry]) -> Self {
        Self { entries }
    }

    /// Whether this toolbar has anything to say — any entry in a *drawn*
    /// group. [`ToolbarLocation::Hidden`] entries are declared-but-withheld
    /// by definition, and [`ToolbarLocation::Overflow`] entries do not count
    /// either while the overflow affordance remains unbuilt ([`toolbar_row`]
    /// draws them nowhere, so a row summoned for them alone would be exactly
    /// the blank band this type exists to remove).
    #[must_use]
    pub fn has_something_to_say(&self) -> bool {
        self.entries.iter().any(|e| {
            matches!(
                e.location,
                ToolbarLocation::Leading | ToolbarLocation::Trailing
            )
        })
    }

    /// Draw the row — or nothing at all.
    ///
    /// When every entry is withheld this allocates **zero height**: the row
    /// disappears entirely rather than reserving a blank band, and the
    /// returned [`ToolbarDrawn`] is empty, which is the same answer
    /// [`toolbar_row`] gives for the entries it filters out.
    pub fn show(self, ui: &mut egui::Ui, mode: Mode) -> ToolbarDrawn {
        if !self.has_something_to_say() {
            return ToolbarDrawn::default();
        }
        toolbar_row(ui, self.entries, mode)
    }
}

/// The toolbar row: leading group, spacer, trailing group.
///
/// [`ToolbarLocation::Overflow`] entries are collected but not yet drawn —
/// the overflow affordance lands with the first row long enough to need one,
/// and until then they are reported in neither `drawn` nor `activated`, which
/// the contract test pins.
///
/// This draws unconditionally — an all-hidden list still allocates the row's
/// height, because the horizontal container reserves it before the filter
/// runs. A caller whose controls are all conditional wants [`Toolbar`], which
/// adds the existence rule on top of this.
pub fn toolbar_row(ui: &mut egui::Ui, entries: &[ToolbarEntry], mode: Mode) -> ToolbarDrawn {
    let mut out = ToolbarDrawn::default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing::CONTROL_GAP;
        for entry in entries
            .iter()
            .filter(|e| e.location == ToolbarLocation::Leading)
        {
            if let Some(verb) = toolbar_button(ui, entry, mode) {
                out.activated.push(verb);
            }
            out.drawn.push(entry.id);
        }
        let trailing: Vec<&ToolbarEntry> = entries
            .iter()
            .filter(|e| e.location == ToolbarLocation::Trailing)
            .collect();
        if !trailing.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for entry in trailing.iter().rev() {
                    if let Some(verb) = toolbar_button(ui, entry, mode) {
                        out.activated.push(verb);
                    }
                    out.drawn.push(entry.id);
                }
            });
        }
    });
    out
}

/// One toolbar control, drawn on its own: the button, its toggle on-state,
/// its tooltip with the registry keystroke appended — and the **one** keyboard
/// focus treatment, `meridian-egui`'s [`focus_ring_for`], the same ring every
/// Meridian surface draws. Returns the entry's verb when it was activated.
///
/// Public as the primitive [`toolbar_row`]'s groups are made of, so a caller
/// composing a row shape this file does not offer still gets the identical
/// control — a second spelling of the toolbar button is how a toolbar stops
/// feeling like one thing.
///
/// [`focus_ring_for`]: meridian_egui::widgets::focus_ring_for
pub fn toolbar_button(ui: &mut egui::Ui, entry: &ToolbarEntry, mode: Mode) -> Option<Verb> {
    let b = control::binding(HEADER_ROW);
    let sem = semantic(mode.is_dark());

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

    // The one focus treatment: the design system's ring, drawn only when the
    // control holds keyboard focus. egui 0.35 folds `has_focus()` into the
    // same visuals bucket as *pressed*, so without this a focused-but-idle
    // control is indistinguishable from an idle one.
    meridian_egui::widgets::focus_ring_for(ui, &response);

    let response = match &entry.tooltip {
        // The keystroke is appended here, from the registry, so a rebinding
        // can never leave a tooltip claiming a key that no longer works.
        Some(tip) => match entry.verb.keys() {
            Some(keys) => response.on_hover_text(format!("{tip}  ({keys})")),
            None => response.on_hover_text(tip.clone()),
        },
        None => response,
    };

    response.clicked().then_some(entry.verb)
}

/// What a status rail did this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusDrawn {
    /// The ids drawn, in draw order.
    pub drawn: Vec<&'static str>,
    /// The verbs the user activated by dismissing an entry.
    pub dismissed: Vec<Verb>,
}

/// The height [`status_rail_overlay`] gives the rail's band, in logical
/// points — the window bar rung: the grid row plus a breath above and below,
/// the same arithmetic the shell's top and hint bars use for theirs.
#[must_use]
pub const fn status_rail_height() -> f32 {
    spacing::ROW_GRID + 2.0 * spacing::SPACE_2
}

/// The status rail, floated over the window's bottom edge.
///
/// An anchored foreground layer rather than a bottom panel, and the
/// difference is the point: the rail **draws nothing when it has nothing to
/// say** — which is most frames — so a standing band would spend a bar's
/// height of every window reserving room for silence, move the window
/// arithmetic of every view, and re-baseline every pixel test. Floating, it
/// costs nothing until a line exists, exactly as the notification layers do.
///
/// `.fade_in(false)`: `egui::Area` fades a *newly-visible* layer in over
/// `Style::animation_time` by default (120 ms here — `meridian_design`'s
/// theme sets it below egui's stock 200 ms, see `theme.rs`), which is right
/// for a toast or a modal announcing itself but wrong for a status line
/// reporting a fact that was already true the frame before: a fact does not
/// need to announce its own arrival. Without this, two headless captures of
/// the identical idle state, captured after a different number of settled
/// frames, show the rail at two different opacities —
/// which is real: `RawInput::time` is `None` across this workspace's
/// capture paths, so egui advances its clock by `predicted_dt` each frame either
/// way, and a two-frame capture and a four-frame capture land at different
/// points on the same 120 ms curve. Disabling the fade removes the curve
/// rather than out-waiting it.
///
/// Drawn *here* rather than by the shell because this file is where every
/// pixel of workbench chrome is painted — a shell hand-placing an
/// `egui::Area` around the rail would be the first line of a second drawing
/// file.
pub fn status_rail_overlay(
    ctx: &egui::Context,
    entries: &[StatusEntry],
    mode: Mode,
) -> StatusDrawn {
    if entries.is_empty() {
        return StatusDrawn::default();
    }
    let screen = ctx.content_rect();
    let mut out = StatusDrawn::default();
    egui::Area::new(egui::Id::new("workbench-status-rail"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .fade_in(false)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(screen.width(), status_rail_height()),
                egui::Sense::hover(),
            );
            let mut band = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            out = status_rail(&mut band, entries, mode);
        });
    out
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EmptyStateDrawn {
    /// Set when the user took the resolving action.
    pub activated: Option<Action>,
    /// Where the resolving action's button drew, in window-space logical
    /// points — `None` when the empty state offers none.
    ///
    /// Reported for the reason a top bar reports its switcher's rect: the test
    /// that proves a user can actually take this action has to *click* the
    /// button, and the empty state is drawn by the chrome rather than by the
    /// pane, so no [`crate::Item`] can record it. A coordinate typed against a
    /// layout nothing derived it from lands today and goes on being green
    /// while clicking empty pane the first time a headline wraps.
    pub affordance: Option<egui::Rect>,
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
            let button = affordance_button(ui, next);
            out.affordance = Some(button.rect);
            if button.clicked() {
                out.activated = Some(next.action);
            }
        }
    });
    out
}

fn affordance_button(ui: &mut egui::Ui, next: &Affordance) -> egui::Response {
    let b = control::binding(spacing::ROW_GRID);
    // A verb's keystroke is rendered from the registry, never stored on the
    // affordance — one source of truth, so a rebinding cannot make the label
    // lie. An `Action::Open` has no keystroke to render and gets none, which
    // is what stops it borrowing an unrelated verb's key to look bindable.
    let label = match next.action {
        Action::Verb(verb) => verb
            .keys()
            .map_or_else(|| next.label.clone(), |k| format!("{}  {k}", next.label)),
        Action::Open(_) => next.label.clone(),
    };
    ui.add(
        egui::Button::new(egui::RichText::new(label).font(ui_font()))
            .corner_radius(radius::CONTROL)
            .min_size(egui::vec2(0.0, b.control)),
    )
}

// ---------------------------------------------------------------------------
// Unit tests — the Toolbar existence rule, which is a property of this file.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use brightfield_keys::BindingContext;

    use super::*;
    use crate::subject::ToolbarEntry;

    /// A real registered verb — the entries here must survive `Verb::new`'s
    /// debug assertion, and inventing one would trip it by design.
    fn verb() -> Verb {
        let name = brightfield_keys::registry()
            .first()
            .expect("the keyboard registry is not empty")
            .longname;
        Verb::new(name)
    }

    /// Run one frame and report how much vertical space `add` consumed.
    fn height_consumed(add: impl FnOnce(&mut egui::Ui) -> ToolbarDrawn) -> (ToolbarDrawn, f32) {
        let ctx = egui::Context::default();
        let mut slot = None;
        let mut add = Some(add);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            if let Some(add) = add.take() {
                let before = ui.cursor().min.y;
                let drawn = add(ui);
                let after = ui.cursor().min.y;
                slot = Some((drawn, after - before));
            }
        });
        slot.expect("the frame ran")
    }

    /// The existence rule itself: a toolbar in which every control is
    /// withheld does not exist — zero height, nothing drawn, nothing
    /// activatable. This is the mechanical source of "quiet when there is
    /// nothing to show": the item keeps *declaring* the control (greppable,
    /// auditable), and the row vanishes.
    #[test]
    fn a_toolbar_of_only_hidden_entries_takes_no_space_at_all() {
        let entries = vec![
            ToolbarEntry::button("a", "Not now", verb()).at(ToolbarLocation::Hidden),
            ToolbarEntry::button("b", "Also not now", verb()).at(ToolbarLocation::Hidden),
        ];
        let toolbar = Toolbar::new(&entries);
        assert!(!toolbar.has_something_to_say());
        let (drawn, height) = height_consumed(|ui| toolbar.show(ui, Mode::Light));
        assert_eq!(
            drawn,
            ToolbarDrawn::default(),
            "nothing drawn, nothing armed"
        );
        assert_eq!(
            height, 0.0,
            "an all-hidden toolbar reserved {height}pt of blank band"
        );
    }

    /// An empty declaration is the same answer as an all-hidden one.
    #[test]
    fn a_toolbar_with_no_entries_takes_no_space_either() {
        let toolbar = Toolbar::new(&[]);
        assert!(!toolbar.has_something_to_say());
        let (_, height) = height_consumed(|ui| toolbar.show(ui, Mode::Light));
        assert_eq!(height, 0.0);
    }

    /// One offered control brings the row back — and the hidden sibling stays
    /// declared-but-unpainted, exactly as [`toolbar_row`] treats it.
    #[test]
    fn one_visible_entry_summons_the_row_and_hidden_siblings_stay_unpainted() {
        let entries = vec![
            ToolbarEntry::button("offered", "Clear", verb()),
            ToolbarEntry::button("withheld", "Not now", verb()).at(ToolbarLocation::Hidden),
        ];
        let toolbar = Toolbar::new(&entries);
        assert!(toolbar.has_something_to_say());
        let (drawn, height) = height_consumed(|ui| toolbar.show(ui, Mode::Light));
        assert_eq!(drawn.drawn, vec!["offered"]);
        assert!(
            height > 0.0,
            "an offered control did not bring the row back"
        );
    }

    // -----------------------------------------------------------------------
    // The chrome a module owns
    // -----------------------------------------------------------------------

    /// Run one frame over a fixed pane and hand back both what the body
    /// produced and the triangles that would reach the GPU.
    fn frame_pixels<R>(body: impl FnOnce(&mut egui::Ui) -> R) -> (R, Vec<egui::ClippedPrimitive>) {
        let ctx = egui::Context::default();
        let mut out = None;
        let mut body = Some(body);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let full = ctx.run_ui(input, |ui| {
            if let Some(body) = body.take() {
                out = Some(body(ui));
            }
        });
        let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
        (out.expect("the frame body always runs"), primitives)
    }

    /// The triangles of a frame, as comparable text.
    fn triangles(primitives: &[egui::ClippedPrimitive]) -> String {
        let mut out = String::new();
        for primitive in primitives {
            out.push_str(&format!("clip {:?}\n", primitive.clip_rect));
            if let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive {
                for vertex in &mesh.vertices {
                    out.push_str(&format!("  v {vertex:?}\n"));
                }
                out.push_str(&format!("  i {:?}\n", mesh.indices));
            }
        }
        out
    }

    /// Whether any vertex in the frame was painted in exactly `ink`. A clipped
    /// shape contributes no vertices at all, so absence is an assertion.
    fn painted(primitives: &[egui::ClippedPrimitive], ink: egui::Color32) -> bool {
        primitives.iter().any(|p| match &p.primitive {
            egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.iter().any(|v| v.color == ink),
            egui::epaint::Primitive::Callback(_) => false,
        })
    }

    /// A marker no chrome paints, so its position in the tessellated output is
    /// unambiguous.
    const MARKER: egui::Color32 = egui::Color32::from_rgb(7, 11, 13);

    fn paint_marker(ui: &egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(30.0, 10.0)),
            0.0,
            MARKER,
        );
    }

    /// The rule that makes [`module_frame`] an extension of [`pane_frame`]
    /// rather than a fork of it: a module which declares no chrome draws into
    /// exactly the rect an item with no module chrome at all draws into.
    ///
    /// Asserted on the triangles rather than on the rect, because a rect that
    /// matched while the clip did not would still move pixels.
    #[test]
    fn a_module_that_declares_no_chrome_draws_where_a_plain_item_does() {
        let (_, plain) = frame_pixels(|ui| {
            let subject = Subject::new("Rows", Icon("list"), BindingContext::Workspace);
            let body = pane_frame(ui, &subject, true, Mode::Light);
            paint_marker(&body);
        });

        let (drawn, module) = frame_pixels(|ui| {
            let subject = Subject::new("Rows", Icon("list"), BindingContext::Workspace);
            let mut body = pane_frame(ui, &subject, true, Mode::Light);
            let (inner, drawn) = module_frame(&mut body, &[], Mode::Light);
            paint_marker(&inner);
            drawn
        });

        assert_eq!(
            drawn,
            ToolbarDrawn::default(),
            "nothing drawn, nothing armed"
        );
        assert!(
            triangles(&plain).contains(&format!("{MARKER:?}")),
            "the marker never reached the frame, so the comparison proves nothing"
        );
        assert_eq!(
            triangles(&plain),
            triangles(&module),
            "a module with no declared chrome moved the pixels of the pane it \
             sits in"
        );
    }

    /// A declared control takes its row out of the **module's** rect: the
    /// module's body starts below the strip, by the row the toolbar itself
    /// measures plus the gap this file reserves.
    #[test]
    fn a_declared_control_takes_a_row_out_of_the_modules_own_rect() {
        let entries = vec![ToolbarEntry::button("log", "Log", verb())];
        // What the row costs, measured by the toolbar rather than typed here.
        let (_, row) = height_consumed(|ui| Toolbar::new(&entries).show(ui, Mode::Light));
        assert!(
            row > 0.0,
            "the row drew nothing, so there is nothing to place"
        );

        let ((top_before, top_after, drawn), _) = frame_pixels(|ui| {
            let before = ui.max_rect().top();
            let (body, drawn) = module_frame(ui, &entries, Mode::Light);
            (before, body.max_rect().top(), drawn)
        });

        assert_eq!(drawn.drawn, vec!["log"], "the control drew");
        let expected = row + spacing::SPACE_2;
        assert!(
            (top_after - top_before - expected).abs() < 0.01,
            "the module's body starts {}pt below its rect; the strip and its \
             gap come to {expected}pt",
            top_after - top_before
        );
    }

    /// The clip half of the strip rule: a module that reaches for
    /// `ui.painter()` and paints over its own control strip is clipped away.
    ///
    /// [`module_frame`] reserves the strip out of the module's rect, but
    /// `Ui::new_child` clones the parent's painter — clip rect included — so
    /// `max_rect` alone constrains layout and nothing else. Delete
    /// `shrink_clip_rect` in [`module_frame`] and this goes red; the same rule
    /// on [`pane_frame`] is pinned the same way, by
    /// `a_pane_that_paints_its_own_header_through_its_ui_is_clipped_away` in
    /// `crates/brightfield-workbench/tests/chrome_rules.rs`.
    ///
    /// What it does **not** claim: the clip does not reach `egui::Area` or
    /// `ctx.layer_painter`, which take a fresh layer from the `Context`.
    /// Against those the rule is a review rule, as it is for a pane.
    #[test]
    fn a_module_that_paints_over_its_own_control_strip_is_clipped_away() {
        // Two colours nothing else in the frame uses, so presence and absence
        // are both unambiguous.
        const STRIP_INTRUDER: egui::Color32 = egui::Color32::from_rgb(255, 0, 255);
        const HONEST_BODY: egui::Color32 = egui::Color32::from_rgb(0, 255, 0);

        let entries = vec![ToolbarEntry::button("log", "Log", verb())];
        let ((strip_top, body_top), primitives) = frame_pixels(|ui| {
            let outer = ui.max_rect();
            let (body, drawn) = module_frame(ui, &entries, Mode::Light);
            assert_eq!(drawn.drawn, vec!["log"], "no strip drew, so none to cover");
            let rect = body.max_rect();
            // Over the strip the module just drew, through the module's own Ui.
            body.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(outer.left() + 4.0, outer.top()),
                    egui::pos2(outer.left() + 60.0, outer.top() + 4.0),
                ),
                0.0,
                STRIP_INTRUDER,
            );
            // …and something the module is entitled to draw, so a frame that
            // painted nothing — or a clip that swallowed everything — cannot
            // pass this by accident.
            body.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(30.0, 10.0)),
                0.0,
                HONEST_BODY,
            );
            (outer.top(), rect.top())
        });

        assert!(
            body_top >= strip_top + 4.0,
            "the body starts {}pt below the module's rect, so the intruder \
             above is inside the body and this test proves nothing",
            body_top - strip_top
        );
        assert!(
            painted(&primitives, HONEST_BODY),
            "the positive control never reached the frame, so the negative \
             below proves nothing"
        );
        assert!(
            !painted(&primitives, STRIP_INTRUDER),
            "a module painted over its own control strip through its own Ui"
        );
    }

    /// A control the kind withholds does not summon a strip, exactly as a
    /// withheld pane control does not summon a toolbar row — the existence
    /// rule reaches the module chrome because the module chrome is built on
    /// [`Toolbar`] rather than beside it.
    #[test]
    fn a_module_whose_controls_are_all_withheld_gets_no_strip() {
        let entries =
            vec![ToolbarEntry::button("later", "Log", verb()).at(ToolbarLocation::Hidden)];
        let ((top_before, top_after, drawn), _) = frame_pixels(|ui| {
            let before = ui.max_rect().top();
            let (body, drawn) = module_frame(ui, &entries, Mode::Light);
            (before, body.max_rect().top(), drawn)
        });
        assert_eq!(drawn, ToolbarDrawn::default());
        assert!(
            (top_after - top_before).abs() < f32::EPSILON,
            "a withheld control reserved {}pt",
            top_after - top_before
        );
    }

    /// Overflow does not summon the row: [`toolbar_row`] draws overflow
    /// entries nowhere until the affordance lands, so a row summoned for them
    /// alone would be a blank band — the exact thing the rule removes. This
    /// test is the tripwire for that landing: when the affordance arrives,
    /// overflow entries become something to say, and this flips.
    #[test]
    fn overflow_alone_does_not_summon_a_row_nothing_would_draw_into() {
        let entries =
            vec![ToolbarEntry::button("later", "More", verb()).at(ToolbarLocation::Overflow)];
        let toolbar = Toolbar::new(&entries);
        assert!(!toolbar.has_something_to_say());
        let (drawn, height) = height_consumed(|ui| toolbar.show(ui, Mode::Light));
        assert_eq!(drawn, ToolbarDrawn::default());
        assert_eq!(height, 0.0);
    }
}
