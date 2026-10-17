//! An arc Protocol's panels in the window: one tile per `panels/*.yaml`, each
//! drawing its chart or saying, in its own tile, why it cannot.
//!
//! [`crate::protocol_panels`] does the loading; this module is what the canvas
//! draws from it. The tiles stand in a grid of equal cells across the canvas,
//! in file-name order, as many to a row as fit at [`MIN_TILE_WIDTH`], each at
//! least [`MIN_TILE_HEIGHT`] tall; a canvas too short for every row scrolls
//! rather than squeezing a tile below the size its axes and legend need. Each
//! chart is composed into its own cell, so its title, axes and legend are laid
//! out by the composer at the size they are drawn at rather than shrunk from
//! another size, and no tile draws outside its cell.
//!
//! A tile that drew opens as the chart document when it is clicked
//! ([`TilesAction::Open`]); the window draws [`back_band`] above it, the way
//! back to the tiles. A Protocol that has not run has no tables to draw, and
//! says so across the canvas with a Run ([`TilesAction::Run`]).
//!
//! **A new run reloads the tiles.** [`PanelTiles::reload_due`] looks at the
//! Protocol's run records at most once per [`RUN_CHECK_EVERY`]: a listing of one
//! folder, and arc's own read of the newest record when that listing has
//! changed (`a_new_run_reloads_the_tiles`), so a quiet frame reads a directory.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use brightfield_keys::BindingContext;
use brightfield_render::canvas_host::{Color, PixelSize};
use brightfield_spec::layout::Rect;
use brightfield_workbench::subject::{Affordance, EmptyState, Icon};
use brightfield_workbench::{chrome, ItemId, PaneKey, Subject, Verb};
use meridian_design::{semantic, spacing};

use crate::canvas::EguiCanvasHost;
use crate::design::Mode;
use crate::protocol::ui_font;
use crate::protocol_panels::{load, OpenProblem, PanelProblem, ProtocolPanels};

/// The narrowest a tile is laid out: room for a bar chart's category labels,
/// its bars and a legend at its right.
pub const MIN_TILE_WIDTH: f32 = 520.0;

/// The shortest a tile is laid out: room for a title, a plot and its axes.
pub const MIN_TILE_HEIGHT: f32 = 360.0;

/// The gap between two tiles.
pub const TILE_GAP: f32 = spacing::SPACE_3;

/// How often the Protocol's run records are looked at for a newer run.
pub const RUN_CHECK_EVERY: Duration = Duration::from_millis(500);

/// How often a Protocol whose database a run held is opened again while its
/// run records stay as they were: the run's record lands as the run ends, and
/// a new record is looked for at each [`RUN_CHECK_EVERY`] regardless.
pub const LOCKED_RETRY_EVERY: Duration = Duration::from_secs(5);

/// How long a changed cell size has to hold before the tiles are composed at
/// it, so a window being dragged wider composes once, when the drag rests.
const RESIZE_SETTLE: Duration = Duration::from_millis(150);

/// The verb a not-run Protocol's Run asks for: the window's own.
const RUN_VERB: &str = crate::window::RUN_PROTOCOL;

/// One tile as the last frame drew it.
#[derive(Clone, Debug, PartialEq)]
pub struct TileDrawn {
    /// The panel file, relative to the Protocol.
    pub file: PathBuf,
    /// The tile's outer rect, header band included.
    pub rect: egui::Rect,
    /// The box below the header band the chart or the message is drawn in.
    pub body: egui::Rect,
    /// Whether the tile holds a chart, rather than its problems.
    pub drew: bool,
}

/// What a frame of the tiles asked the window to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TilesAction {
    /// Open the tile at this index as the chart document.
    Open(usize),
    /// Run the Protocol, which has not run.
    Run,
}

/// The raster a tile was last presented at.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Raster {
    dev_width: u32,
    dev_height: u32,
    dark: bool,
    generation: u64,
    texture: egui::TextureId,
}

/// What the run records folder held when it was last looked at: how many
/// entries, and the newest modification time among them.
type RunMark = Option<(usize, Option<SystemTime>)>;

/// The Protocol's panels as the window holds them.
pub struct PanelTiles {
    dir: PathBuf,
    name: String,
    state: Result<ProtocolPanels, OpenProblem>,
    /// The cell body size and mode the tiles were composed at.
    composed_at: Option<(egui::Vec2, Mode)>,
    /// A cell body size the canvas asked for that the tiles are not composed
    /// at yet, and when it was first asked for.
    pending: Option<(egui::Vec2, Instant)>,
    /// Bumped each time the tiles are composed, so a raster of the old
    /// composition is not taken for one of the new.
    generation: u64,
    rasters: Vec<Option<Raster>>,
    selected: Option<usize>,
    drawn: Vec<TileDrawn>,
    run_mark: RunMark,
    last_check: Option<Instant>,
    /// When the Protocol was last opened for its panels.
    last_load: Instant,
}

impl PanelTiles {
    /// The window's hold on `panels`, composed into cells of `body`.
    #[must_use]
    pub fn new(panels: ProtocolPanels, body: egui::Vec2) -> Self {
        let dir = panels.dir.clone();
        let name = panels.name.clone();
        Self::over(dir, name, Ok(panels), Some((body, Mode::Light)))
    }

    /// The window's hold on a Protocol in `dir` that did not open for its
    /// panels, saying `problem` until a run lets it.
    #[must_use]
    pub fn unopened(dir: PathBuf, problem: OpenProblem) -> Self {
        let name = arc::spec::Manifest::load(&dir)
            .map(|m| m.name)
            .unwrap_or_else(|_| dir.display().to_string());
        Self::over(dir, name, Err(problem), None)
    }

    fn over(
        dir: PathBuf,
        name: String,
        state: Result<ProtocolPanels, OpenProblem>,
        composed_at: Option<(egui::Vec2, Mode)>,
    ) -> Self {
        let run_mark = run_mark(&dir);
        Self {
            dir,
            name,
            state,
            composed_at,
            pending: None,
            generation: 0,
            rasters: Vec::new(),
            selected: None,
            drawn: Vec::new(),
            run_mark,
            last_check: Some(Instant::now()),
            last_load: Instant::now(),
        }
    }

    /// The Protocol folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The Protocol's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The loaded panels, when the Protocol opened for them.
    #[must_use]
    pub fn panels(&self) -> Option<&ProtocolPanels> {
        self.state.as_ref().ok()
    }

    /// Why the Protocol did not open for its panels, when it did not.
    #[must_use]
    pub fn problem(&self) -> Option<&OpenProblem> {
        self.state.as_ref().err()
    }

    /// The tile open as the chart document, when one is.
    #[must_use]
    pub const fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Mark the tile at `index` as the one open as the chart document, or none.
    pub fn select(&mut self, index: Option<usize>) {
        self.selected = index;
    }

    /// The tiles the last frame drew, in file-name order. Empty while a tile is
    /// open as the chart document, and while the Protocol is not open.
    #[must_use]
    pub fn drawn(&self) -> &[TileDrawn] {
        &self.drawn
    }

    /// The raster slots the tiles hold, for the window's sweep to keep.
    pub fn raster_keys(&self) -> impl Iterator<Item = PaneKey> + '_ {
        let drew = |i: usize| {
            self.panels()
                .and_then(|p| p.tiles.get(i))
                .is_some_and(|t| t.outcome.is_ok())
        };
        self.rasters
            .iter()
            .enumerate()
            .filter(move |(i, r)| r.is_some() && drew(*i))
            .map(|(i, _)| tile_key(i))
    }

    /// Whether the Protocol's newest run is not the one the tiles were drawn
    /// from, looked at no more often than [`RUN_CHECK_EVERY`].
    ///
    /// A Protocol the run held locked is tried again when the listing changes,
    /// and otherwise once per [`LOCKED_RETRY_EVERY`]: the listing may already
    /// have held the run's record when the run still held the database.
    pub fn reload_due(&mut self) -> bool {
        if self
            .last_check
            .is_some_and(|at| at.elapsed() < RUN_CHECK_EVERY)
        {
            return false;
        }
        self.last_check = Some(Instant::now());
        let mark = run_mark(&self.dir);
        let changed = mark != self.run_mark;
        if matches!(self.state, Err(OpenProblem::Locked { .. })) {
            return changed || self.last_load.elapsed() >= LOCKED_RETRY_EVERY;
        }
        if !changed {
            return false;
        }
        self.run_mark = mark;
        let newest = arc::spec::run_tables(&self.dir, None)
            .ok()
            .map(|run| run.run_id);
        let drawn = self.panels().and_then(|p| p.run_id.clone());
        newest.is_some() && newest != drawn
    }

    /// Open the Protocol again, from its newest run, composing the tiles at the
    /// size and mode they were last composed at.
    pub fn reload(&mut self) {
        let (body, mode) = self
            .composed_at
            .unwrap_or((egui::vec2(MIN_TILE_WIDTH, MIN_TILE_HEIGHT), Mode::Light));
        self.state = load(&self.dir, viewport(body));
        self.last_load = Instant::now();
        // The rasters were of the tiles that were; the sweep frees their slots.
        self.rasters.clear();
        if let Ok(panels) = &mut self.state {
            self.name = panels.name.clone();
            if mode != Mode::Light {
                panels.compose_tiles_at(viewport(body), mode);
            }
            self.composed_at = Some((body, mode));
        }
        self.run_mark = run_mark(&self.dir);
        self.generation += 1;
        self.selected = None;
    }

    /// Draw the tiles across `area`, or the Protocol's problem when it did not
    /// open, and say what the reader asked for.
    ///
    /// `host` rasters each chart into a slot of its own; a window with no
    /// device behind it lays the tiles out and draws their words, and leaves
    /// the charts' boxes empty.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        host: Option<&mut EguiCanvasHost>,
        mode: Mode,
    ) -> Option<TilesAction> {
        self.drawn.clear();
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.shrink_clip_rect(area);
        if self.state.is_err() {
            return self.problem_ui(&mut child, mode);
        }
        let count = self.panels().map_or(0, |p| p.tiles.len());
        let width = area.width() - scrollbar_room();
        let cells = tile_rects(width, area.height(), count);
        let first = cells.first()?;
        self.compose_for(body_of(*first).size(), mode, ui.ctx());
        let content = cells
            .iter()
            .fold(egui::Rect::NOTHING, |all, cell| all.union(*cell));
        let ppp = ui.ctx().pixels_per_point();
        let mut host = host;
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("panel-tiles")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                let (space, _) = ui
                    .allocate_exact_size(egui::vec2(width, content.height()), egui::Sense::hover());
                let origin = space.min.to_vec2();
                for (index, cell) in cells.iter().enumerate() {
                    let rect = cell.translate(origin);
                    if let Some(picked) =
                        self.tile_ui(ui, index, rect, host.as_deref_mut(), ppp, mode)
                    {
                        action = Some(picked);
                    }
                }
            });
        action
    }

    /// Compose the tiles into cells of `body` in `mode`, when that is not
    /// what they were composed at: at once for a change of mode or the first
    /// frame's size, and once a new size has held for [`RESIZE_SETTLE`].
    fn compose_for(&mut self, body: egui::Vec2, mode: Mode, ctx: &egui::Context) {
        let Some((at, was)) = self.composed_at else {
            self.recompose(body, mode);
            return;
        };
        let same_size = (at - body).abs().max_elem() <= 1.0;
        if same_size && was == mode {
            self.pending = None;
            return;
        }
        let first = self.generation == 0 && self.pending.is_none();
        if was != mode || first {
            self.recompose(body, mode);
            return;
        }
        match self.pending {
            Some((size, since)) if (size - body).abs().max_elem() <= 1.0 => {
                if since.elapsed() >= RESIZE_SETTLE {
                    self.recompose(body, mode);
                } else {
                    ctx.request_repaint_after(RESIZE_SETTLE);
                }
            }
            _ => {
                self.pending = Some((body, Instant::now()));
                ctx.request_repaint_after(RESIZE_SETTLE);
            }
        }
    }

    fn recompose(&mut self, body: egui::Vec2, mode: Mode) {
        if let Ok(panels) = &mut self.state {
            panels.compose_tiles_at(viewport(body), mode);
        }
        self.composed_at = Some((body, mode));
        self.pending = None;
        self.generation += 1;
    }

    fn tile_ui(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        rect: egui::Rect,
        host: Option<&mut EguiCanvasHost>,
        ppp: f32,
        mode: Mode,
    ) -> Option<TilesAction> {
        let generation = self.generation;
        let panels = self.state.as_ref().ok()?;
        let tile = panels.tiles.get(index)?;
        let subject = Subject::new(
            panel_name(&tile.file),
            Icon("chart-bar"),
            BindingContext::Workspace,
        );
        let about = panel_about(&tile.file);
        let mut frame = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        frame.shrink_clip_rect(rect.intersect(ui.clip_rect()));
        let mut body_ui = chrome::pane_frame(&mut frame, &subject, true, mode);
        let body = body_ui.max_rect();
        let sem = semantic(mode.is_dark());
        let drew = tile.outcome.is_ok();
        self.drawn.push(TileDrawn {
            file: tile.file.clone(),
            rect,
            body,
            drew,
        });
        match &tile.outcome {
            Ok(composed) => {
                let size = egui::vec2(composed.width as f32, composed.height as f32);
                let page = egui::Rect::from_min_size(body.min, size);
                if let Some(host) = host {
                    let dev = PixelSize {
                        width: (size.x * ppp).round().max(1.0) as u32,
                        height: (size.y * ppp).round().max(1.0) as u32,
                    };
                    if self.rasters.len() <= index {
                        self.rasters.resize(index + 1, None);
                    }
                    let stale = self.rasters[index].is_none_or(|r| {
                        r.dev_width != dev.width
                            || r.dev_height != dev.height
                            || r.dark != mode.is_dark()
                            || r.generation != generation
                    });
                    if stale {
                        let base = Color::from_token(sem.surfaces.raised);
                        let mut scaled = vello::Scene::new();
                        scaled.append(&composed.scene, Some(kurbo::Affine::scale(f64::from(ppp))));
                        let texture = host.present_keyed(tile_key(index), &scaled, dev, base);
                        self.rasters[index] = Some(Raster {
                            dev_width: dev.width,
                            dev_height: dev.height,
                            dark: mode.is_dark(),
                            generation,
                            texture,
                        });
                    }
                    if let Some(raster) = self.rasters[index] {
                        body_ui.painter().image(
                            raster.texture,
                            page,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                }
                // The legends, where the layout put them beside or under the
                // plot, drawn as the chart pane draws them: a legend is egui's
                // ink over the page, not part of the raster.
                crate::legend::draw_right(&body_ui, page.min, composed);
                crate::legend::draw_below(&body_ui, page.min, composed, composed.mode);
                let response = ui
                    .interact(
                        rect.intersect(ui.clip_rect()),
                        ui.id().with(("panel-tile", index)),
                        egui::Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!("{about}\nClick to open this panel as the chart"));
                response.clicked().then_some(TilesAction::Open(index))
            }
            Err(problems) => {
                problems_ui(&mut body_ui, tile.title.as_deref(), problems, mode);
                let _ = ui
                    .interact(
                        rect.intersect(ui.clip_rect()),
                        ui.id().with(("panel-tile", index)),
                        egui::Sense::hover(),
                    )
                    .on_hover_text(about);
                // A tile that drew before and holds problems now gives its
                // raster slot back.
                if let Some(slot) = self.rasters.get_mut(index) {
                    *slot = None;
                }
                None
            }
        }
    }

    /// The canvas-wide state of a Protocol that did not open: what is wrong,
    /// and a Run when what is wrong is that it has not run.
    fn problem_ui(&self, ui: &mut egui::Ui, mode: Mode) -> Option<TilesAction> {
        let problem = self.problem()?;
        let not_run = matches!(problem, OpenProblem::NotRun { .. });
        let mut empty = EmptyState::new(
            Icon("chart-bar"),
            if not_run {
                "This Protocol has not run yet"
            } else {
                "This Protocol's panels cannot be drawn"
            },
            problem.message(),
        );
        if not_run {
            empty = empty.with_next(Affordance::new("Run", Verb::new(RUN_VERB)));
        }
        let drawn = chrome::empty_state(ui, &empty, mode);
        match drawn.activated {
            Some(brightfield_workbench::subject::Action::Verb(_)) => Some(TilesAction::Run),
            _ => None,
        }
    }
}

/// A panel's name as the window shows it: its file name without `.yaml`.
#[must_use]
pub fn panel_name(file: &Path) -> String {
    file.file_stem().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// What a panel is, for its tooltip: its path in the Protocol and its kind.
#[must_use]
pub fn panel_about(file: &Path) -> String {
    format!("{} \u{b7} Mosaic spec", file.display())
}

/// The header the window draws above a tile open as the chart document: the
/// way back to every tile, as an "All panels" control at its left, and the
/// panel's name. Answers whether the way back was taken.
pub fn back_band(ui: &mut egui::Ui, band: egui::Rect, file: &Path, mode: Mode) -> bool {
    let sem = semantic(mode.is_dark());
    ui.painter()
        .rect_filled(band, 0.0, chrome::colour(sem.surfaces.header));
    ui.painter().line_segment(
        [band.left_bottom(), band.right_bottom()],
        egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(band.shrink2(egui::vec2(spacing::SPACE_3, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.shrink_clip_rect(band);
    let back = child
        .button(
            egui::RichText::new("\u{2190} All panels")
                .font(ui_font())
                .color(chrome::colour(sem.text.primary)),
        )
        .on_hover_text("Back to every panel (Esc)");
    child.add_space(spacing::SPACE_4);
    child
        .label(
            egui::RichText::new(panel_name(file))
                .font(ui_font())
                .color(chrome::colour(sem.text.primary)),
        )
        .on_hover_text(panel_about(file));
    back.clicked()
}

/// The height [`back_band`] takes above the chart.
#[must_use]
pub fn back_band_height() -> f32 {
    chrome::header_band_height()
}

/// Each problem a tile holds, in its body: the panel's declared title when it
/// has one, then one paragraph per problem, wrapped to the tile.
fn problems_ui(ui: &mut egui::Ui, title: Option<&str>, problems: &[PanelProblem], mode: Mode) {
    let sem = semantic(mode.is_dark());
    if let Some(title) = title {
        ui.label(
            egui::RichText::new(title)
                .font(ui_font())
                .color(chrome::colour(sem.text.primary)),
        );
        ui.add_space(spacing::SPACE_2);
    }
    ui.label(
        egui::RichText::new("This panel cannot be drawn:")
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
    for problem in problems {
        ui.add_space(spacing::SPACE_2);
        ui.add(
            egui::Label::new(
                egui::RichText::new(problem.message())
                    .font(ui_font())
                    .color(chrome::colour(sem.text.primary)),
            )
            .wrap(),
        );
    }
}

/// The cells `count` tiles stand in across a canvas `width` wide and `height`
/// tall, relative to its top left, in file-name order: as many to a row as fit
/// at [`MIN_TILE_WIDTH`] (at least one), the rows sharing the height, and each
/// at least [`MIN_TILE_HEIGHT`] tall. Every cell is the same size, and no two
/// overlap. The cells may reach below `height`; the canvas scrolls them.
#[must_use]
pub fn tile_rects(width: f32, height: f32, count: usize) -> Vec<egui::Rect> {
    if count == 0 || width <= 0.0 {
        return Vec::new();
    }
    let fit = ((width + TILE_GAP) / (MIN_TILE_WIDTH + TILE_GAP)).floor();
    let columns = (fit.max(1.0) as usize).min(count);
    let rows = count.div_ceil(columns);
    let cell_w = (width - TILE_GAP * (columns as f32 - 1.0)) / columns as f32;
    let cell_h = ((height - TILE_GAP * (rows as f32 - 1.0)) / rows as f32).max(MIN_TILE_HEIGHT);
    (0..count)
        .map(|i| {
            let (row, column) = (i / columns, i % columns);
            egui::Rect::from_min_size(
                egui::pos2(
                    column as f32 * (cell_w + TILE_GAP),
                    row as f32 * (cell_h + TILE_GAP),
                ),
                egui::vec2(cell_w, cell_h),
            )
        })
        .collect()
}

/// The box a chart is composed into inside a tile `cell`: below the header
/// band and inside the pane's inset, as [`chrome::pane_frame`] lays it.
#[must_use]
pub fn body_of(cell: egui::Rect) -> egui::Rect {
    let mut body = cell;
    body.min.y += chrome::header_band_height();
    body.shrink(chrome::pane_content_inset())
}

/// The cell body every tile of `count` is composed into on a canvas of
/// `canvas`: what a boot composes at before the window has drawn a frame.
#[must_use]
pub fn body_for_canvas(canvas: egui::Vec2, count: usize) -> egui::Vec2 {
    tile_rects(canvas.x - scrollbar_room(), canvas.y, count)
        .first()
        .map_or(egui::vec2(MIN_TILE_WIDTH, MIN_TILE_HEIGHT), |cell| {
            body_of(*cell).size()
        })
}

/// The composer's box for a cell body of `size`.
#[must_use]
pub fn viewport(size: egui::Vec2) -> Rect {
    Rect::new(
        0.0,
        0.0,
        f64::from(size.x.max(1.0)),
        f64::from(size.y.max(1.0)),
    )
}

/// The width kept at the canvas's right for the scroll bar, so a canvas that
/// scrolls does not lay its tiles under it.
fn scrollbar_room() -> f32 {
    spacing::SPACE_3
}

/// The raster slot key of the tile at `index`. The names are made once per
/// index and kept for the process, a few bytes per tile ever drawn.
fn tile_key(index: usize) -> PaneKey {
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    while names.len() <= index {
        let name = format!("panel-tile-{}", names.len());
        names.push(Box::leak(name.into_boxed_str()));
    }
    PaneKey::new(ItemId::new(names[index]))
}

/// What the Protocol's run records folder holds now, by arc's layout of it.
fn run_mark(dir: &Path) -> RunMark {
    let entries = std::fs::read_dir(dir.join("build").join(".arcform").join("runs")).ok()?;
    let mut count = 0;
    let mut newest: Option<SystemTime> = None;
    for entry in entries.flatten() {
        count += 1;
        let modified = entry.metadata().and_then(|m| m.modified()).ok();
        newest = newest.max(modified);
    }
    Some((count, newest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_are_equal_and_never_overlap() {
        for count in 1..=7 {
            for (w, h) in [
                (700.0, 500.0),
                (1200.0, 700.0),
                (400.0, 300.0),
                (2000.0, 1200.0),
            ] {
                let cells = tile_rects(w, h, count);
                assert_eq!(cells.len(), count);
                for (i, a) in cells.iter().enumerate() {
                    assert!(a.width() > 0.0 && a.height() >= MIN_TILE_HEIGHT);
                    assert!(a.right() <= w + 0.01, "{a:?} is past {w}");
                    // Equal to within rounding: each rect's size is taken
                    // back out of its own corners.
                    assert!((a.size() - cells[0].size()).abs().max_elem() < 0.01);
                    for b in &cells[i + 1..] {
                        assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_tile_is_never_narrower_than_the_minimum_unless_the_canvas_is() {
        let cells = tile_rects(1100.0, 600.0, 2);
        assert_eq!(cells.len(), 2);
        assert!(cells[0].width() >= MIN_TILE_WIDTH);
        assert_eq!(cells[0].top(), cells[1].top(), "two fit side by side");
        let narrow = tile_rects(700.0, 600.0, 2);
        assert!(narrow[1].top() > narrow[0].bottom(), "one to a row");
    }
}
