//! The Data pane — the chart's peer — and the one Meridian table chrome.
//!
//! # Two queries over one materialisation
//!
//! A step's output is a table in DuckDB, and the chart and this grid are two
//! queries over that same materialisation: the chart projects channels, the
//! grid keeps the row shape. Both queries are compiled by the one emission
//! path in `brightfield-sql` (the chart through `emit_query`, the grid through
//! `emit_rows_query`, sharing one `compile_selection`), so the two surfaces
//! **cannot disagree about the PREDICATE** — the selection state resolves to
//! one WHERE clause, neither surface is authoritative over the other, and
//! neither ever filters a materialised batch client-side. That is why they are
//! peers: two projections of one step, one toggle between them (the centre tab
//! strip).
//!
//! ## The sampling clause is the one deliberate divergence
//!
//! There is exactly one way the two can now show different rows, and it is on
//! purpose. Above the renderer's drawable ceiling the chart draws a pushed-down
//! sample — a `hash(row) % 2^k = 0` clause the chart's query carries and this
//! grid's does not — and the chart says so in its own ink. **The grid never
//! samples.** It is the unsampled view: the place someone goes precisely
//! because they doubt the picture, and a grid that quietly agreed with a
//! sampled chart would answer the doubt with the same partial evidence that
//! caused it. So the honest statement is the narrow one above: same predicate,
//! same materialisation, and one clause the chart adds and labels.
//!
//! # The visible window is all that ever exists client-side
//!
//! The grid renders by querying the engine for the **visible window**
//! ([`brightfield_engine::Session::execute_step_rows_window`] — `LIMIT`/
//! `OFFSET` under the deterministic total order that seam imposes, wrapped
//! around the identical emitted rows SQL) and sizes its scroll range from a
//! `count(*)` over the same SQL. Scrolling a table larger
//! than memory therefore never materialises it on this side of the seam: the
//! page cache below holds the fetched window and nothing else, bounded by the
//! viewport plus [`PAGE_PAD`] rows of scroll margin. There is deliberately no
//! full-result `Vec` anywhere in this module — a grid that materialises a
//! result set to scroll it is the client-side architecture the engine seam
//! exists to reject, and at crosswalk scale it is the first thing that would
//! fall over.
//!
//! # Meridian cell chrome
//!
//! One table treatment, used by the DuckDB grid **and** the protocol Steps
//! sheet (whose bespoke `egui::Grid` this module retires):
//!
//! - the dense row rung, whole: every dimension comes from
//!   `control::binding(spacing::ROW_DENSE)`, so row height, padding and text
//!   size agree by construction;
//! - tabular numerals + slashed zero **declared at table scope, never
//!   globally** ([`NUMERIC_FEATURES`], from the token layer — see the note
//!   below);
//! - numeric columns right-aligned — tabular figures align digits, right
//!   alignment aligns magnitudes;
//! - alignment **never** via a monospace face. The density guideline is
//!   explicit that a mono UI font is not the way to align numbers (too wide,
//!   reads as "technical"); the retired Steps grid set its order column in
//!   `mono_font()`, and that treatment ends here.
//!
//! ## The `tnum`/`zero` wiring point
//!
//! epaint 0.35 shapes text through HarfBuzz (`harfrust`) but exposes no
//! OpenType feature list — its shaper is invoked with an empty feature slice
//! — so the declared features cannot reach glyph selection yet.
//! [`NUMERIC_FEATURES`] is the **single wiring point**: the table-scope
//! declaration, consumed from `meridian_design::typography` (`TNUM` + `ZERO`),
//! pinned by test to exactly that pair and to table scope (the global egui
//! style declares none). When epaint grows a feature API, this constant is
//! where it plugs in; until then the cell chrome still delivers the
//! rest of the density contract (dense rung, right-aligned numerics, no
//! monospace alignment) with the proportional UI face.

use std::ops::Range;

use arrow::array::{Array, StringArray};
use arrow::compute::cast;
use arrow::datatypes::DataType;

use brightfield_engine::error::EngineError;
use brightfield_engine::{RecordBatch, Session};
use brightfield_keys::BindingContext;
use brightfield_protocol::{sheet, StepRow};
use brightfield_workbench::registry::Slot;
use brightfield_workbench::{
    chrome, Activity, EmptyState, Icon, Item, ItemCtx, ItemId, ItemSpec, Subject, Verb,
};

use meridian_design::typography::{self, FontFeature};
use meridian_design::{control, semantic, spacing};

use crate::app::ChartDoc;
use crate::chart_item::run_state_pill;
use crate::design::Mode;

// ---------------------------------------------------------------------------
// Cell chrome — the table-scope typography and geometry.
// ---------------------------------------------------------------------------

/// The OpenType features every numeric cell in a Meridian table declares —
/// tabular numerals plus slashed zero, straight from the token layer, **at
/// table scope only** (the global style declares no features; prose keeps
/// proportional figures). See the module note for what "declared" can and
/// cannot mean under epaint 0.35.
pub const NUMERIC_FEATURES: [FontFeature; 2] = [typography::TNUM, typography::ZERO];

/// The one cell face: the proportional UI sans at the UI base size. Never a
/// monospace face — column alignment comes from right alignment and the
/// tabular-figure declaration, not from a fixed-advance font.
#[must_use]
pub fn cell_font() -> egui::FontId {
    egui::FontId::new(typography::UI_SIZE, egui::FontFamily::Proportional)
}

/// One column of a Meridian table: its header text and whether its cells are
/// numeric (right-aligned, tabular-figure scope).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridColumn {
    /// The header label.
    pub name: String,
    /// Whether cells right-align and sit in the tabular-numeral scope.
    pub numeric: bool,
}

/// Which ink a cell's text takes, resolved against the mode's semantic set at
/// draw time — data in primary, supporting columns in secondary, absence
/// (NULL, live-state placeholders) in muted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellInk {
    /// The data itself.
    Primary,
    /// Supporting detail beside it.
    Secondary,
    /// Absence or metadata — a NULL, a not-yet state.
    Muted,
}

/// One cell's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellText {
    /// The text.
    pub text: String,
    /// Its ink.
    pub ink: CellInk,
}

impl CellText {
    /// Data ink.
    #[must_use]
    pub fn primary(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ink: CellInk::Primary,
        }
    }

    /// Supporting ink.
    #[must_use]
    pub fn secondary(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ink: CellInk::Secondary,
        }
    }

    /// Absence ink.
    #[must_use]
    pub fn muted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ink: CellInk::Muted,
        }
    }
}

// ---------------------------------------------------------------------------
// The row source seam — what a table renders over.
// ---------------------------------------------------------------------------

/// A windowed row supplier for [`show_table`]: the engine-backed grid and the
/// in-memory Steps sheet both speak this, so there is exactly one table
/// treatment in the shell.
pub trait RowSource {
    /// Load whatever `visible` needs before cells draw. The engine source
    /// fetches a window from DuckDB here; an in-memory source does nothing.
    fn prepare(&mut self, visible: Range<u64>);
    /// Total rows — the scroll range, not what is held client-side.
    fn total_rows(&self) -> u64;
    /// The columns.
    fn columns(&self) -> &[GridColumn];
    /// The cell at (`row`, `col`), or `None` when the row is outside the
    /// currently held window (the frame after a jump paints it blank once;
    /// [`RowSource::prepare`] has already been asked for the window).
    fn cell_text(&self, row: u64, col: usize) -> Option<CellText>;
    /// The row wearing the selection wash, if any.
    fn selected_row(&self) -> Option<u64> {
        None
    }
}

/// Render `source` as the one Meridian table: `egui_table` (sticky header,
/// virtualised rows) under the dense-rung cell chrome.
pub fn show_table(ui: &mut egui::Ui, salt: &str, mode: Mode, source: &mut dyn RowSource) {
    let binding = control::binding(spacing::ROW_DENSE);
    let num_rows = source.total_rows();
    let num_columns = source.columns().len();
    if num_columns == 0 {
        // Nothing fetched yet, or a schema-less result — there is no table to
        // draw. The engine source reports its state through the pane around
        // this call; an empty reserve here would be a table-shaped lie.
        return;
    }
    let columns: Vec<egui_table::Column> = (0..num_columns)
        .map(|_| {
            egui_table::Column::new(120.0)
                .range(egui::Rangef::new(2.0 * binding.pad_x + binding.icon, 480.0))
                .resizable(true)
        })
        .collect();
    let mut delegate = MeridianTableDelegate {
        source,
        mode,
        binding,
    };
    let _ = egui_table::Table::new()
        .id_salt(salt)
        .num_rows(num_rows)
        .columns(columns)
        .headers([egui_table::HeaderRow::new(binding.row)])
        .show(ui, &mut delegate);
}

/// The delegate `egui_table` calls back into: window prefetch, the dense row
/// rung, and the cell chrome.
struct MeridianTableDelegate<'a> {
    source: &'a mut dyn RowSource,
    mode: Mode,
    binding: control::Binding,
}

impl MeridianTableDelegate<'_> {
    /// Lay one cell out: leading pad, vertical centre, and right alignment for
    /// numerics — magnitude alignment, per the density guideline, with no
    /// monospace anywhere near it.
    fn cell_layout(&self, ui: &mut egui::Ui, numeric: bool, add: impl FnOnce(&mut egui::Ui)) {
        let pad = self.binding.pad_x;
        if numeric {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(pad);
                add(ui);
            });
        } else {
            ui.add_space(pad);
            add(ui);
        }
    }
}

impl egui_table::TableDelegate for MeridianTableDelegate<'_> {
    fn prepare(&mut self, info: &egui_table::PrefetchInfo) {
        self.source.prepare(info.visible_rows.clone());
    }

    fn default_row_height(&self) -> f32 {
        self.binding.row
    }

    fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell: &egui_table::HeaderCellInfo) {
        let Some(column) = self.source.columns().get(cell.col_range.start) else {
            return;
        };
        let (name, numeric) = (column.name.clone(), column.numeric);
        let sem = semantic(self.mode.is_dark());
        self.cell_layout(ui, numeric, |ui| {
            ui.label(
                egui::RichText::new(name)
                    .font(cell_font())
                    .color(chrome::colour(sem.text.muted)),
            );
        });
    }

    fn row_ui(&mut self, ui: &mut egui::Ui, row_nr: u64) {
        // The full-width underlays, painted before any cell draws over them:
        // the one selection wash for the cursor row, a zebra stripe otherwise.
        // The retired egui::Grid had to reserve a wash slot and fill it after
        // layout because a grid row's width is unknown until its widest cell
        // is measured; here the row rect is handed in whole.
        let rect = ui.max_rect();
        if self.source.selected_row() == Some(row_nr) {
            chrome::selection_wash(ui, rect, self.mode);
        } else if row_nr % 2 == 1 {
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
        }
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &egui_table::CellInfo) {
        let Some(column) = self.source.columns().get(cell.col_nr) else {
            return;
        };
        let numeric = column.numeric;
        let Some(value) = self.source.cell_text(cell.row_nr, cell.col_nr) else {
            return;
        };
        let sem = semantic(self.mode.is_dark());
        let ink = match value.ink {
            CellInk::Primary => sem.text.primary,
            CellInk::Secondary => sem.text.secondary,
            CellInk::Muted => sem.text.muted,
        };
        self.cell_layout(ui, numeric, |ui| {
            ui.label(
                egui::RichText::new(value.text)
                    .font(cell_font())
                    .color(chrome::colour(ink)),
            );
        });
    }
}

// ---------------------------------------------------------------------------
// The engine-backed source — the visible-window read path.
// ---------------------------------------------------------------------------

/// Scroll margin fetched around the visible window, in rows, each side. The
/// bound on what this module ever holds client-side is therefore
/// `visible + 2 * PAGE_PAD` rows — a function of the viewport, never of the
/// result set.
pub const PAGE_PAD: u64 = 128;

/// One fetched window of a step's rows.
#[derive(Clone, Debug, Default)]
pub struct GridPage {
    /// Which rows of the materialisation this page holds.
    pub window: Range<u64>,
    /// The step's columns.
    pub columns: Vec<GridColumn>,
    /// The window's cells, `rows[row - window.start][col]`.
    pub rows: Vec<Vec<CellText>>,
}

/// Fetch `window` of the step's rows through the engine's windowed seam and
/// tabulate it for the cell chrome.
///
/// This is the grid's whole read path, and it is deliberately a free function
/// over `&Session`: the tests exercise the exact code the pane scrolls with —
/// including the grid/chart agreement re-exercised from this side — rather
/// than a lookalike.
///
/// # Errors
///
/// As [`Session::execute_step_rows_window`].
// The Err variant is the engine's own error type, at the engine's own size —
// boxing it at this one seam would make the grid the only consumer to receive
// it boxed, for a path taken once per scroll page.
#[allow(clippy::result_large_err)]
pub fn fetch_page(
    session: &Session,
    mark_index: usize,
    window: Range<u64>,
) -> Result<GridPage, EngineError> {
    let limit = window.end.saturating_sub(window.start);
    let batches = session.execute_step_rows_window(mark_index, window.start, limit)?;
    let (columns, rows) = tabulate(&batches);
    let held = window.start..window.start + rows.len() as u64;
    Ok(GridPage {
        window: held,
        columns,
        rows,
    })
}

/// Turn Arrow batches into columns + cells. Every column is rendered through
/// one universal path — an Arrow cast to `Utf8` — so a type this module never
/// anticipated still displays rather than panicking; a NULL is spelled `NULL`
/// in muted ink, never an empty cell a reader could mistake for an empty
/// string.
fn tabulate(batches: &[RecordBatch]) -> (Vec<GridColumn>, Vec<Vec<CellText>>) {
    let Some(first) = batches.first() else {
        return (Vec::new(), Vec::new());
    };
    let schema = first.schema();
    let columns: Vec<GridColumn> = schema
        .fields()
        .iter()
        .map(|f| GridColumn {
            name: f.name().clone(),
            numeric: f.data_type().is_numeric(),
        })
        .collect();

    let mut rows: Vec<Vec<CellText>> = Vec::new();
    for batch in batches {
        let as_text: Vec<Option<StringArray>> = batch
            .columns()
            .iter()
            .map(|col| {
                cast(col, &DataType::Utf8)
                    .ok()
                    .and_then(|c| c.as_any().downcast_ref::<StringArray>().cloned())
            })
            .collect();
        for row in 0..batch.num_rows() {
            let cells: Vec<CellText> = as_text
                .iter()
                .map(|col| match col {
                    Some(col) if col.is_null(row) => CellText::muted("NULL"),
                    Some(col) => CellText::primary(col.value(row)),
                    // A column whose type would not cast to text — display
                    // its absence honestly rather than guessing at bytes.
                    None => CellText::muted("?"),
                })
                .collect();
            rows.push(cells);
        }
    }
    (columns, rows)
}

/// The pane's view-local cache: the fetched window and nothing else. Keyed to
/// the coordinator's interaction generation, so a brush/param re-query drops
/// it and the next frame reads the new materialisation state.
#[derive(Default)]
pub struct GridCache {
    /// The generation the cache was read at; `None` before the first read.
    generation: Option<u64>,
    /// `count(*)` over the step's rows SQL at that generation.
    total: u64,
    /// The step's columns.
    columns: Vec<GridColumn>,
    /// The held window.
    window: Range<u64>,
    /// The held cells — bounded by the viewport plus scroll margin.
    rows: Vec<Vec<CellText>>,
    /// The last read failure, shown under the table until a read succeeds.
    error: Option<String>,
}

impl GridCache {
    /// How many rows are held client-side right now — what the
    /// larger-than-memory test bounds.
    #[must_use]
    pub fn held_rows(&self) -> usize {
        self.rows.len()
    }
}

/// The engine-backed [`RowSource`]: windowed reads over the live session.
pub struct EngineRows<'a> {
    session: &'a Session,
    mark_index: usize,
    generation: u64,
    cache: &'a mut GridCache,
}

impl<'a> EngineRows<'a> {
    /// A source over `session`'s step at `mark_index`, caching into `cache`
    /// keyed by the coordinator `generation`.
    ///
    /// Construction refreshes the count when the generation moved (an
    /// interaction re-queried the materialisation), so the scroll range is
    /// already right when the table lays out.
    pub fn new(
        session: &'a Session,
        mark_index: usize,
        generation: u64,
        cache: &'a mut GridCache,
    ) -> Self {
        if cache.generation != Some(generation) {
            match session.step_rows_count(mark_index) {
                Ok(total) => {
                    cache.total = total;
                    cache.error = None;
                }
                Err(e) => {
                    cache.total = 0;
                    cache.error = Some(e.to_string());
                }
            }
            cache.columns.clear();
            cache.window = 0..0;
            cache.rows.clear();
            cache.generation = Some(generation);
        }
        // The schema has to exist before the table can lay out at all — the
        // column count is the table's shape — and it only arrives with a
        // fetched page, so seed the head of the materialisation here. The
        // window [`RowSource::prepare`] asks for replaces this immediately
        // when the scroll position is elsewhere.
        if cache.error.is_none() && cache.columns.is_empty() && cache.total > 0 {
            match fetch_page(session, mark_index, 0..cache.total.min(PAGE_PAD)) {
                Ok(page) => {
                    cache.window = page.window;
                    cache.columns = page.columns;
                    cache.rows = page.rows;
                }
                Err(e) => {
                    cache.error = Some(e.to_string());
                }
            }
        }
        Self {
            session,
            mark_index,
            generation,
            cache,
        }
    }

    /// The last read failure, if the cache holds one.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.cache.error.as_deref()
    }
}

impl RowSource for EngineRows<'_> {
    fn prepare(&mut self, visible: Range<u64>) {
        debug_assert_eq!(
            self.cache.generation,
            Some(self.generation),
            "EngineRows::new keys the cache before the table draws"
        );
        if visible.start >= self.cache.window.start && visible.end <= self.cache.window.end {
            return; // served from the held window
        }
        let total = self.cache.total;
        let start = visible.start.saturating_sub(PAGE_PAD).min(total);
        let end = visible.end.saturating_add(PAGE_PAD).min(total);
        if start >= end {
            return; // an empty materialisation has no window to fetch
        }
        match fetch_page(self.session, self.mark_index, start..end) {
            Ok(page) => {
                self.cache.window = page.window;
                self.cache.columns = page.columns;
                self.cache.rows = page.rows;
                self.cache.error = None;
            }
            Err(e) => {
                self.cache.error = Some(e.to_string());
            }
        }
    }

    fn total_rows(&self) -> u64 {
        self.cache.total
    }

    fn columns(&self) -> &[GridColumn] {
        &self.cache.columns
    }

    fn cell_text(&self, row: u64, col: usize) -> Option<CellText> {
        let offset = row.checked_sub(self.cache.window.start)?;
        let row = self.cache.rows.get(usize::try_from(offset).ok()?)?;
        row.get(col).cloned()
    }
}

// ---------------------------------------------------------------------------
// The Steps-sheet source — the same chrome over the in-memory sheet.
// ---------------------------------------------------------------------------

/// The protocol Steps sheet as a [`RowSource`]: the sheet core stays in
/// `brightfield-protocol` untouched; this adapter is only the projection into
/// the one table chrome. The order column is numeric — right-aligned in the
/// tabular-numeral scope — where the retired render set it in a monospace
/// face, which is exactly the alignment-by-font the density guideline forbids.
pub struct StepSheetRows<'a> {
    rows: &'a [StepRow],
    cursor: Option<u64>,
    columns: Vec<GridColumn>,
}

impl<'a> StepSheetRows<'a> {
    /// A source over the sheet's rows with `cursor` wearing the wash.
    #[must_use]
    pub fn new(rows: &'a [StepRow], cursor: usize) -> Self {
        let columns = sheet::COLUMNS
            .iter()
            .enumerate()
            .map(|(i, name)| GridColumn {
                name: (*name).to_string(),
                numeric: i == 0,
            })
            .collect();
        Self {
            rows,
            cursor: (cursor < rows.len()).then_some(cursor as u64),
            columns,
        }
    }
}

impl RowSource for StepSheetRows<'_> {
    fn prepare(&mut self, _visible: Range<u64>) {}

    fn total_rows(&self) -> u64 {
        self.rows.len() as u64
    }

    fn columns(&self) -> &[GridColumn] {
        &self.columns
    }

    fn selected_row(&self) -> Option<u64> {
        self.cursor
    }

    fn cell_text(&self, row: u64, col: usize) -> Option<CellText> {
        let row = self.rows.get(usize::try_from(row).ok()?)?;
        Some(match col {
            0 => CellText::primary(row.order.to_string()),
            1 => CellText::primary(if row.gate {
                format!("{} ◈", row.name)
            } else {
                row.name.clone()
            }),
            2 => CellText::secondary(row.kind),
            3 => CellText::secondary(truncate(&row.detail, 40)),
            4 => CellText::secondary(row.status),
            _ => CellText::muted(
                row.live_state
                    .clone()
                    .or_else(|| row.skip_reason.clone())
                    .unwrap_or_default(),
            ),
        })
    }
}

/// Ellipsise `s` to at most `max` characters.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

// ---------------------------------------------------------------------------
// The Data pane.
// ---------------------------------------------------------------------------

/// The data grid — the chart's peer in the centre tab strip.
pub const DATA: ItemId = ItemId::new("chart-data-grid");

/// The pane's icon name, from the Meridian icon set.
const ICON_DATA: Icon = Icon("table");

/// The Data pane's registry entry: a centre tab beside the chart — the ONE
/// toggle between the step's two projections.
#[must_use]
pub fn data_grid_spec() -> ItemSpec<ChartDoc> {
    ItemSpec {
        id: DATA,
        slot: Slot::CentreTab,
        toggle: Some(Verb::new("toggle-data-grid")),
        make: || Box::new(DataGridItem::new()),
    }
}

/// The grid pane. Holds only view-local state — the page cache — per the
/// workbench aliasing rule; the session it reads belongs to the document and
/// is borrowed for the duration of one draw.
pub struct DataGridItem {
    cache: GridCache,
}

impl DataGridItem {
    /// A grid pane with nothing fetched.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: GridCache::default(),
        }
    }

    /// How many rows the pane holds client-side — test access to the bound.
    #[must_use]
    pub fn held_rows(&self) -> usize {
        self.cache.held_rows()
    }
}

impl Default for DataGridItem {
    fn default() -> Self {
        Self::new()
    }
}

impl Item<ChartDoc> for DataGridItem {
    fn item_id(&self) -> ItemId {
        DATA
    }

    fn empty_state(&self, doc: &ChartDoc) -> Option<EmptyState> {
        // Empty means NOTHING IS OPEN — the same predicate the chart answers
        // with, so the two peers agree about whether there is a document at
        // all. A composed-but-not-live dashboard is not empty; that case is
        // the body's to explain (see `ui`), the way the canvas pane renders
        // a quiet surface rather than an apology over a headless document.
        doc.is_empty().then(|| {
            EmptyState::new(
                ICON_DATA,
                "No data to tabulate",
                "This grid shows the rows behind the chart beside it. Open a \
                 spec from the chart pane.",
            )
        })
    }

    /// No run-state line here, and that is the whole of it: the run state is
    /// the *document's*, and the chart pane declares it for the view. Both
    /// panes read `doc.composed.run_state` and both draw `run_state_pill` in
    /// their own body — but the rail collects the status lines of each placed
    /// pane, so two declarations of one document-level fact draw the same line
    /// twice. `brightfield_shell::chart_item` carries the argument for why the
    /// centre pane is the one that owns it, and
    /// `tests/rail_entry_ids.rs` reddens if a second declaration reappears.
    ///
    /// What this pane says about the run state, it says in `ui` below, where
    /// a state the record cannot vouch for stops the rows being fetched.
    /// Dropping the rail entry costs the grid nothing it was telling anyone.
    fn describe(&self, _doc: &ChartDoc) -> Subject {
        Subject::new("Data", ICON_DATA, BindingContext::Workspace)
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        let mode = cx.mode;
        let sem = semantic(mode.is_dark());

        // The honesty gate, before any row is fetched: when the preview
        // carries a recorded run-state, render it — and when that state says
        // the materialisation is not there to be trusted at all (never run,
        // or the run failed), the state IS the content. Materialised rows are
        // never presented as current unless the record proves currency;
        // stale rows render, but only under the stale banner beside them.
        if let Some(state) = doc.composed.run_state {
            run_state_pill(ui, state);
            if !(state.is_current() || state.is_stale()) {
                ui.add_space(spacing::SPACE_2);
                ui.label(
                    egui::RichText::new(state.gloss())
                        .font(cell_font())
                        .color(chrome::colour(sem.text.muted)),
                );
                return;
            }
        }

        if doc.live_coordinator().is_none() {
            // A one-shot composition (a capture, a shipped still) carries no
            // session, so there is nothing to query — and pretending
            // otherwise with a frozen copy would be exactly the client-side
            // grid this pane exists not to be. Say so, quietly.
            ui.label(
                egui::RichText::new(
                    "A still frame — this dashboard was composed without a \
                     live query session, so its rows cannot be read back.",
                )
                .font(cell_font())
                .color(chrome::colour(sem.text.muted)),
            );
            return;
        }
        if doc
            .live_coordinator()
            .is_some_and(|c| c.session().mark_count() == 0)
        {
            ui.label(
                egui::RichText::new("This spec declares no marks, so no step to tabulate.")
                    .font(cell_font())
                    .color(chrome::colour(sem.text.muted)),
            );
            return;
        }

        // The step this grid projects is the one the chart pane presents:
        // depth-first mark 0, the dashboard's presenting step. The read marks
        // itself as engine work — resolved within the frame today (see the
        // activity module for why a synchronous read owes no cue), begun and
        // ended around the borrow so the seam is already marked when this
        // read moves off the UI thread.
        doc.activity.begin(Activity::EngineQuery);
        if let Some(coordinator) = doc.live_coordinator() {
            let generation = coordinator.generation();
            let session = coordinator.session();
            let mut source = EngineRows::new(session, 0, generation, &mut self.cache);
            show_table(ui, "chart-data-grid", mode, &mut source);
        }
        doc.activity.end(Activity::EngineQuery);

        if self.cache.error.is_none() && self.cache.total == 0 {
            // A real answer, not an empty pane: the query ran and selected
            // nothing under the current interaction state.
            ui.label(
                egui::RichText::new("0 rows under the current selection.")
                    .font(cell_font())
                    .color(chrome::colour(sem.text.muted)),
            );
        }
        if let Some(error) = self.cache.error.as_ref() {
            ui.add_space(spacing::SPACE_2);
            ui.label(
                egui::RichText::new(format!("Grid read failed: {error}"))
                    .font(cell_font())
                    .color(chrome::colour(sem.text.muted)),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests — the pure pieces; the engine-backed path is exercised in
// `tests/data_grid.rs` against a live session.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The table-scope feature declaration is exactly the token pair, from
    /// the token layer — tabular numerals plus slashed zero, nothing else,
    /// nothing invented here.
    #[test]
    fn numeric_features_are_the_design_tokens_at_table_scope() {
        assert_eq!(NUMERIC_FEATURES, [typography::TNUM, typography::ZERO]);
        assert_eq!(&NUMERIC_FEATURES[0].tag, b"tnum");
        assert_eq!(NUMERIC_FEATURES[0].value, 1);
        assert_eq!(&NUMERIC_FEATURES[1].tag, b"zero");
        assert_eq!(NUMERIC_FEATURES[1].value, 1);
    }

    /// Table scope, never globally: the global egui style this shell installs
    /// declares no OpenType features anywhere — the pair above exists only on
    /// this module's numeric-cell declaration.
    #[test]
    fn the_global_style_carries_no_font_features() {
        // egui's Style has no feature field at all (see the module note on
        // epaint's shaper), so "globally declared features" cannot exist by
        // construction — the assertion that matters is that the CELL font is
        // the proportional UI face, not a monospace one smuggling alignment.
        let font = cell_font();
        assert_eq!(font.family, egui::FontFamily::Proportional);
        assert_eq!(font.size, typography::UI_SIZE);
    }

    /// The steps adapter: order is the one numeric column, the cursor maps to
    /// the wash, a gate wears its shield, and the columns are the sheet's own.
    #[test]
    fn step_sheet_rows_project_the_sheet_into_the_chrome() {
        let rows = vec![
            StepRow {
                order: 0,
                name: "load".into(),
                label: "load".into(),
                kind: "sql",
                detail: "models/load.sql".into(),
                status: "ok",
                live_state: None,
                skip_reason: Some("hash_clean".into()),
                gate: false,
            },
            StepRow {
                order: 1,
                name: "validate".into(),
                label: "validate".into(),
                kind: "op",
                detail: "finetype_validate".into(),
                status: "ok",
                live_state: None,
                skip_reason: None,
                gate: true,
            },
        ];
        let source = StepSheetRows::new(&rows, 1);
        assert_eq!(source.total_rows(), 2);
        assert_eq!(source.selected_row(), Some(1));
        let names: Vec<&str> = source.columns().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, sheet::COLUMNS);
        let numeric: Vec<bool> = source.columns().iter().map(|c| c.numeric).collect();
        assert_eq!(
            numeric,
            [true, false, false, false, false, false],
            "the order column is numeric — right-aligned tabular figures, \
             never the monospace face the retired render used"
        );
        assert_eq!(source.cell_text(0, 0).unwrap().text, "0");
        assert_eq!(
            source.cell_text(1, 1).unwrap().text,
            "validate ◈",
            "a gate wears its shield"
        );
        assert_eq!(
            source.cell_text(0, 5).unwrap().text,
            "hash_clean",
            "the live column falls back to the skip reason"
        );
        assert_eq!(source.cell_text(2, 0), None, "past the sheet is no cell");
    }

    /// A cursor past the rows wears no wash rather than washing a phantom row.
    #[test]
    fn an_out_of_range_cursor_selects_nothing() {
        let source = StepSheetRows::new(&[], 0);
        assert_eq!(source.selected_row(), None);
        assert_eq!(source.total_rows(), 0);
    }

    /// The audit's requirements on this pane over an empty document, checked
    /// here as well so a failure names this file: a real empty state, house
    /// style prose.
    #[test]
    fn the_grid_answers_an_empty_document_with_a_real_empty_state() {
        let item = DataGridItem::new();
        let doc = ChartDoc::empty();
        let empty = item.empty_state(&doc).expect("an empty doc has an answer");
        assert_eq!(empty.headline, "No data to tabulate");
        let subject = item.describe(&doc);
        assert_eq!(subject.title, "Data");
    }

    /// The run-state honesty gate in `ui` keys off the vocabulary's own
    /// predicates: exactly Fresh presents as current, exactly the two stales
    /// present rows under a banner, and the remaining two states present no
    /// rows at all.
    #[test]
    fn the_row_presenting_states_are_exactly_fresh_and_stale() {
        use brightfield_workbench::subject::RunState;
        let presents_rows: Vec<RunState> = RunState::ALL
            .into_iter()
            .filter(|s| s.is_current() || s.is_stale())
            .collect();
        assert_eq!(
            presents_rows,
            [
                RunState::Fresh,
                RunState::StaleOwnEdit,
                RunState::StaleUpstream
            ]
        );
    }
}
