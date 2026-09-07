//! MarkRenderer trait and implementations for dot, bar, and line marks.
//!
//! Each renderer consumes a RecordBatch + ChannelMap + ScaleSet and produces
//! Vello scene fragments (fill/stroke operations).

use arrow::array::{Array, Float64Array, StringArray, TimestampMicrosecondArray};
use arrow::datatypes::{DataType, TimeUnit};
use arrow::record_batch::RecordBatch;
use brightfield_spec::layout::DEFAULT_GEOMETRY_COLUMN;
use brightfield_spec::vocab::MarkKind;
use kurbo::{Affine, BezPath, Circle, Line, Rect};
use peniko::{Color, Fill};
use vello::Scene;

use crate::channel::{Channel, ChannelMap, LabelForm};
use crate::ink::ChartInk;
use crate::kde::{kde_1d_weighted, kde_2d, silverman_1d_weighted, silverman_2d_per_axis};
use crate::scale::{
    apply_colour_override, merge_linear_scale, ColourOverride, Scale, ScaleSet, SequentialScheme,
};
use crate::text::{draw_text, TextAnchor};

/// The `otherwise` override a highlight applies to its NON-matching
/// (deemphasised) rows — the flat Mosaic `Highlight` surface. Every
/// field is optional; an all-`None` style falls back to the Mosaic default
/// (`opacity` 0.2). `opacity`/`fill_opacity` SCALE the resolved alpha; `fill`
/// REPLACES the resolved RGB. `stroke`/`stroke_opacity` are modelled for corpus
/// fidelity but unimplemented at the render site (no fill-vs-stroke
/// discriminator, no driving fixture).
#[derive(Clone, Debug, Default)]
pub struct HighlightStyle {
    /// Element-opacity multiplier for deemphasised rows (Mosaic default 0.2).
    pub opacity: Option<f64>,
    /// Literal fill colour replacing the resolved RGB (e.g. `#ccc`).
    pub fill: Option<Color>,
    /// Fill-alpha multiplier for deemphasised rows.
    pub fill_opacity: Option<f64>,
    /// Modelled, unimplemented in v1.
    pub stroke: Option<Color>,
    /// Modelled, unimplemented in v1.
    pub stroke_opacity: Option<f64>,
}

impl From<&brightfield_spec::analysis::HighlightStyle> for HighlightStyle {
    /// Resolve a spec-side highlight `otherwise` style (CSS-hex colour strings)
    /// into the render style (parsed `Color`s). Shared by app assembly
    /// AND the command-log mark rebuild, so a re-queried mark dims
    /// identically after a structural edit / undo (finding 1/2/4 — a rebuild that
    /// dropped this silently killed highlight dimming).
    fn from(style: &brightfield_spec::analysis::HighlightStyle) -> Self {
        HighlightStyle {
            opacity: style.opacity,
            fill: style.fill.as_deref().and_then(parse_css_hex),
            fill_opacity: style.fill_opacity,
            stroke: style.stroke.as_deref().and_then(parse_css_hex),
            stroke_opacity: style.stroke_opacity,
        }
    }
}

/// Default deemphasis alpha multiplier when a highlight carries no override
/// fields — Mosaic's `opacity` default for the non-matching set.
const DEFAULT_DIMMED_ALPHA: f32 = 0.2;

/// Highlight state for per-row dim/emphasis rendering.
///
/// When active, rows where `predicate(row_index)` returns `true` render
/// untouched (the SELECTED set keeps its normal appearance); rows where it
/// returns `false` are deemphasised per `otherwise` — the Mosaic `highlight`
/// semantics.
pub struct HighlightState {
    /// Predicate: returns `true` for rows that should render untouched.
    pub predicate: Box<dyn Fn(usize) -> bool + Send + Sync>,
    /// The override applied to non-matching (deemphasised) rows.
    pub otherwise: HighlightStyle,
}

/// Reserved column carrying the per-GROUP count of selected rows, as emitted by
/// `brightfield-sql`'s `SELECTED_COUNT_COLUMN`. The two literals must agree; the
/// same duplication `__bf_count` already carries across this boundary, and for
/// the same reason — this crate does not depend on `brightfield-sql`.
const SELECTED_COUNT_COLUMN: &str = "__bf_selected_count";

/// Build a per-row [`HighlightState`] from a re-queried batch's reserved
/// membership column and the mark's resolved `otherwise` style. Returns `None`
/// when the batch carries neither — the at-rest / empty-selection case, so every
/// row renders normally — or when the column it does carry has the wrong type.
///
/// Two forms, one for each shape a query can come back in.
///
/// * [`brightfield_spec::analysis::SELECTED_COLUMN`], a per-row boolean, for a
///   mark whose rows are rows. The booleans are copied into an owned `Vec` the
///   predicate closure captures, so the returned state is self-contained
///   (`Send + Sync + 'static`). A NULL membership (a predicate over a NULL
///   column) reads as not-selected → deemphasised, matching Mosaic (only rows
///   the predicate proves in stay lit).
/// * The private `SELECTED_COUNT_COLUMN`, a per-group count, for a mark whose
///   rows are groups. There is no whole element to keep lit here — a group is
///   selected in PART — so every group reads as non-matching and the mark is
///   deemphasised whole, which is Mosaic's `highlight` reading of a group. What
///   this state adds over the batch alone is the author's `otherwise`: a
///   renderer that can draw a part of its own shape reads the counts straight
///   off the batch either way.
#[must_use]
pub fn build_highlight_state(
    batch: &RecordBatch,
    style: &HighlightStyle,
) -> Option<HighlightState> {
    use arrow::array::BooleanArray;
    if let Ok(idx) = batch
        .schema()
        .index_of(brightfield_spec::analysis::SELECTED_COLUMN)
    {
        let col = batch.column(idx).as_any().downcast_ref::<BooleanArray>()?;
        let selected: Vec<bool> = (0..col.len())
            .map(|i| !col.is_null(i) && col.value(i))
            .collect();
        return Some(HighlightState {
            predicate: Box::new(move |row| selected.get(row).copied().unwrap_or(false)),
            otherwise: style.clone(),
        });
    }
    batch.schema().index_of(SELECTED_COUNT_COLUMN).ok()?;
    Some(HighlightState {
        predicate: Box::new(|_| false),
        otherwise: style.clone(),
    })
}

/// The per-group selected counts a batch carries, read once. `None` for a batch
/// without the column, which is every mark whose rows are rows and every mark at
/// rest.
///
/// Read off the BATCH rather than off a [`HighlightState`], the way `__bf_count`
/// and the hexbin geometry columns are: the column is projected only under a
/// live highlight, so its presence is the signal, and a renderer handed the
/// batch can act on it whether or not the caller also resolved the author's
/// deemphasis style.
fn selected_counts(batch: &RecordBatch) -> Option<Vec<Option<f64>>> {
    column_as_f64(batch, SELECTED_COUNT_COLUMN)
}

/// The ink for the part of a bar the selection did NOT account for: the mark's
/// own colour, deemphasised.
///
/// Takes the author's `otherwise` when the render call was handed a
/// [`HighlightState`], and the module default when it was not — the same default
/// [`apply_highlight`] applies to a non-matching row of a per-row mark.
fn remainder_ink(ink: Color, row: usize, highlight: Option<&HighlightState>) -> Color {
    match highlight {
        Some(_) => apply_highlight(ink, row, highlight),
        None => deemphasise(ink, &HighlightStyle::default()),
    }
}

/// The fraction of row `row`'s group the selection accounts for, given the
/// group's own drawn value — `None` when the mark carries no per-group counts,
/// when this group has none, or when the total is not a positive number to take
/// a fraction of.
///
/// Clamped to `[0, 1]`: the selected rows of a group are a subset of it, so a
/// ratio above one would be a fraction of something the bar is not drawn from.
fn selected_fraction_of(counts: Option<&Vec<Option<f64>>>, row: usize, total: f64) -> Option<f64> {
    let selected = (*counts?.get(row)?)?;
    (total > 0.0 && selected > 0.0).then(|| (selected / total).min(1.0))
}

/// Floor on the pixel extent of a part-of-whole overdraw that stands for a
/// non-empty selection.
///
/// A sub-pixel rectangle rasterises as partial coverage, so a selection small
/// enough fades toward invisible — and invisible is the reading that means no
/// selection at all, which is the confusion this treatment exists to remove. A
/// floor makes a tiny selection a hairline instead.
const MIN_SELECTED_EXTENT_PX: f64 = 0.25;

/// The far edge of the part-of-whole overdraw: `fraction` of the way from a
/// bar's baseline at `base` to its tip at `tip`, in pixels.
///
/// Floored at [`MIN_SELECTED_EXTENT_PX`] in the direction the bar grows, and
/// never past `tip` — a bar that is itself sub-pixel gets its whole extent
/// rather than an overdraw longer than the bar it is part of.
fn selected_tip(base: f64, tip: f64, fraction: f64) -> f64 {
    let span = tip - base;
    let drawn = span * fraction;
    let floor = MIN_SELECTED_EXTENT_PX.min(span.abs());
    if drawn.abs() >= floor {
        base + drawn
    } else {
        base + floor.copysign(span)
    }
}

/// Parse a CSS hex colour (`#rgb`, `#rgba`, `#rrggbb`, or `#rrggbbaa`) into a
/// [`Color`]. `None` for any other form (a named colour, `none`, or malformed
/// hex) — the caller then leaves the resolved colour's RGB untouched. Alpha
/// defaults to opaque when not given.
///
/// The four accepted lengths are exactly the four
/// `brightfield_spec::vocab::is_colour_literal` recognises. They have to match:
/// a form classified as a colour but not parseable here is a spec the chart
/// silently draws in the wrong ink.
#[must_use]
pub fn parse_css_hex(s: &str) -> Option<Color> {
    let hex = s.trim().strip_prefix('#')?;
    let to = |b: &[u8]| -> Option<f32> {
        let s = std::str::from_utf8(b).ok()?;
        Some(u8::from_str_radix(s, 16).ok()? as f32 / 255.0)
    };
    // A single hex nibble in `#rgb` expands to two identical nibbles (`c` → `cc`),
    // i.e. the byte value `nibble * 17` (0x11).
    let dup = |c: u8| -> Option<f32> {
        let s = std::str::from_utf8(std::slice::from_ref(&c)).ok()?;
        Some(u8::from_str_radix(s, 16).ok()? as f32 * 17.0 / 255.0)
    };
    let b = hex.as_bytes();
    match b.len() {
        3 => Some(Color::new([dup(b[0])?, dup(b[1])?, dup(b[2])?, 1.0])),
        4 => Some(Color::new([dup(b[0])?, dup(b[1])?, dup(b[2])?, dup(b[3])?])),
        6 => Some(Color::new([
            to(&b[0..2])?,
            to(&b[2..4])?,
            to(&b[4..6])?,
            1.0,
        ])),
        8 => Some(Color::new([
            to(&b[0..2])?,
            to(&b[2..4])?,
            to(&b[4..6])?,
            to(&b[6..8])?,
        ])),
        _ => None,
    }
}

/// Resolve a colour-channel **constant** — what a spec means by
/// `fill: steelblue`, `fill: '#ccc'` or `stroke: none` — into the ink to paint.
///
/// Hex first (this crate's own parser), then the CSS keyword table in
/// `brightfield-spec`, which is the same table
/// [`brightfield_spec::vocab::is_colour_literal`] classifies against — so a
/// string classified as a colour and a string paintable as one cannot drift
/// apart in the keyword direction.
///
/// `None` means **recognised but not resolvable here**, and the caller keeps
/// whatever default it had. Two forms land there deliberately:
///
/// - `currentColor`, which names an inherited text colour from a CSS cascade
///   this renderer does not have.
/// - functional notation — `rgb(…)`, `hsl(…)`, `lab(…)` — which
///   `is_colour_literal` accepts by shape (so a binned rect binding one is
///   correctly read as carrying no groups) and nothing here parses.
///
/// Both are gaps, not decisions, and a spec that writes one gets the default
/// mark ink rather than an invented colour.
#[must_use]
pub fn parse_colour_literal(s: &str) -> Option<Color> {
    if let Some(c) = parse_css_hex(s) {
        return Some(c);
    }
    let [r, g, b, a] = brightfield_spec::vocab::css_colour_keyword_rgb(s)?;
    Some(Color::new([
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ]))
}

/// Trait for per-mark-family rendering.
///
/// Each implementation produces Vello scene fragments from Arrow data
/// mapped through scales.
pub trait MarkRenderer {
    /// Render the mark into the given scene.
    ///
    /// When `highlight` is `Some`, matching rows render untouched;
    /// non-matching rows are deemphasised per the highlight's `otherwise` style.
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    );

    /// Render the mark knowing its query could **not** be narrowed to the
    /// plot's frame — it still summarises rows that are off screen.
    ///
    /// A navigated plot filters the marks whose plans can carry a bound; a
    /// scalar aggregate with no grouping key beneath it cannot, so it returns
    /// the byte-identical row it returned at full extent and its drawing is
    /// clipped at the frame edge. What a reader then sees is a fit that looks
    /// exactly like a fit over the visible points. The panel says otherwise in
    /// a sentence, but a screenshot carries the picture and drops the sentence,
    /// and the picture is what people keep. So the difference has to be in the
    /// data ink.
    ///
    /// **The default is deliberately `render` unchanged, and that is a hazard
    /// worth naming.** Most marks are row-level: they rescope, this is never
    /// called for them, and inventing a treatment they would never wear is
    /// worse than nothing. But it means a mark that *should* distinguish itself
    /// and does not looks perfectly healthy from here. The obligation travels
    /// with the mark: if a summarising mark is added — and `declined` names
    /// mark kinds, not just this one — it owes an override, and the test that
    /// holds this one (`the_unrescoped_fit_is_dashed_in_the_exported_picture`)
    /// is the shape to copy.
    fn render_beyond_frame(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    ) {
        self.render(scene, batch, channel_map, scales, highlight);
    }

    /// Render with interpolation between previous and current positions.
    ///
    /// `prev_positions` are pixel (x, y) pairs from the previous frame.
    /// `t` is the interpolation factor (0.0 = prev, 1.0 = current).
    /// Default implementation forwards to `render()`, ignoring interpolation.
    ///
    /// 8 arguments against clippy's threshold of 7. Every one is a distinct
    /// input the renderer needs, and the two extra over `render` are precisely
    /// what "interpolated" means (`prev_positions` and `t`). Bundling them into
    /// a context struct would change every implementor and every call site of a
    /// public trait to satisfy an arbitrary count.
    #[allow(clippy::too_many_arguments)]
    fn render_interpolated(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _prev_positions: &[(f64, f64)],
        _t: f64,
        highlight: Option<&HighlightState>,
    ) {
        self.render(scene, batch, channel_map, scales, highlight);
    }

    /// The channel whose value-axis domain must include zero for this mark to
    /// render correctly — e.g. bars baseline at zero, so a domain of [10, 30]
    /// would otherwise place the baseline far below the plot. `None` for marks
    /// that don't need a zero baseline. The scene builder extends the named
    /// scale's domain to include 0 before rendering.
    fn zero_baseline_channel(&self) -> Option<Channel> {
        None
    }

    /// Contribute positional scales this mark needs but that generic column
    /// inference can't supply from the executed batch. The scene builders call
    /// this once per mark after `infer_scales`/`infer_scales_multi`, before
    /// rendering, passing the plot-area pixel ranges.
    ///
    /// Most marks need nothing here — their x/y bind to inferable columns, so
    /// the default is a no-op. Statistical marks transform their data, leaving a
    /// positional axis with no inferable column:
    ///   - regression emits only coefficients, so its x/y domains come from the
    ///     emitted `x_min`/`x_max`/`y_min`/`y_max` extents;
    ///   - 1D density has no data column on the perpendicular "density" axis.
    fn augment_scales(
        &self,
        _scales: &mut ScaleSet,
        _batch: &RecordBatch,
        _channel_map: &ChannelMap,
        _x_range: (f64, f64),
        _y_range: (f64, f64),
    ) {
    }

    /// Whether this mark suppresses the plot frame — the grid, axes, and tick
    /// labels. A geo/map mark projects its own coordinate space and reads as a
    /// map, not a cartesian plot, so it draws no axes or gridlines. Defaults to
    /// `false` (mirrors [`Self::zero_baseline_channel`]: zero impact on existing
    /// renderers). The scene builders skip the frame when any entry returns
    /// `true` (geo mark).
    fn suppresses_frame(&self) -> bool {
        false
    }
}

/// Default dot radius in pixels.
const DOT_RADIUS: f64 = 4.0;

// The default mark colour — Meridian Harbour slot 1 blue, replacing the former
// Tableau10 blue `#4e79a7` (which an old comment mislabelled CSS steelblue; CSS
// steelblue is `#4682b4` — neither survives here) — is
// [`ChartInk::mark_default`], and the NULL ink is [`ChartInk::null`]. Both are
// read off the scale set a renderer is already handed, so a mark draws the mode
// the plot is in without a single `MarkRenderer` signature moving.
//
// NULL ink is a warm gray deliberately below the series chroma floor, so a NULL
// can never impersonate a scheme colour (it used to fall through to the default
// at full opacity and read as a HIGH value on light-anchored schemes). Reserved
// for genuine NULLs — every other fallthrough (no fill channel, no colour
// scale, unmapped category) keeps the default mark colour.

/// Default line stroke width.
const LINE_STROKE_WIDTH: f64 = 2.0;

/// The dash rhythm a mark wears when its query could not be narrowed to the
/// plot's frame: **6 px of ink, 4 px of gap**.
///
/// Neither number is chosen here. Both are named steps off the design system's
/// spacing ladder — `SPACE_3`, the icon-to-label gap, and `SPACE_2`, the base
/// unit — which is that ladder's whole purpose: a consumer that needs a length
/// picks a rung instead of inventing a fifth value. The rhythm is a texture and
/// not a colour, so it takes no colour token; see [`dash_polyline`] for why the
/// treatment is texture rather than hue.
const BEYOND_FRAME_DASH: f64 = meridian_design::spacing::SPACE_3 as f64;

/// The gap between dashes — see [`BEYOND_FRAME_DASH`].
const BEYOND_FRAME_GAP: f64 = meridian_design::spacing::SPACE_2 as f64;

/// The alpha a confidence band is drawn at — its fill, and its edge when that
/// edge is drawn.
///
/// One constant for both because they are one mark. The band is a WEAKER
/// statement than the fit it surrounds and has to stay weaker: it covers far
/// more of the plot, and an edge at the fit's own opacity would out-shout the
/// fit and read as a second, thicker pair of fitted lines.
///
/// Holding the edge here also keeps the band out of the measure
/// `the_unrescoped_fit_is_dashed_in_the_exported_picture` counts runs with.
/// That test asks whether the FIT is dashed and admits only pixels near full
/// mark colour; at this alpha the band composites nowhere near it, so the
/// measure still sees the fit alone. The hazard is worth naming precisely,
/// because it is not a failing test: that assertion is a lower bound on runs,
/// so an edge drawn at full opacity would be counted, the test would go on
/// PASSING, and it would quietly stop holding the thing it is named for.
const BAND_ALPHA: f32 = 0.20;

/// Cut a pixel-space polyline into the drawn runs of a dash pattern.
///
/// # Why a dash, and not desaturation or an end-cap
///
/// The job is to make a mark that could not rescope tell a reader so **in the
/// picture**, without them reading a word. Three treatments were on the table.
///
/// *Desaturation* is the one the design system argues against. `colour.md`'s
/// rules are rules about identity: colour follows the entity, never its rank,
/// and a filter that changes what is on screen must not repaint the survivors.
/// Fading a series off its assigned palette slot is exactly that repaint, and
/// it walks the mark toward `null_ink`, the one grey the system reserves for
/// "this value is NULL". The fit is not null and its colour has not changed
/// meaning. Desaturation also fails on its own terms: it is only visible beside
/// something at full chroma, and a lone fit line has no such neighbour in
/// frame.
///
/// *An end-cap or fade where the mark's true domain leaves the frame* is the
/// most literal statement — "my data continues past here" — and it is the one
/// this cannot do yet, for a reason worth writing down rather than rediscover-
/// ing. The wired signal (`NavigationFilterPass::declined`) names the AXIS
/// COLUMNS the extent could not be pushed into. It does not say which SIDE the
/// mark's data runs off, and deriving that from the batch would stand a second,
/// independent detection beside the first — two answers to one question is how
/// they drift apart. A cap also has to overcome its own ambiguity: a marker at
/// the frame edge reads about as easily as "the data stops here", which is the
/// opposite of the message.
///
/// *A dash* asks nothing of the colour channel. It invents no token, cannot
/// collide with a reserved status hue or with a second series in a grouped
/// regression, and needs no reference in frame to be read. It survives what a
/// screenshot survives — greyscale, colour-vision deficiency, and being scaled
/// down in someone's slide deck — which matters here more than usual, because
/// the whole premise of this treatment is that the picture travels and the
/// sentence beside it does not. It is also unspent: nothing else in this
/// renderer draws a dashed anything, so the vocabulary is free.
///
/// # What it does
///
/// Walks arc length across the whole polyline, so the rhythm is continuous over
/// segment joins rather than restarting at each one — a phase reset per segment
/// would bunch the dashes wherever the sampling happens to be dense.
fn dash_polyline(points: &[(f64, f64)], on: f64, off: f64) -> Vec<Line> {
    let mut out = Vec::new();
    if on <= 0.0 || off <= 0.0 {
        return out;
    }
    let period = on + off;
    let mut travelled = 0.0f64;
    for w in points.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        let len = (x1 - x0).hypot(y1 - y0);
        // NaN or a zero-length segment contributes nothing and must not enter
        // the walk; written as an inclusion so a NaN falls out rather than
        // through.
        if !(len.is_finite() && len > 0.0) {
            continue;
        }
        let mut s = 0.0f64;
        while s < len {
            let phase = (travelled + s) % period;
            // A floor on the step, so a run that rounds to nothing cannot spin
            // here. At 1e-6 px it is far below anything that reaches a pixel.
            let step = if phase < on {
                let run = (on - phase).min(len - s).max(1e-6);
                let (a, b) = (s / len, (s + run).min(len) / len);
                out.push(Line::new(
                    kurbo::Point::new(x0 + (x1 - x0) * a, y0 + (y1 - y0) * a),
                    kurbo::Point::new(x0 + (x1 - x0) * b, y0 + (y1 - y0) * b),
                ));
                run
            } else {
                (period - phase).min(len - s).max(1e-6)
            };
            s += step;
        }
        travelled += len;
    }
    out
}

/// Reserved output-column name the density lowerers emit for the per-bin
/// occupancy count. Kept distinct from any user column so a density positional
/// channel bound to a column literally named `count` can't collide with it (the
/// bin centre is aliased to the channel column name). Must match the alias in
/// brightfield-sql's `build_density_1d`/`build_density_2d`.
const DENSITY_COUNT_COL: &str = "__bf_count";

// ---------------------------------------------------------------------------
// Helpers: extract f64 values from columns regardless of source type
// ---------------------------------------------------------------------------

pub(crate) fn column_as_f64(batch: &RecordBatch, col_name: &str) -> Option<Vec<Option<f64>>> {
    use arrow::array::{
        Float32Array, Int16Array, Int32Array, Int8Array, UInt16Array, UInt32Array, UInt64Array,
        UInt8Array,
    };
    let idx = batch.schema().index_of(col_name).ok()?;
    let col = batch.column(idx);

    macro_rules! cast_numeric {
        ($arr_ty:ty) => {{
            let arr = col.as_any().downcast_ref::<$arr_ty>()?;
            Some(
                (0..arr.len())
                    .map(|i| {
                        if arr.is_null(i) {
                            None
                        } else {
                            Some(arr.value(i) as f64)
                        }
                    })
                    .collect(),
            )
        }};
    }

    match col.data_type() {
        DataType::Float64 => cast_numeric!(Float64Array),
        DataType::Float32 => cast_numeric!(Float32Array),
        DataType::Int64 => cast_numeric!(arrow::array::Int64Array),
        DataType::Int32 => cast_numeric!(Int32Array),
        DataType::Int16 => cast_numeric!(Int16Array),
        DataType::Int8 => cast_numeric!(Int8Array),
        DataType::UInt64 => cast_numeric!(UInt64Array),
        DataType::UInt32 => cast_numeric!(UInt32Array),
        DataType::UInt16 => cast_numeric!(UInt16Array),
        DataType::UInt8 => cast_numeric!(UInt8Array),
        DataType::Timestamp(TimeUnit::Microsecond, _) => cast_numeric!(TimestampMicrosecondArray),
        _ => None,
    }
}

/// One column's values as the strings a band scale, a colour scale or a text
/// mark reads them by.
///
/// **`Date32` answers here as well as `Utf8`**, in the ISO spelling
/// `Scale::Band` collects its categories in. A `DATE` reaches this crate as
/// `Date32` and every band a mark draws is keyed by string, so without this arm
/// a date column bound to a band axis returned `None` here, the band scale was
/// never built, and the mark returned before laying down a single fill — a
/// plot with axes, gridlines and **no bars**, at exit 0. That is the failure
/// `tests/bar_orientation.rs` was written for, arriving by a different door.
fn column_as_string(batch: &RecordBatch, col_name: &str) -> Option<Vec<Option<String>>> {
    let idx = batch.schema().index_of(col_name).ok()?;
    let col = batch.column(idx);
    if let DataType::Date32 = col.data_type() {
        let arr = col.as_any().downcast_ref::<arrow::array::Date32Array>()?;
        return Some(
            (0..arr.len())
                .map(|i| {
                    if arr.is_null(i) {
                        None
                    } else {
                        arr.value_as_date(i).map(|d| d.to_string())
                    }
                })
                .collect(),
        );
    }
    if !matches!(col.data_type(), DataType::Utf8) {
        return None;
    }
    let arr = col.as_any().downcast_ref::<StringArray>().unwrap();
    Some(
        (0..arr.len())
            .map(|i| {
                if arr.is_null(i) {
                    None
                } else {
                    Some(arr.value(i).to_string())
                }
            })
            .collect(),
    )
}

/// Resolve the pixel position for a value given a channel's scale.
fn resolve_position(scale: &Scale, value_f64: Option<f64>, value_str: Option<&str>) -> Option<f64> {
    match scale {
        Scale::Linear { .. } | Scale::Time { .. } => value_f64.map(|v| scale.map_f64(v)),
        Scale::Band { .. } => value_str.and_then(|s| scale.map_category(s)),
        // Colour ramps (categorical or sequential) don't position on an axis.
        Scale::Colour { .. } | Scale::Sequential { .. } => None,
    }
}

/// Whether a mark's bound fill VALUE is genuinely NULL at `row` — read
/// type-agnostically off the Arrow validity bitmap, so a NULL in a string OR
/// numeric fill column is caught. `false` for an absent column (that is a
/// binding problem, not a NULL value — it keeps the default colour).
fn fill_value_is_null(batch: &RecordBatch, fill_col: &str, row: usize) -> bool {
    let Ok(idx) = batch.schema().index_of(fill_col) else {
        return false;
    };
    let col = batch.column(idx);
    row < col.len() && col.is_null(row)
}

/// Resolve the colour for a data point.
///
/// A `fill` bound to a colour CONSTANT (`fill: steelblue`, `fill: '#ccc'`) is
/// that colour for every row, and is checked first because it is not a column
/// and there is nothing per-row to look up. A bound fill whose value is
/// genuinely NULL at this row renders [`ChartInk::null`] rather than
/// [`ChartInk::mark_default`], which would impersonate a data value; the other
/// fallthroughs (no fill channel, no colour scale, unmapped category) keep the
/// default mark colour.
fn resolve_colour(
    scales: &ScaleSet,
    channel_map: &ChannelMap,
    batch: &RecordBatch,
    row: usize,
) -> Color {
    let ink = scales.ink();
    if let Some(constant) = channel_map.colour(Channel::Fill) {
        return constant;
    }
    if let Some(fill_col) = channel_map.get(Channel::Fill) {
        if let Some(fill_scale) = scales.get(Channel::Fill) {
            if let Some(strings) = column_as_string(batch, fill_col) {
                if let Some(Some(ref cat)) = strings.get(row) {
                    if let Some(components) = fill_scale.map_colour(cat) {
                        return Color::new(components);
                    }
                }
            }
        }
        if fill_value_is_null(batch, fill_col, row) {
            return ink.null;
        }
    }
    ink.mark_default
}

/// The constant ink a mark's spec named, if it named one — `stroke` first, then
/// `fill`, then [`ChartInk::mark_default`].
///
/// This is the *whole* colour story for the marks that draw one shape in one
/// colour and have no per-row colour path at all: line, rule, contour, the 1-D
/// density band. Each of those hard-coded [`ChartInk::mark_default`] before, which is
/// correct only for a spec that names no colour — and silently wrong for one
/// that does.
///
/// `stroke` outranks `fill` because these marks are drawn with a stroke; a spec
/// that sets both is naming the outline with the more specific channel.
/// Column-bound colour channels are deliberately NOT consulted here: mapping a
/// column to a stroke is a per-row question, these renderers emit one path, and
/// answering it from row 0 would be a guess wearing the clothes of a scale.
fn constant_ink(channel_map: &ChannelMap, ink: ChartInk) -> Color {
    channel_map
        .colour(Channel::Stroke)
        .or_else(|| channel_map.colour(Channel::Fill))
        .unwrap_or(ink.mark_default)
}

/// Apply a highlight's deemphasis to a resolved colour.
///
/// If highlight is active and the predicate returns `false` for this row
/// (non-matching), the `otherwise` override is applied: `fill` replaces the RGB,
/// `opacity`/`fill_opacity` scale the alpha, and — when the style carries NO
/// deemphasis field at all — the Mosaic default (alpha × 0.2) is used. A
/// matching row (predicate `true`) and the no-highlight case are returned
/// unchanged.
fn apply_highlight(colour: Color, row: usize, highlight: Option<&HighlightState>) -> Color {
    match highlight {
        Some(hs) if !(hs.predicate)(row) => deemphasise(colour, &hs.otherwise),
        _ => colour,
    }
}

/// Resolve the deemphasised colour for a non-matching row per an `otherwise`
/// style: `fill` replaces RGB; `opacity` and `fill_opacity` multiply the alpha;
/// with no field set, the Mosaic default alpha × 0.2 applies.
fn deemphasise(colour: Color, style: &HighlightStyle) -> Color {
    let [r, g, b, a] = colour.components;
    // fill replaces the RGB, keeping the resolved alpha as the starting point.
    let (r, g, b) = match style.fill {
        Some(f) => {
            let [fr, fg, fb, _] = f.components;
            (fr, fg, fb)
        }
        None => (r, g, b),
    };
    // opacity / fillOpacity scale the alpha (SVG semantics); if the author gave
    // no deemphasis field at all, fall back to the Mosaic default multiplier.
    let mut alpha = a;
    let mut any = false;
    if let Some(op) = style.opacity {
        alpha *= op as f32;
        any = true;
    }
    if let Some(fo) = style.fill_opacity {
        alpha *= fo as f32;
        any = true;
    }
    if style.fill.is_some() {
        any = true;
    }
    if !any {
        alpha *= DEFAULT_DIMMED_ALPHA;
    }
    Color::new([r, g, b, alpha])
}

// ---------------------------------------------------------------------------
// DotRenderer
// ---------------------------------------------------------------------------

/// The pixel position of one dot — through the mark's projection when it has
/// one, and through the scale directly when it does not.
///
/// A projected dot reads its two coordinate columns as NUMBERS rather than as
/// strings: a longitude is not a category, and `resolve_position`'s band lookup
/// has no work to do on a projected axis. `None` means the row does not draw —
/// either a coordinate is missing, or the projection has no position for it (see
/// [`Projection::project`]).
#[allow(clippy::too_many_arguments)]
fn dot_position(
    projection: Option<Projection>,
    x_scale: &Scale,
    y_scale: &Scale,
    xf: Option<f64>,
    xs: Option<&str>,
    yf: Option<f64>,
    ys: Option<&str>,
) -> Option<(f64, f64)> {
    if let Some(projection) = projection {
        let (u, v) = projection.project(xf?, yf?)?;
        return Some((x_scale.map_f64(u), y_scale.map_f64(v)));
    }
    Some((
        resolve_position(x_scale, xf, xs)?,
        resolve_position(y_scale, yf, ys)?,
    ))
}

/// Renders dot/scatter marks as circles at x/y positions.
pub struct DotRenderer;

impl MarkRenderer for DotRenderer {
    /// Equal-aspect the X/Y domains when the mark asked for it
    /// ([`ChannelMap::equal_aspect`]) — the point-map's device, a `dot` with
    /// `aspectRatio: 1`, and a no-op for a `dot` mark that did not ask,
    /// scatter included — held by
    /// `augment_scales_without_the_flag_leaves_scales_untouched` in this
    /// module's own tests.
    ///
    /// Reuses `aspect_fit_domains`, the same equal-px-per-unit fit
    /// [`GeoRenderer::augment_scales`] computes from a projected geometry
    /// bbox — here the bbox is the domain generic column inference already
    /// wrote into `scales` before this runs, so no geometry parsing is
    /// needed. Widening is idempotent (a domain already fit to the pixel
    /// ranges maps back to itself, since its own `du`/`dv` already equal the
    /// pixel-range-implied span), so it does not matter that a two-layer tile
    /// (ghost + subset) calls this once per layer.
    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        _batch: &RecordBatch,
        channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        // A projected mark aspect-fits for the same reason an equal-aspect one
        // does, and by the same arithmetic: the difference is the UNITS its
        // domains are already in, which `infer_scales` decided — degrees for an
        // equal-aspect mark, the projection's planar units for a projected one
        // (`project_positional_domains`). Neither may be true of the other, and
        // `ChannelMap::equal_aspect` is what guarantees it: it answers `false`
        // whenever a projection is set, so a mark that wrote both takes this
        // branch once, through the projection, rather than widening degrees
        // against projected units.
        if !(channel_map.equal_aspect() || channel_map.projection().is_some()) {
            return;
        }
        let (
            Some(Scale::Linear {
                domain_min: x0,
                domain_max: x1,
                ..
            }),
            Some(Scale::Linear {
                domain_min: y0,
                domain_max: y1,
                ..
            }),
        ) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            // No linear pair to fit — a table with nothing bound on one axis
            // draws nothing either way, and there is no bbox to widen.
            return;
        };
        let bbox = (*x0, *x1, *y0, *y1);
        let ((nx0, nx1), (ny0, ny1)) = aspect_fit_domains(bbox, x_range, y_range);
        merge_linear_scale(scales, Channel::X, nx0, nx1, x_range);
        merge_linear_scale(scales, Channel::Y, ny0, ny1, y_range);
    }

    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        // The graticule first, so the points sit on top of it: it is
        // scaffolding under the data, not data.
        //
        // Its extent comes off the SCALE SET, not off this mark's batch. The
        // scales are the plot's — one set shared by every layer — so a point
        // map's ghost and its brushed subset compute the same lines and lay them
        // down on top of each other. Read per batch, the brushed layer's extent
        // is the selection's, so it picks a finer step off the ladder and draws
        // a second, denser graticule over the region the reader swept.
        let projection = channel_map.projection();
        if let Some(projection) = projection {
            if let Some(extent) = scales.geo_extent() {
                let lines = graticule(projection, extent);
                stroke_graticule(scene, &lines, x_scale, y_scale, scales.ink().grid);
            }
        }

        let x_f64 = column_as_f64(batch, x_col);
        let x_str = column_as_string(batch, x_col);
        let y_f64 = column_as_f64(batch, y_col);
        let y_str = column_as_string(batch, y_col);

        let n = batch.num_rows();
        for i in 0..n {
            let xf = x_f64.as_ref().and_then(|v| v[i]);
            let xs = x_str.as_ref().and_then(|v| v[i].as_deref());
            let yf = y_f64.as_ref().and_then(|v| v[i]);
            let ys = y_str.as_ref().and_then(|v| v[i].as_deref());

            let Some((px, py)) = dot_position(projection, x_scale, y_scale, xf, xs, yf, ys) else {
                continue;
            };

            let colour = resolve_colour(scales, channel_map, batch, i);
            let colour = apply_highlight(colour, i, highlight);
            let circle = Circle::new((px, py), DOT_RADIUS);
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &circle);
        }
    }

    fn render_interpolated(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        prev_positions: &[(f64, f64)],
        t: f64,
        highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let x_f64 = column_as_f64(batch, x_col);
        let x_str = column_as_string(batch, x_col);
        let y_f64 = column_as_f64(batch, y_col);
        let y_str = column_as_string(batch, y_col);

        let n = batch.num_rows();
        for i in 0..n {
            let xf = x_f64.as_ref().and_then(|v| v[i]);
            let xs = x_str.as_ref().and_then(|v| v[i].as_deref());
            let yf = y_f64.as_ref().and_then(|v| v[i]);
            let ys = y_str.as_ref().and_then(|v| v[i].as_deref());

            let Some((target_px, target_py)) =
                dot_position(channel_map.projection(), x_scale, y_scale, xf, xs, yf, ys)
            else {
                continue;
            };

            // Lerp from prev to current
            let (px, py) = if let Some(&(prev_x, prev_y)) = prev_positions.get(i) {
                let x = prev_x + (target_px - prev_x) * t;
                let y = prev_y + (target_py - prev_y) * t;
                (x, y)
            } else {
                (target_px, target_py)
            };

            let colour = resolve_colour(scales, channel_map, batch, i);
            let colour = apply_highlight(colour, i, highlight);
            let circle = Circle::new((px, py), DOT_RADIUS);
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &circle);
        }
    }
}

// ---------------------------------------------------------------------------
// BarRenderer
// ---------------------------------------------------------------------------

/// Which axis a bar mark is oriented along.
#[derive(Clone, Copy)]
pub enum BarAxis {
    /// `barY`: categorical band on x, value on y, baselined at `y = 0`.
    Y,
    /// `barX`: categorical band on y, value on x, baselined at `x = 0`.
    X,
}

/// Renders bar marks as rectangles spanning a categorical band on one axis and
/// running from a zero baseline to the value on the other.
///
/// The `axis` discriminator is load-bearing in three places, all reading it
/// through [`MarkRenderer::zero_baseline_channel`] or the match below, and all
/// three were wrong for `barX` while this struct was a bare unit: the bars
/// (band read off the wrong scale, so `band_width()` returned `None` and this
/// returned before a single fill), the value axis (never extended to zero), and
/// the baseline inset (never exempted, so the value axis got a 5 px gap at the
/// end the bars are supposed to sit flush against).
///
/// # The part-of-whole reading, and why the axis holds still
///
/// A band-aggregating bar under a live `highlight` gets its per-group count of
/// selected rows projected beside its own aggregate — one grouped query, the
/// predicate inside a conditional `SUM` rather than in a `WHERE`. The bar then
/// draws twice: **the whole**, deemphasised, standing for the unfiltered total,
/// and **the selected part** overdrawn on it at full ink from the same
/// baseline.
///
/// The consequence worth naming is about the CATEGORY AXIS, not about the ink.
/// Because the selection never reaches the `GROUP BY`, the rows, the grouping,
/// the ranking and any `limit:` above them are all computed from the unfiltered
/// table — so the bands, their order and their pixel positions do not depend on
/// what is selected. A category cannot drop off the axis by being selected
/// against, and a bar cannot change length under a gesture that did not change
/// the data. That property is a consequence of where the predicate sits; it is
/// not enforced anywhere, and moving the predicate into a filter would lose it
/// silently while every test that only reads ink stayed green.
///
/// [`RectRenderer`] draws the same device over binned continuous groups; the
/// two share `remainder_ink`, `selected_fraction_of` and `selected_tip` so the
/// categorical and continuous forms cannot drift apart.
pub struct BarRenderer {
    /// Orientation — `Y` for barY, `X` for barX.
    pub axis: BarAxis,
}

/// Padding between an in-bar label and the bar's tip, in logical pixels.
const BAR_LABEL_PAD: f64 = 6.0;

/// Font size for an in-bar label. One step below the private `TEXT_MARK_SIZE`
/// a `text` mark draws at — named rather than linked, since a doc link does not
/// get to widen an API: the label annotates a mark rather than being one, and
/// it has to fit inside a band.
///
/// `pub` because a test that reads a label back off a raster has to draw the
/// same string at the same size to compare against, and a second copy of this
/// number in the test would be free to drift from this one — which is the
/// drift that would make such a test go red for a reason that is not a defect.
pub const BAR_LABEL_SIZE: f32 = 10.0;

/// Format one number for an in-bar label.
///
/// Integral values print without a decimal point — a count of rows is the
/// common case and `1234.0` reads as a measurement rather than a tally.
fn label_number(v: f64) -> String {
    if (v.fract()).abs() < f64::EPSILON {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// The text of one bar's label: the whole at rest, and `part / whole` once the
/// batch carries a selected count for this group.
///
/// `share` is the denominator [`LabelForm::Percent`] takes its percentages
/// against — the sum of the values this mark drew. `None` (or a non-positive
/// sum) drops back to the count form rather than dividing by nothing, so a
/// percentage is never printed against a denominator that does not exist.
fn bar_label(
    form: LabelForm,
    value: f64,
    selected: Option<f64>,
    share: Option<f64>,
) -> Option<String> {
    let (whole, part) = match form {
        LabelForm::Count => (label_number(value), selected.map(label_number)),
        LabelForm::Percent => {
            let total = share.filter(|t| *t > 0.0)?;
            let pct = |v: f64| format!("{:.0}%", 100.0 * v / total);
            (pct(value), selected.map(pct))
        }
    };
    Some(match part {
        Some(part) => format!("{part} / {whole}"),
        None => whole,
    })
}

/// Where an in-bar label goes, and in what ink.
///
/// Inside the bar against its tip when the bar is long enough to hold the text
/// and the padding, knocked out in the design system's ink-on-a-solid; just
/// past the tip otherwise, in the bar's own colour so it reads against the
/// plot surface.
///
/// The fallback is what makes the label honest on a ranked chart: the whole
/// point of ranking is that the last bars are short, and a label that silently
/// vanished on them would leave exactly the rows a reader most needs a number
/// for unlabelled.
struct LabelPlacement {
    /// The anchor position along the VALUE axis.
    at: f64,
    anchor: TextAnchor,
    colour: Color,
}

/// Ink for a label knocked out of a filled shape.
///
/// `meridian_design`'s `text.on_solid`, which that crate documents as the same
/// paint in both modes, so a renderer with no mode to consult may read it
/// through either. Not a colour invented here.
fn knockout_ink() -> Color {
    let c = meridian_design::semantic(false).text.on_solid;
    Color::new([c.r, c.g, c.b, c.a])
}

/// Place a label on a bar running from `base` to `tip` in pixels.
///
/// `needed` is the extent the text occupies **along the bar's own axis** — its
/// width for a horizontal bar, its cap height for a vertical one. The caller
/// knows which because the caller knows the orientation; passing the wrong one
/// puts every short bar's label on the wrong side of its tip.
fn place_bar_label(needed: f64, base: f64, tip: f64, ink: Color) -> LabelPlacement {
    let width = needed;
    let span = tip - base;
    let direction = if span < 0.0 { -1.0 } else { 1.0 };
    if span.abs() >= width + 2.0 * BAR_LABEL_PAD {
        LabelPlacement {
            at: tip - direction * BAR_LABEL_PAD,
            anchor: if direction > 0.0 {
                TextAnchor::End
            } else {
                TextAnchor::Start
            },
            colour: knockout_ink(),
        }
    } else {
        LabelPlacement {
            at: tip + direction * BAR_LABEL_PAD,
            anchor: if direction > 0.0 {
                TextAnchor::Start
            } else {
                TextAnchor::End
            },
            colour: ink,
        }
    }
}

/// Vertical offset from a band's centre to the text baseline of a label
/// centred in it, for a font of `size`.
///
/// Half the cap height, near enough: [`draw_text`] positions by BASELINE, so
/// centring means dropping the baseline by half the height of the digits.
fn label_baseline_offset(size: f32) -> f64 {
    f64::from(size) * 0.35
}

impl MarkRenderer for BarRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    ) {
        let (band_channel, value_channel) = match self.axis {
            BarAxis::Y => (Channel::X, Channel::Y),
            BarAxis::X => (Channel::Y, Channel::X),
        };

        let band_col = match channel_map.get(band_channel) {
            Some(c) => c,
            None => return,
        };
        let value_col = match channel_map.get(value_channel) {
            Some(c) => c,
            None => return,
        };
        let band_scale = match scales.get(band_channel) {
            Some(s) => s,
            None => return,
        };
        let value_scale = match scales.get(value_channel) {
            Some(s) => s,
            None => return,
        };

        let band_width = match band_scale.band_width() {
            Some(bw) => bw,
            None => return,
        };

        let band_str = match column_as_string(batch, band_col) {
            Some(v) => v,
            None => return,
        };
        let value_f64 = match column_as_f64(batch, value_col) {
            Some(v) => v,
            None => return,
        };

        // Baseline: 0 mapped through the value scale.
        let baseline = value_scale.map_f64(0.0);

        // The per-group counts of selected rows, when a live highlight put them
        // in the batch. Their presence is what turns each bar into a
        // denominator with a part drawn inside it.
        let counts = selected_counts(batch);
        // The denominator a percentage label is a percentage OF: the values
        // this mark drew, summed. Computed once, and only when a label asked
        // for it.
        let label_form = channel_map.label();
        let drawn_total = (label_form == Some(LabelForm::Percent))
            .then(|| value_f64.iter().filter_map(|v| *v).sum::<f64>());

        let n = batch.num_rows();
        for i in 0..n {
            let cat = match band_str[i].as_deref() {
                Some(c) => c,
                None => continue,
            };
            let value = match value_f64[i] {
                Some(v) => v,
                None => continue,
            };

            let centre = match band_scale.map_category(cat) {
                Some(p) => p,
                None => continue,
            };
            let tip = value_scale.map_f64(value);

            // The BAND span is NOT normalised, and for `BarAxis::X` it is
            // genuinely inverted: a y Band's pixel range runs downward
            // (`layout.rs` `y_range()` is `(bottom, top)`), so `band_width()` is
            // negative and `band_hi < band_lo`. That is fine here —
            // `Fill::NonZero` does not care about winding, and `CellRenderer`
            // already emits the same shape — but it means `Rect::height()` and
            // `Rect::contains()` would be wrong on this rect, so do not hand it
            // to a consumer that reads either without normalising first.
            let band_lo = centre - band_width / 2.0;
            let band_hi = band_lo + band_width;
            // The VALUE span, by contrast, IS ordered low-to-high, so a negative
            // bar draws from its tip back to the baseline rather than inside out.
            let (val_lo, val_hi) = if tip < baseline {
                (tip, baseline)
            } else {
                (baseline, tip)
            };

            let ink = resolve_colour(scales, channel_map, batch, i);
            // The whole bar. Deemphasised when this mark carries per-group
            // selected counts — it is about to become the denominator behind a
            // part — and otherwise exactly as before. Identical to
            // `RectRenderer`'s choice, through the same two helpers.
            let colour = if counts.is_some() {
                remainder_ink(ink, i, highlight)
            } else {
                apply_highlight(ink, i, highlight)
            };
            let rect = match self.axis {
                BarAxis::Y => Rect::new(band_lo, val_lo, band_hi, val_hi),
                BarAxis::X => Rect::new(val_lo, band_lo, val_hi, band_hi),
            };
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &rect);

            // The selected part, overdrawn on the whole from the same baseline.
            // The bar itself did not move, so the part reads as a fraction of a
            // total still on the page.
            let selected = counts.as_ref().and_then(|c| *c.get(i)?);
            if let Some(fraction) = selected_fraction_of(counts.as_ref(), i, value) {
                let edge = selected_tip(baseline, tip, fraction);
                let (lo, hi) = (baseline.min(edge), baseline.max(edge));
                let part = match self.axis {
                    BarAxis::Y => Rect::new(band_lo, lo, band_hi, hi),
                    BarAxis::X => Rect::new(lo, band_lo, hi, band_hi),
                };
                scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, &part);
            }

            // The number, printed on the bar it belongs to.
            if let Some(text) =
                label_form.and_then(|form| bar_label(form, value, selected, drawn_total))
            {
                let centre_offset = label_baseline_offset(BAR_LABEL_SIZE);
                let needed = match self.axis {
                    // A vertical bar's label runs ACROSS the bar, so what has
                    // to fit along the bar is the text's height, not its width.
                    BarAxis::Y => f64::from(BAR_LABEL_SIZE),
                    BarAxis::X => crate::text::measure_width(&text, BAR_LABEL_SIZE),
                };
                let placed = place_bar_label(needed, baseline, tip, ink);
                let (x, y) = match self.axis {
                    BarAxis::Y => (centre, placed.at + centre_offset),
                    BarAxis::X => (placed.at, centre + centre_offset),
                };
                draw_text(
                    scene,
                    &text,
                    x,
                    y,
                    BAR_LABEL_SIZE,
                    placed.colour,
                    match self.axis {
                        // A vertical bar's label is centred across the band and
                        // sits at the tip; the anchor that runs along the value
                        // axis has no horizontal meaning there.
                        BarAxis::Y => TextAnchor::Middle,
                        BarAxis::X => placed.anchor,
                    },
                );
            }
        }
    }

    fn zero_baseline_channel(&self) -> Option<Channel> {
        // Bars baseline at zero on the VALUE axis, so that axis's domain has to
        // include 0. Answering Y unconditionally is what left barX's value axis
        // starting at the data minimum and its baseline end inset off the frame.
        match self.axis {
            BarAxis::Y => Some(Channel::Y),
            BarAxis::X => Some(Channel::X),
        }
    }
}

// ---------------------------------------------------------------------------
// LineRenderer
// ---------------------------------------------------------------------------

/// Renders line marks as a connected path in x-order.
pub struct LineRenderer;

impl MarkRenderer for LineRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let x_f64 = column_as_f64(batch, x_col);
        let y_f64 = column_as_f64(batch, y_col);

        if x_f64.is_none() || y_f64.is_none() {
            return;
        }
        let x_vals = x_f64.unwrap();
        let y_vals = y_f64.unwrap();

        // Collect valid (x_data, y_data) pairs, then sort by x.
        let mut points: Vec<(f64, f64)> = Vec::new();
        for i in 0..batch.num_rows() {
            if let (Some(xv), Some(yv)) = (x_vals[i], y_vals[i]) {
                let px = x_scale.map_f64(xv);
                let py = y_scale.map_f64(yv);
                points.push((px, py));
            }
        }

        // Sort by pixel x (preserves data x-order since the scale is monotonic).
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        if points.len() < 2 {
            return;
        }

        // Draw connected line segments.
        let colour = constant_ink(channel_map, scales.ink());
        let stroke = kurbo::Stroke::new(LINE_STROKE_WIDTH);
        for window in points.windows(2) {
            let line = Line::new(
                kurbo::Point::new(window[0].0, window[0].1),
                kurbo::Point::new(window[1].0, window[1].1),
            );
            scene.stroke(&stroke, Affine::IDENTITY, colour, None, &line);
        }
    }
}

// ---------------------------------------------------------------------------
// AreaRenderer (areaY / areaX)
// ---------------------------------------------------------------------------

/// Fill alpha for area marks, so an overlaid line or dots stay legible.
const AREA_FILL_ALPHA: f32 = 0.75;

/// Which axis an area mark is oriented along.
#[derive(Clone, Copy)]
pub enum AreaAxis {
    /// `areaY`: fill vertically between the `y = 0` baseline and the value line
    /// `y(x)`; points ordered along x.
    Y,
    /// `areaX`: fill horizontally between the `x = 0` baseline and the value
    /// line `x(y)`; points ordered along y.
    X,
}

/// Renders an area mark: the band between a zero baseline and the value line,
/// filled. Points are taken in order along the position axis (like
/// [`LineRenderer`]); the fill is the resolved colour softened by the private
/// `AREA_FILL_ALPHA`.
pub struct AreaRenderer {
    /// Orientation — `Y` for areaY, `X` for areaX.
    pub axis: AreaAxis,
}

impl MarkRenderer for AreaRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let (x_vals, y_vals) = match (column_as_f64(batch, x_col), column_as_f64(batch, y_col)) {
            (Some(x), Some(y)) => (x, y),
            _ => return,
        };

        // Valid (pixel x, pixel y) pairs.
        let mut points: Vec<(f64, f64)> = Vec::new();
        for i in 0..batch.num_rows() {
            if let (Some(xv), Some(yv)) = (x_vals[i], y_vals[i]) {
                points.push((x_scale.map_f64(xv), y_scale.map_f64(yv)));
            }
        }
        // Order along the position axis (the scale is monotonic, so pixel order
        // matches data order): x for areaY, y for areaX.
        match self.axis {
            AreaAxis::Y => {
                points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
            }
            AreaAxis::X => {
                points.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            }
        }
        if points.len() < 2 {
            return;
        }

        // Outline: start on the baseline at the first point, trace the value
        // line, drop back to the baseline at the last point, and close.
        let mut path = BezPath::new();
        match self.axis {
            AreaAxis::Y => {
                let baseline = y_scale.map_f64(0.0);
                path.move_to((points[0].0, baseline));
                for &(px, py) in &points {
                    path.line_to((px, py));
                }
                path.line_to((points[points.len() - 1].0, baseline));
            }
            AreaAxis::X => {
                let baseline = x_scale.map_f64(0.0);
                path.move_to((baseline, points[0].1));
                for &(px, py) in &points {
                    path.line_to((px, py));
                }
                path.line_to((baseline, points[points.len() - 1].1));
            }
        }
        path.close_path();

        let [r, g, b, a] = resolve_colour(scales, channel_map, batch, 0).components;
        let colour = Color::new([r, g, b, a * AREA_FILL_ALPHA]);
        scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &path);
    }

    fn zero_baseline_channel(&self) -> Option<Channel> {
        // The filled band reaches the zero baseline on the value axis, so that
        // axis's domain must include 0.
        match self.axis {
            AreaAxis::Y => Some(Channel::Y),
            AreaAxis::X => Some(Channel::X),
        }
    }
}

// ---------------------------------------------------------------------------
// RectRenderer (rect / rectX / rectY)
// ---------------------------------------------------------------------------

/// Which ranged form a rect mark takes.
#[derive(Clone, Copy)]
pub enum RectKind {
    /// `rect`: an explicit x-extent (`x1`..`x2`) × y-extent (`y1`..`y2`).
    Xy,
    /// `rectX`: the x-axis is a zero-baselined value (`x`), the y-axis a ranged
    /// interval (`y1`..`y2`) — a horizontal numeric-edged bar / histogram.
    X,
    /// `rectY`: the y-axis is a zero-baselined value (`y`), the x-axis a ranged
    /// interval (`x1`..`x2`) — a vertical numeric-edged bar / histogram.
    Y,
}

/// Renders a rectangle per row spanning an x-extent × y-extent. Unlike
/// [`BarRenderer`] (categorical band axis + value axis), rect works in a purely
/// quantitative frame: the extents come from `x1`/`x2`/`y1`/`y2` columns, or —
/// for the `rectX`/`rectY` value forms — from a zero baseline to the `x`/`y`
/// value. This is the substrate for binned 2-D charts and histograms with
/// numeric bin edges.
pub struct RectRenderer {
    /// The ranged form (`rect` / `rectX` / `rectY`).
    pub kind: RectKind,
}

/// Min/max over one or more `Option<f64>` column vectors, ignoring nulls.
fn columns_extent(cols: &[&[Option<f64>]]) -> Option<(f64, f64)> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for col in cols {
        for v in col.iter().flatten() {
            min = min.min(*v);
            max = max.max(*v);
        }
    }
    (min.is_finite() && max.is_finite()).then_some((min, max))
}

impl RectRenderer {
    /// Per-row data-space edges `(a, b)` for one axis. For a ranged axis these
    /// are the two interval columns; for a value axis they are the zero baseline
    /// and the value column. `None` when a required channel/column is absent.
    // The return type IS the documentation here: two per-row columns of
    // optional f64, one per edge. A `type` alias would name it something like
    // `AxisEdges` and force the reader to jump to find out it means exactly
    // what it already says. Single call site, private to the impl.
    #[allow(clippy::type_complexity)]
    fn axis_edges(
        &self,
        ranged: bool,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        interval: (Channel, Channel),
        value: Channel,
    ) -> Option<(Vec<Option<f64>>, Vec<Option<f64>>)> {
        if ranged {
            let a = column_as_f64(batch, channel_map.get(interval.0)?)?;
            let b = column_as_f64(batch, channel_map.get(interval.1)?)?;
            Some((a, b))
        } else {
            let v = column_as_f64(batch, channel_map.get(value)?)?;
            let baseline = vec![Some(0.0); v.len()];
            Some((baseline, v))
        }
    }
}

impl MarkRenderer for RectRenderer {
    // clippy::float_equality_without_abs fires on the `(right - left) <
    // f64::EPSILON` degeneracy guards below. Taking its advice would change what
    // the guard rejects: `(right - left).abs() < EPSILON` skips only a rect that
    // is degenerate, and KEEPS one whose edges are inverted (right < left), which
    // is exactly the malformed path the guard exists to drop before it reaches
    // the rasteriser. The unsigned comparison is deliberate — it means "no
    // positive extent", not "the two edges are equal".
    #[allow(clippy::float_equality_without_abs)]
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    ) {
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        // X ranged for rect/rectY; a value (baseline→x) for rectX.
        let x_ranged = matches!(self.kind, RectKind::Xy | RectKind::Y);
        let y_ranged = matches!(self.kind, RectKind::Xy | RectKind::X);
        let (xa, xb) = match self.axis_edges(
            x_ranged,
            batch,
            channel_map,
            (Channel::X1, Channel::X2),
            Channel::X,
        ) {
            Some(e) => e,
            None => return,
        };
        let (ya, yb) = match self.axis_edges(
            y_ranged,
            batch,
            channel_map,
            (Channel::Y1, Channel::Y2),
            Channel::Y,
        ) {
            Some(e) => e,
            None => return,
        };

        let counts = selected_counts(batch);

        for i in 0..batch.num_rows() {
            let (xav, xbv, yav, ybv) = match (xa[i], xb[i], ya[i], yb[i]) {
                (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
                _ => continue,
            };
            // Map both endpoints through the ONE shared axis scale, then order.
            let (left, right) = {
                let (p, q) = (x_scale.map_f64(xav), x_scale.map_f64(xbv));
                (p.min(q), p.max(q))
            };
            let (top, bottom) = {
                let (p, q) = (y_scale.map_f64(yav), y_scale.map_f64(ybv));
                (p.min(q), p.max(q))
            };
            // Drop non-finite geometry before it reaches Vello. A genuine (non-
            // null) NaN in a bound column survives the None check above, and
            // `f64::min/max` propagate NaN only when BOTH endpoints of an axis are
            // NaN (a single NaN edge collapses to the finite one → caught as
            // zero-area below). A zero-span synthesized scale can likewise map to
            // ±inf. Reject both here rather than rasterise malformed paths.
            if !left.is_finite() || !right.is_finite() || !top.is_finite() || !bottom.is_finite() {
                continue;
            }
            // Skip degenerate (zero-area) rects — an empty bin or collapsed edge.
            if (right - left) < f64::EPSILON || (bottom - top) < f64::EPSILON {
                continue;
            }

            let ink = resolve_colour(scales, channel_map, batch, i);
            // The whole bar. Deemphasised when this mark carries per-group
            // selected counts — it is about to become the denominator behind a
            // part — and otherwise exactly as before.
            let colour = if counts.is_some() {
                remainder_ink(ink, i, highlight)
            } else {
                apply_highlight(ink, i, highlight)
            };
            let rect = Rect::new(left, top, right, bottom);
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &rect);

            // THE PART-OF-WHOLE READING. When the batch carries a per-group
            // count of selected rows, the whole bar above has just been drawn
            // deemphasised — it is the denominator — and the selected part is
            // overdrawn on it at full ink, growing from the same baseline. So a
            // selection reads as a fraction of a bar that did not move, rather
            // than as a bar that changed height for reasons off the page.
            //
            // Only a value form has a baseline to grow that part from; the
            // fully-ranged `rect` has none, so it keeps the deemphasised whole.
            let part = match self.kind {
                RectKind::Y => selected_fraction_of(counts.as_ref(), i, ybv).map(|f| {
                    let base = y_scale.map_f64(yav);
                    let edge = selected_tip(base, y_scale.map_f64(ybv), f);
                    Rect::new(left, base.min(edge), right, base.max(edge))
                }),
                RectKind::X => selected_fraction_of(counts.as_ref(), i, xbv).map(|f| {
                    let base = x_scale.map_f64(xav);
                    let edge = selected_tip(base, x_scale.map_f64(xbv), f);
                    Rect::new(base.min(edge), top, base.max(edge), bottom)
                }),
                RectKind::Xy => None,
            };
            if let Some(part) = part {
                scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, &part);
            }
        }
    }

    fn zero_baseline_channel(&self) -> Option<Channel> {
        // The value form baselines at zero on its value axis, so that axis's
        // domain must include 0. The fully-ranged `rect` has no baseline.
        match self.kind {
            RectKind::Y => Some(Channel::Y),
            RectKind::X => Some(Channel::X),
            RectKind::Xy => None,
        }
    }

    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        // A ranged axis has x1/x2 (or y1/y2) columns but no bare x/y column, so
        // `infer_scales` never builds Channel::X/Y — which axes and grid key off.
        // Synthesise one shared Channel::X (Y) Linear scale spanning both edges,
        // so both endpoints map through the SAME scale (not their own X1/X2).
        //
        // KNOWN LIMITATION (inherited from `merge_linear_scale`, shared with the
        // regression/density synthesis; see the rect-marks follow-up memo):
        //   * If a sibling mark in the same plot already set a NON-linear
        //     Channel::X/Y (a line over a Timestamp → Time, a bar over a Band),
        //     `merge_linear_scale` leaves it untouched, so this rect's extent
        //     never widens that axis and bins past the sibling's domain clip.
        //   * A standalone time-binned rect synthesises a plain Linear scale over
        //     raw-microsecond edges (`column_as_f64` casts Timestamp → µs), so the
        //     axis shows microsecond integers rather than a Time scale.
        // Both await time/band-aware ranged-axis synthesis; geometry is correct.
        if let (Some(x1c), Some(x2c)) = (channel_map.get(Channel::X1), channel_map.get(Channel::X2))
        {
            if let (Some(x1), Some(x2)) = (column_as_f64(batch, x1c), column_as_f64(batch, x2c)) {
                if let Some((min, max)) = columns_extent(&[&x1, &x2]) {
                    merge_linear_scale(scales, Channel::X, min, max, x_range);
                }
            }
        }
        if let (Some(y1c), Some(y2c)) = (channel_map.get(Channel::Y1), channel_map.get(Channel::Y2))
        {
            if let (Some(y1), Some(y2)) = (column_as_f64(batch, y1c), column_as_f64(batch, y2c)) {
                if let Some((min, max)) = columns_extent(&[&y1, &y2]) {
                    merge_linear_scale(scales, Channel::Y, min, max, y_range);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RuleRenderer (ruleX / ruleY)
// ---------------------------------------------------------------------------

/// Which axis a rule is positioned on.
#[derive(Clone, Copy)]
pub enum RuleAxis {
    /// `ruleX`: vertical lines at each x position, spanning the full y-extent.
    X,
    /// `ruleY`: horizontal lines at each y position, spanning the full x-extent.
    Y,
}

/// Renders rule marks: thin straight lines spanning the plot, positioned by one
/// channel — reference lines, thresholds, baselines. The position channel may be
/// a column (one rule per row) OR a constant literal (one rule, e.g. `y: 0`).
///
/// A rule spans the PERPENDICULAR axis, so that axis's scale must exist. It does
/// whenever a sibling mark (or the rule's own data) gives that axis a scale; a
/// standalone single-channel rule with no perpendicular data does not render.
pub struct RuleRenderer {
    /// Orientation — `X` for ruleX (verticals), `Y` for ruleY (horizontals).
    pub axis: RuleAxis,
}

impl MarkRenderer for RuleRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let (pos_channel, span_channel) = match self.axis {
            RuleAxis::X => (Channel::X, Channel::Y),
            RuleAxis::Y => (Channel::Y, Channel::X),
        };
        let pos_scale = match scales.get(pos_channel) {
            Some(s) => s,
            None => return,
        };
        // The line spans the perpendicular axis's pixel range.
        let span_scale = match scales.get(span_channel) {
            Some(s) => s,
            None => return,
        };
        let (span0, span1) = (span_scale.range_start(), span_scale.range_end());

        // Positions in pixels: a literal constant (one rule) or a column value
        // per row (one rule each).
        let positions: Vec<f64> = if let Some(literal) = channel_map.literal(pos_channel) {
            vec![pos_scale.map_f64(literal)]
        } else if let Some(col) = channel_map.get(pos_channel) {
            match column_as_f64(batch, col) {
                Some(vals) => vals
                    .iter()
                    .filter_map(|v| v.map(|x| pos_scale.map_f64(x)))
                    .collect(),
                None => return,
            }
        } else {
            return;
        };

        let stroke = kurbo::Stroke::new(LINE_STROKE_WIDTH);
        let colour = constant_ink(channel_map, scales.ink());
        for p in positions {
            let line = match self.axis {
                RuleAxis::X => Line::new((p, span0), (p, span1)),
                RuleAxis::Y => Line::new((span0, p), (span1, p)),
            };
            scene.stroke(&stroke, Affine::IDENTITY, colour, None, &line);
        }
    }
}

// ---------------------------------------------------------------------------
// TextRenderer (text)
// ---------------------------------------------------------------------------

/// Font size for text-mark labels, in logical pixels.
const TEXT_MARK_SIZE: f32 = 11.0;

/// Renders a `text` mark: a string label centred at each `(x, y)` data position.
/// The label content comes from the `text` channel (a string column); `x`/`y`
/// position it (numeric or categorical, like the dot mark). A row with no label
/// or an unresolvable position is skipped.
pub struct TextRenderer;

impl MarkRenderer for TextRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let text_col = match channel_map.get(Channel::Text) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let x_f64 = column_as_f64(batch, x_col);
        let x_str = column_as_string(batch, x_col);
        let y_f64 = column_as_f64(batch, y_col);
        let y_str = column_as_string(batch, y_col);
        let labels = match column_as_string(batch, text_col) {
            Some(v) => v,
            None => return,
        };

        for i in 0..batch.num_rows() {
            let label = match labels.get(i).and_then(|o| o.as_deref()) {
                Some(s) => s,
                None => continue,
            };
            let px = match resolve_position(
                x_scale,
                x_f64.as_ref().and_then(|v| v[i]),
                x_str.as_ref().and_then(|v| v[i].as_deref()),
            ) {
                Some(p) => p,
                None => continue,
            };
            let py = match resolve_position(
                y_scale,
                y_f64.as_ref().and_then(|v| v[i]),
                y_str.as_ref().and_then(|v| v[i].as_deref()),
            ) {
                Some(p) => p,
                None => continue,
            };
            let colour =
                apply_highlight(resolve_colour(scales, channel_map, batch, i), i, highlight);
            draw_text(
                scene,
                label,
                px,
                py,
                TEXT_MARK_SIZE,
                colour,
                TextAnchor::Middle,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Density1DRenderer (density / densityX / densityY)
// ---------------------------------------------------------------------------

/// Which axis carries the density curve.
///
/// `DensityX` plots density along x as a function of x; the curve fills
/// downward from the density baseline. `DensityY` plots density along y.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DensityAxis {
    /// Density on x — peak height encoded in y.
    X,
    /// Density on y — peak height encoded in x.
    Y,
}

/// Renders a 1D density curve from a pre-binned (centre, count) batch.
///
/// The lowerer produces a RecordBatch with two columns:
///   - the binned axis column, aliased to the channel column name — Float64 bin
///     centres in data units (only OCCUPIED buckets; a GROUP BY omits empties)
///   - a `count` column — Float64
///
/// At render time the (centre, count) pairs are treated as weighted samples; a
/// Gaussian KDE is evaluated over a fixed grid via `kde_1d_weighted` (robust to
/// the gapped, non-uniform centres a GROUP BY leaves behind), then drawn as a
/// filled path against a synthesised normalised density axis.
pub struct Density1DRenderer {
    pub axis: DensityAxis,
}

/// Read a 1D density mark's occupied `(centre, count)` pairs from `batch`,
/// sorted by centre — the weighted samples both `render` and `augment_scales`
/// build the KDE (and its extent) from. Counts come from the reserved
/// [`DENSITY_COUNT_COL`]. `None` when fewer than two distinct bins survive (too
/// few to form a curve).
fn density_1d_weighted_pairs(batch: &RecordBatch, bin_col: &str) -> Option<Vec<(f64, f64)>> {
    let bin_vals = column_as_f64(batch, bin_col)?;
    let count_vals = column_as_f64(batch, DENSITY_COUNT_COL)?;
    let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(bin_vals.len());
    for (b, c) in bin_vals.into_iter().zip(count_vals) {
        if let (Some(b), Some(c)) = (b, c) {
            pairs.push((b, c.max(0.0)));
        }
    }
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if pairs.len() < 2 {
        None
    } else {
        Some(pairs)
    }
}

/// KDE evaluation extends this many bandwidths past the data extremes — the
/// kernel's ±3σ truncation support — so the density curve tapers to ~0 at the
/// tails rather than dropping in a vertical cliff at the data min/max. `render`
/// pads the grid by this and `augment_scales` widens the bin axis to match.
const DENSITY_TAIL_SIGMAS: f64 = 3.0;

impl MarkRenderer for Density1DRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        // Bin column is the axis specified by the renderer; count is the
        // density-mapped channel.
        let (bin_channel, density_channel) = match self.axis {
            DensityAxis::X => (Channel::X, Channel::Y),
            DensityAxis::Y => (Channel::Y, Channel::X),
        };

        let bin_col = match channel_map.get(bin_channel) {
            Some(c) => c,
            None => return,
        };
        let bin_scale = match scales.get(bin_channel) {
            Some(s) => s,
            None => return,
        };
        let density_scale = match scales.get(density_channel) {
            Some(s) => s,
            None => return,
        };

        // (centre, count) pairs, sorted by centre, as WEIGHTED samples — never
        // un-bin to the full row count (a single bucket's count can be in the
        // millions, and render() runs on every scene build / reactive rebuild).
        let pairs = match density_1d_weighted_pairs(batch, bin_col) {
            Some(p) => p,
            None => return,
        };
        let bandwidth = silverman_1d_weighted(&pairs);
        if bandwidth <= 0.0 {
            return;
        }
        let lo = pairs.first().unwrap().0;
        let hi = pairs.last().unwrap().0;
        if hi <= lo {
            return;
        }

        // Evaluate the KDE on a fixed uniform grid. A GROUP BY density query
        // returns only the OCCUPIED buckets, so the bin centres are gapped /
        // non-uniform whenever the data has gaps; summing the weighted kernel
        // directly (rather than convolving a histogram, which assumes a dense
        // uniform grid) is robust to that — see `kde_1d_weighted`. The grid
        // extends ±3σ past the data (matching the bin axis widened in
        // `augment_scales`) so the curve tapers to ~0 at the tails.
        let pad = DENSITY_TAIL_SIGMAS * bandwidth;
        let g_lo = lo - pad;
        let g_hi = hi + pad;
        const GRID: usize = 192;
        let grid: Vec<f64> = (0..GRID)
            .map(|i| g_lo + (g_hi - g_lo) * (i as f64) / ((GRID - 1) as f64))
            .collect();
        let density = kde_1d_weighted(&pairs, bandwidth, &grid);

        let max_density = density.iter().cloned().fold(0.0_f64, f64::max);
        if max_density <= 0.0 {
            return;
        }

        // Density 0 renders at the density-axis baseline; density max near the
        // far end of the axis range.
        let baseline_pixel = density_scale.range_start();
        let peak_pixel = density_scale.range_end();
        let pixel_height = peak_pixel - baseline_pixel;

        let mut path = BezPath::new();
        for (i, &centre) in grid.iter().enumerate() {
            let bin_pixel = bin_scale.map_f64(centre);
            let normalised = density[i] / max_density;
            let dens_pixel = baseline_pixel + normalised * pixel_height;
            let (px, py) = match self.axis {
                DensityAxis::X => (bin_pixel, dens_pixel),
                DensityAxis::Y => (dens_pixel, bin_pixel),
            };
            if i == 0 {
                path.move_to((px, py));
            } else {
                path.line_to((px, py));
            }
        }
        // Close back to the baseline at the padded ends (density ~0 there).
        let last_bin = bin_scale.map_f64(g_hi);
        let first_bin = bin_scale.map_f64(g_lo);
        match self.axis {
            DensityAxis::X => {
                path.line_to((last_bin, baseline_pixel));
                path.line_to((first_bin, baseline_pixel));
            }
            DensityAxis::Y => {
                path.line_to((baseline_pixel, last_bin));
                path.line_to((baseline_pixel, first_bin));
            }
        }
        path.close_path();

        let colour = constant_ink(channel_map, scales.ink());
        scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &path);
    }

    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        let (bin_channel, bin_range, density_channel, density_range) = match self.axis {
            DensityAxis::X => (Channel::X, x_range, Channel::Y, y_range),
            DensityAxis::Y => (Channel::Y, y_range, Channel::X, x_range),
        };

        // Widen the bin axis to the padded KDE domain (±3σ past the data,
        // matching `render`'s grid) so the tapered tails are on-plot rather than
        // clipped. Unions with the inferred [lo, hi] via merge_linear_scale.
        if let Some(bin_col) = channel_map.get(bin_channel) {
            if let Some(pairs) = density_1d_weighted_pairs(batch, bin_col) {
                let bandwidth = silverman_1d_weighted(&pairs);
                let lo = pairs.first().unwrap().0;
                let hi = pairs.last().unwrap().0;
                if bandwidth > 0.0 && hi > lo {
                    let pad = DENSITY_TAIL_SIGMAS * bandwidth;
                    merge_linear_scale(scales, bin_channel, lo - pad, hi + pad, bin_range);
                }
            }
        }

        // The perpendicular "density" axis has no data column — synthesise a
        // normalised [0, 1] scale over its pixel range unless a sibling mark
        // already provided that axis.
        if scales.get(density_channel).is_none() {
            scales.insert(
                density_channel,
                Scale::Linear {
                    domain_min: 0.0,
                    domain_max: 1.0,
                    range_start: density_range.0,
                    range_end: density_range.1,
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Shared KDE grid (density / heatmap / contour)
// ---------------------------------------------------------------------------

/// A reconstructed, KDE-smoothed 2D grid — the shared substrate of the
/// `density`, `heatmap`, and `contour` renderers (density marks).
///
/// The density lowerer emits one `(x centre, y centre, __bf_count)` row per
/// OCCUPIED bin; [`build_kde_grid`] reconstructs the dense rectangular
/// histogram from those rows, picks bandwidths, and runs [`kde_2d`]. Each
/// consumer then draws the smoothed field its own way: density as
/// alpha-encoded circles, heatmap as ramp-filled cells, contour as iso-lines.
///
/// The lattice is DENSE: the centres run `first..last` at the recovered bin
/// pitch (the GCD of the occupied-centre gaps, via [`bin_step`]), so unoccupied
/// interior bins are materialised with zero mass. kde_2d then smooths over the
/// true geometry — a sparse axis is not collapsed to adjacency.
pub(crate) struct KdeGrid {
    /// Dense x bin centres (grid columns), `first..last` at pitch `dx`.
    pub x_centres: Vec<f64>,
    /// Dense y bin centres (grid rows), `first..last` at pitch `dy`.
    pub y_centres: Vec<f64>,
    /// Column pitch — the recovered bin step, uniform across `x_centres` (> 0).
    pub dx: f64,
    /// Row pitch — the recovered bin step, uniform across `y_centres` (> 0).
    pub dy: f64,
    /// Row-major smoothed density: cell `(row, col)` — row indexing
    /// `y_centres`, col indexing `x_centres` — is `density[row * cols + col]`.
    pub density: Vec<f64>,
    /// Maximum density value over the grid (> 0).
    pub max_density: f64,
}

impl KdeGrid {
    /// Number of grid rows (y bin centres).
    pub fn rows(&self) -> usize {
        self.y_centres.len()
    }

    /// Number of grid columns (x bin centres).
    pub fn cols(&self) -> usize {
        self.x_centres.len()
    }
}

/// Reconstruct the 2D histogram from a density-lowerer batch and smooth it
/// with [`kde_2d`] — extracted verbatim from `Density2DRenderer::render` so
/// heatmap and contour ride the identical grid (behaviour-identity is pinned
/// by the byte-identical density example PNGs).
///
/// `bandwidth`, when present (the mark's `bandwidth:` attribute, in data
/// units), is applied to both axes; otherwise Silverman's rule runs per axis
/// over the reconstructed samples. Returns `None` whenever the inline path
/// would have early-returned: a missing/non-numeric column, fewer than two
/// distinct centres on either axis, a non-positive pitch or bandwidth, or an
/// all-zero smoothed field.
pub(crate) fn build_kde_grid(
    batch: &RecordBatch,
    x_col: &str,
    y_col: &str,
    bandwidth: Option<f64>,
) -> Option<KdeGrid> {
    let x_vals = column_as_f64(batch, x_col)?;
    let y_vals = column_as_f64(batch, y_col)?;
    let count_vals = column_as_f64(batch, DENSITY_COUNT_COL)?;

    // Collect the OCCUPIED bin centres on each axis (sorted) + the (x, y, count)
    // tuples the lowerer emitted (only occupied bins survive its GROUP BY).
    let mut x_occ: Vec<f64> = Vec::new();
    let mut y_occ: Vec<f64> = Vec::new();
    let mut tuples: Vec<(f64, f64, u32)> = Vec::new();
    for i in 0..batch.num_rows() {
        if let (Some(xv), Some(yv), Some(c)) = (x_vals[i], y_vals[i], count_vals[i]) {
            tuples.push((xv, yv, c.max(0.0).round() as u32));
            if !x_occ.iter().any(|v| (*v - xv).abs() < 1e-9) {
                x_occ.push(xv);
            }
            if !y_occ.iter().any(|v| (*v - yv).abs() < 1e-9) {
                y_occ.push(yv);
            }
        }
    }
    x_occ.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    y_occ.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    if x_occ.len() < 2 || y_occ.len() < 2 {
        return None;
    }
    // Recover the TRUE bin pitch (GCD of the occupied-centre gaps, not the first
    // adjacent gap), then build a DENSE first..last lattice at that pitch —
    // unoccupied interior bins are materialised with zero mass. This makes
    // kde_2d smooth over the real geometry (gaps are gaps, not collapsed to
    // adjacency) and gives contour true gap geometry. Before this fix the grid
    // held only the occupied centres, so a sparse axis read as densely packed
    // (the deliberate density-family re-baseline).
    let dx = bin_step(&x_occ)?;
    let dy = bin_step(&y_occ)?;
    if dx <= 0.0 || dy <= 0.0 {
        return None;
    }
    let dense_lattice = |occ: &[f64], step: f64| -> Vec<f64> {
        let (lo, hi) = (occ[0], occ[occ.len() - 1]);
        let n = ((hi - lo) / step).round() as usize + 1;
        (0..n).map(|i| lo + (i as f64) * step).collect()
    };
    let x_centres = dense_lattice(&x_occ, dx);
    let y_centres = dense_lattice(&y_occ, dy);

    let cols = x_centres.len();
    let rows = y_centres.len();

    // Build the flat row-major histogram over the DENSE lattice: each occupied
    // bin maps to its lattice index (round the offset by the pitch); every other
    // dense cell keeps zero.
    let mut bins = vec![0u32; rows * cols];
    for (xv, yv, c) in &tuples {
        let cx = ((xv - x_centres[0]) / dx).round() as usize;
        let cy = ((yv - y_centres[0]) / dy).round() as usize;
        if cx < cols && cy < rows {
            bins[cy * cols + cx] = *c;
        }
    }

    // Bandwidth: the mark's explicit attribute on both axes, else Silverman
    // from the reconstructed (x, y) samples.
    let (h_x, h_y) = match bandwidth {
        Some(h) => (h, h),
        None => {
            let mut xs_samples: Vec<f64> = Vec::new();
            let mut ys_samples: Vec<f64> = Vec::new();
            for r in 0..rows {
                for c in 0..cols {
                    for _ in 0..bins[r * cols + c] {
                        xs_samples.push(x_centres[c]);
                        ys_samples.push(y_centres[r]);
                    }
                }
            }
            silverman_2d_per_axis(&xs_samples, &ys_samples)
        }
    };
    if h_x <= 0.0 || h_y <= 0.0 {
        return None;
    }

    let density = kde_2d(&bins, (rows, cols), (h_x, h_y), (dx, dy));
    let max_density = density.iter().cloned().fold(0.0_f64, f64::max);
    if max_density <= 0.0 {
        return None;
    }

    Some(KdeGrid {
        x_centres,
        y_centres,
        dx,
        dy,
        density,
        max_density,
    })
}

// ---------------------------------------------------------------------------
// Density2DRenderer (density with both x and y bins)
// ---------------------------------------------------------------------------

/// Renders 2D density as a grid of circles whose alpha encodes density value.
///
/// The lowerer emits `(x_bin, y_bin, count)`; this renderer reconstructs the
/// rectangular histogram via the shared crate-private `build_kde_grid` helper
/// and draws a circle per cell with alpha proportional to normalised density.
pub struct Density2DRenderer;

impl MarkRenderer for Density2DRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let x_col = match channel_map.get(Channel::X) {
            Some(c) => c,
            None => return,
        };
        let y_col = match channel_map.get(Channel::Y) {
            Some(c) => c,
            None => return,
        };
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        let grid = match build_kde_grid(batch, x_col, y_col, None) {
            Some(g) => g,
            None => return,
        };

        let (rows, cols) = (grid.rows(), grid.cols());
        let radius = DOT_RADIUS.max(2.0);
        for r in 0..rows {
            for c in 0..cols {
                let normalised = grid.density[r * cols + c] / grid.max_density;
                if normalised <= 0.01 {
                    continue;
                }
                let px = x_scale.map_f64(grid.x_centres[c]);
                let py = y_scale.map_f64(grid.y_centres[r]);
                let [cr, cg, cb, _ca] = scales.ink().mark_default.components;
                let colour = Color::new([cr, cg, cb, normalised as f32]);
                let circle = Circle::new((px, py), radius);
                scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &circle);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RasterRenderer (raster — binned 2D count heatmap)
// ---------------------------------------------------------------------------

/// Minimum ramp position for an OCCUPIED bin (count ≥ 1), so the sparsest cells
/// stay visibly tinted rather than washing out to the low end of a light-anchored
/// scheme (blues starts near-white). Replaces the former `RASTER_MIN_ALPHA` — the
/// same visibility guarantee, expressed as a floor on ramp position `t` (the
/// domain is zero-anchored, so an occupied cell already sits above `t = 0`).
const RASTER_MIN_T: f64 = 0.15;

/// Sorted unique values from a nullable column, de-duplicated within a tolerance.
fn sorted_unique(vals: &[Option<f64>]) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for v in vals.iter().flatten() {
        if !out.iter().any(|u| (*u - *v).abs() < 1e-9) {
            out.push(*v);
        }
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// The bin pitch (width of one bin, in data units). Equiwidth bin centres sit at
/// `lo + (bucket + 0.5)·w`, so every gap between two occupied centres is an
/// *integer multiple* of the width `w` — the pitch is therefore the GCD of the
/// gaps (their largest common divisor), which we recover as `min_gap / k` for the
/// smallest `k` whose quotient divides every gap.
///
/// This stays correct when NO two occupied bins are adjacent: with discrete data
/// and `bins` over-specified relative to the range, consecutive values can land in
/// buckets two-or-more apart, so the smallest gap alone would over-estimate the
/// width (a 2-apart gap reads as `2w`) and cells would over-cover the empty bins
/// between them. `None` for fewer than two distinct centres.
fn bin_step(centres: &[f64]) -> Option<f64> {
    let gaps: Vec<f64> = centres
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 1e-9)
        .collect();
    let min_gap = gaps.iter().copied().fold(f64::INFINITY, f64::min);
    if !min_gap.is_finite() {
        return None;
    }
    // The width divides min_gap, so try min_gap/1, min_gap/2, … and take the first
    // (largest width) that divides every gap — that is their GCD. The common dense
    // case has an adjacent occupied pair, so k=1 (width == min_gap) hits immediately.
    // Cap k so extreme sparsity (>12-apart bins) degrades to min_gap rather than
    // float noise picking a spuriously tiny width.
    let divides = |w: f64, gap: f64| {
        let ratio = gap / w;
        (ratio - ratio.round()).abs() < 1e-6
    };
    for k in 1..=12 {
        let w = min_gap / f64::from(k);
        if gaps.iter().all(|g| divides(w, *g)) {
            return Some(w);
        }
    }
    Some(min_gap)
}

/// Renders a 2D count heatmap. The density lowerer emits `(x_centre, y_centre,
/// count)` for each OCCUPIED bin; this draws one filled cell per bin coloured by
/// its raw count through a sequential colour ramp — a true count → gradient
/// heatmap, no KDE smoothing.
///
/// The ramp is the Fill [`Scale::Sequential`] this mark's [`Self::augment_scales`]
/// builds from the count domain; `scheme` selects its colours. If the Fill scale
/// is somehow absent, `render` falls back to the legacy alpha-on-default-blue path.
#[derive(Debug, Clone, Copy, Default)]
pub struct RasterRenderer {
    /// The continuous colour scheme (default viridis).
    pub scheme: SequentialScheme,
}

impl MarkRenderer for RasterRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let (Some(x_col), Some(y_col)) = (channel_map.get(Channel::X), channel_map.get(Channel::Y))
        else {
            return;
        };
        let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            return;
        };
        let (Some(x_vals), Some(y_vals), Some(count_vals)) = (
            column_as_f64(batch, x_col),
            column_as_f64(batch, y_col),
            column_as_f64(batch, DENSITY_COUNT_COL),
        ) else {
            return;
        };

        // Bin pitch from the unique centres on each axis.
        let (Some(dx), Some(dy)) = (
            bin_step(&sorted_unique(&x_vals)),
            bin_step(&sorted_unique(&y_vals)),
        ) else {
            return;
        };

        let max_count = count_vals.iter().flatten().cloned().fold(0.0_f64, f64::max);
        if max_count <= 0.0 {
            return;
        }

        // Prefer the Fill Sequential ramp (built by augment_scales). Its domain is
        // zero-anchored [0, max_count]; each occupied cell samples the ramp at its
        // count, floored at RASTER_MIN_T so the sparsest cells stay visible. A
        // missing / non-Sequential Fill scale falls back to alpha-on-default-blue.
        let fill_ramp = match scales.get(Channel::Fill) {
            Some(scale @ Scale::Sequential { .. }) => Some(scale),
            _ => None,
        };
        let [cr, cg, cb, _] = scales.ink().mark_default.components;
        for i in 0..batch.num_rows() {
            let (Some(cx), Some(cy), Some(count)) = (x_vals[i], y_vals[i], count_vals[i]) else {
                continue;
            };
            if count <= 0.0 {
                continue;
            }
            // Cell spans its centre ± half a bin, mapped to pixels.
            let xa = x_scale.map_f64(cx - dx / 2.0);
            let xb = x_scale.map_f64(cx + dx / 2.0);
            let ya = y_scale.map_f64(cy - dy / 2.0);
            let yb = y_scale.map_f64(cy + dy / 2.0);
            let (left, right) = (xa.min(xb), xa.max(xb));
            let (top, bottom) = (ya.min(yb), ya.max(yb));
            if !(left.is_finite() && right.is_finite() && top.is_finite() && bottom.is_finite()) {
                continue;
            }
            let colour = match fill_ramp {
                Some(ramp) => {
                    // Sample the ramp in ITS OWN domain — which may be a union of
                    // several co-rendered rasters' counts, not this batch's local
                    // max — flooring the position at RASTER_MIN_T so the sparsest
                    // occupied cell stays visible; the ramp colour carries full
                    // alpha. Rounding through the local max_count would break both
                    // the colour and the floor whenever the two domains differ.
                    let dmax = ramp.domain_max().filter(|d| *d > 0.0).unwrap_or(max_count);
                    let pos = (count / dmax).clamp(0.0, 1.0).max(RASTER_MIN_T);
                    Color::new(ramp.map_continuous(pos * dmax))
                }
                None => {
                    // Legacy fallback: single-hue with count-proportional alpha,
                    // floored at RASTER_MIN_T so every occupied cell is visible.
                    let t = (count / max_count).clamp(0.0, 1.0).max(RASTER_MIN_T) as f32;
                    Color::new([cr, cg, cb, t])
                }
            };
            let cell = kurbo::Rect::new(left, top, right, bottom);
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &cell);
        }
    }

    /// Widen the linear x/y domains by half a bin so the outermost cells (which
    /// extend ±half a bin past their centres) fit inside the plot area rather
    /// than overflowing into the axis margins, and build the count → colour ramp
    /// under [`Channel::Fill`] so the legend plumbing picks it up.
    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        for (channel, range) in [(Channel::X, x_range), (Channel::Y, y_range)] {
            let Some(col) = channel_map.get(channel) else {
                continue;
            };
            let Some(vals) = column_as_f64(batch, col) else {
                continue;
            };
            let centres = sorted_unique(&vals);
            if let (Some(step), Some(&lo), Some(&hi)) =
                (bin_step(&centres), centres.first(), centres.last())
            {
                merge_linear_scale(scales, channel, lo - step / 2.0, hi + step / 2.0, range);
            }
        }

        // Count → colour ramp under Fill, zero-anchored at [0, max_count] so an
        // occupied cell (count ≥ 1) maps above the ramp's low end. Generic column
        // inference can't build this — the count lives in the reserved
        // `__bf_count` column, not the mark's channel map.
        //
        // Merge rather than clobber: build_multi_mark_scene runs every mark's
        // augment_scales against one shared ScaleSet, so blind-inserting would
        // (a) destroy a sibling mark's categorical Colour Fill (a layered
        // `dot fill:<category>` + raster would lose its swatches), and (b) let a
        // second raster overwrite the first's domain. A co-rendered raster unions
        // its zero-anchored domain (keeping the first's stops, per union_scales);
        // a non-Sequential Fill is left untouched — render falls back to the
        // legacy path for it, so the layered plot degrades gracefully.
        if let Some(counts) = column_as_f64(batch, DENSITY_COUNT_COL) {
            let max_count = counts.iter().flatten().cloned().fold(0.0_f64, f64::max);
            if max_count > 0.0 {
                let merged = match scales.get(Channel::Fill) {
                    Some(Scale::Sequential {
                        domain_min,
                        domain_max,
                        stops,
                    }) => Some(Scale::Sequential {
                        domain_min: domain_min.min(0.0),
                        domain_max: domain_max.max(max_count),
                        stops: stops.clone(),
                    }),
                    Some(_) => None, // a sibling's categorical Fill scale wins
                    None => Some(Scale::Sequential {
                        domain_min: 0.0,
                        domain_max: max_count,
                        stops: self.scheme.stops(),
                    }),
                };
                if let Some(scale) = merged {
                    scales.insert(Channel::Fill, scale);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HeatmapRenderer (heatmap — KDE-smoothed density ramp)
// ---------------------------------------------------------------------------

/// Renders the KDE-smoothed 2D density field as ramp-filled grid cells — the
/// smoothed sibling of raster (density marks). Raster ramps the raw
/// `__bf_count` of each OCCUPIED bin; heatmap ramps the `build_kde_grid`
/// field over EVERY grid cell, so the plot reads as a continuous density
/// surface rather than discrete count tiles. Both register against the same
/// 2D density lowerer.
///
/// The ramp is the Fill [`Scale::Sequential`] this mark's
/// [`Self::augment_scales`] builds from the density domain (zero-anchored
/// `[0, max_density]`); `scheme` selects its colours and `bandwidth` (the
/// mark's attribute, in data units) overrides Silverman's rule on both axes.
/// If the Fill scale is somehow absent, `render` falls back to
/// alpha-on-default-blue like the density mark.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeatmapRenderer {
    /// The continuous colour scheme (default viridis).
    pub scheme: SequentialScheme,
    /// Explicit KDE bandwidth in data units (both axes); Silverman per axis
    /// when absent.
    pub bandwidth: Option<f64>,
}

impl MarkRenderer for HeatmapRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let (Some(x_col), Some(y_col)) = (channel_map.get(Channel::X), channel_map.get(Channel::Y))
        else {
            return;
        };
        let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            return;
        };
        let Some(grid) = build_kde_grid(batch, x_col, y_col, self.bandwidth) else {
            return;
        };

        // Draw pitch per axis. The KDE lattice is DENSE (a
        // `first..last` run at the recovered `bin_step` pitch, interior gap bins
        // materialised), so `grid.dx`/`grid.dy` already ARE the true uniform
        // pitch. The `bin_step` recompute below is therefore a no-op on that
        // uniform lattice — kept only as a defensive belt (its GCD equals the
        // adjacent gap on any uniform run) so a future caller passing a
        // non-uniform grid still sizes the drawn cells truthfully.
        let draw_dx = bin_step(&grid.x_centres).unwrap_or(grid.dx);
        let draw_dy = bin_step(&grid.y_centres).unwrap_or(grid.dy);

        // Prefer the Fill Sequential ramp (built by augment_scales); a missing /
        // non-Sequential Fill scale falls back to alpha-on-default-blue.
        let fill_ramp = match scales.get(Channel::Fill) {
            Some(scale @ Scale::Sequential { .. }) => Some(scale),
            _ => None,
        };
        let [cr, cg, cb, _] = scales.ink().mark_default.components;
        let (rows, cols) = (grid.rows(), grid.cols());
        for r in 0..rows {
            for c in 0..cols {
                let value = grid.density[r * cols + c];
                // Cell spans its centre ± half a bin, mapped to pixels.
                let cx = grid.x_centres[c];
                let cy = grid.y_centres[r];
                let xa = x_scale.map_f64(cx - draw_dx / 2.0);
                let xb = x_scale.map_f64(cx + draw_dx / 2.0);
                let ya = y_scale.map_f64(cy - draw_dy / 2.0);
                let yb = y_scale.map_f64(cy + draw_dy / 2.0);
                let (left, right) = (xa.min(xb), xa.max(xb));
                let (top, bottom) = (ya.min(yb), ya.max(yb));
                if !(left.is_finite() && right.is_finite() && top.is_finite() && bottom.is_finite())
                {
                    continue;
                }
                let colour = match fill_ramp {
                    // Sample the ramp in ITS OWN domain (which may be a union of
                    // co-rendered marks' domains). No occupancy floor here — the
                    // smoothed field is continuous, so the ramp's low end IS the
                    // zero-density background.
                    Some(ramp) => Color::new(ramp.map_continuous(value)),
                    None => {
                        let t = (value / grid.max_density).clamp(0.0, 1.0) as f32;
                        Color::new([cr, cg, cb, t])
                    }
                };
                let cell = kurbo::Rect::new(left, top, right, bottom);
                scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &cell);
            }
        }
    }

    /// Widen the linear x/y domains by half a bin so the outermost cells fit
    /// inside the plot area, and build the density → colour ramp under
    /// [`Channel::Fill`] (zero-anchored `[0, max_density]`) so the legend
    /// plumbing picks it up. Merge-not-clobber mirrors
    /// [`RasterRenderer::augment_scales`]: a co-rendered Sequential unions its
    /// domain (keeping the first's stops); a sibling's categorical Colour Fill
    /// survives untouched.
    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        let (Some(x_col), Some(y_col)) = (channel_map.get(Channel::X), channel_map.get(Channel::Y))
        else {
            return;
        };
        let Some(grid) = build_kde_grid(batch, x_col, y_col, self.bandwidth) else {
            return;
        };

        // Same per-axis DRAW pitch as `render`. On the dense lattice this
        // equals `grid.dx`/`grid.dy` (a no-op recompute); kept in lockstep with
        // `render` so the half-bin widening matches the cells actually drawn.
        let draw_dx = bin_step(&grid.x_centres).unwrap_or(grid.dx);
        let draw_dy = bin_step(&grid.y_centres).unwrap_or(grid.dy);

        if let (Some(&x_lo), Some(&x_hi)) = (grid.x_centres.first(), grid.x_centres.last()) {
            merge_linear_scale(
                scales,
                Channel::X,
                x_lo - draw_dx / 2.0,
                x_hi + draw_dx / 2.0,
                x_range,
            );
        }
        if let (Some(&y_lo), Some(&y_hi)) = (grid.y_centres.first(), grid.y_centres.last()) {
            merge_linear_scale(
                scales,
                Channel::Y,
                y_lo - draw_dy / 2.0,
                y_hi + draw_dy / 2.0,
                y_range,
            );
        }

        let merged = match scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            }) => Some(Scale::Sequential {
                domain_min: domain_min.min(0.0),
                domain_max: domain_max.max(grid.max_density),
                stops: stops.clone(),
            }),
            Some(_) => None, // a sibling's categorical Fill scale wins
            None => Some(Scale::Sequential {
                domain_min: 0.0,
                domain_max: grid.max_density,
                stops: self.scheme.stops(),
            }),
        };
        if let Some(scale) = merged {
            scales.insert(Channel::Fill, scale);
        }
    }
}

// ---------------------------------------------------------------------------
// CellRenderer (cell — categorical × categorical value grid)
// ---------------------------------------------------------------------------

/// Renders one filled rect per (x category, y category) pair — a
/// calendar-style value matrix (density marks). Cell v1 is
/// PRE-AGGREGATED: one row per pair, with a numeric `fill:` column carrying
/// the cell's value. Both axes ride the existing per-channel Band inference;
/// each rect is centred via `map_category` and sized via `band_width`.
///
/// A NUMERIC fill maps through the Fill [`Scale::Sequential`] built in
/// [`Self::augment_scales`] — generic column inference types a numeric fill
/// Linear, so the ramp must be built here. A Utf8 fill keeps the existing
/// categorical Colour path (`resolve_colour`) untouched. Self-aggregating
/// `fill: count`/`avg` (a CellLowerer) is deferred with hexbin.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellRenderer {
    /// The continuous colour scheme for numeric fills (default viridis).
    pub scheme: SequentialScheme,
}

impl MarkRenderer for CellRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let (Some(x_col), Some(y_col)) = (channel_map.get(Channel::X), channel_map.get(Channel::Y))
        else {
            return;
        };
        let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            return;
        };
        // Cell v1 is categorical × categorical: both axes must be Band scales
        // with string categories. band_width is None on non-Band scales, so a
        // numeric axis degrades to rendering nothing rather than misplacing.
        let (Some(x_strs), Some(y_strs)) = (
            column_as_string(batch, x_col),
            column_as_string(batch, y_col),
        ) else {
            return;
        };
        let (Some(bw), Some(bh)) = (x_scale.band_width(), y_scale.band_width()) else {
            return;
        };

        // Numeric fill values (None for a Utf8 / absent fill — those take the
        // categorical resolve_colour path below).
        let fill_vals = channel_map
            .get(Channel::Fill)
            .and_then(|c| column_as_f64(batch, c));
        let fill_ramp = match scales.get(Channel::Fill) {
            Some(scale @ Scale::Sequential { .. }) => Some(scale),
            _ => None,
        };

        for i in 0..batch.num_rows() {
            let (Some(xc), Some(yc)) = (x_strs[i].as_deref(), y_strs[i].as_deref()) else {
                continue;
            };
            let (Some(cx), Some(cy)) = (x_scale.map_category(xc), y_scale.map_category(yc)) else {
                continue;
            };
            let colour = match (&fill_ramp, fill_vals.as_ref().and_then(|v| v[i])) {
                (Some(ramp), Some(value)) => Color::new(ramp.map_continuous(value)),
                // A ramp-backed numeric fill whose value is NULL at this row is
                // genuinely NULL — render NULL ink, never a colour a scheme
                // value could produce (the NULL-reads-as-high bug).
                (Some(_), None) if fill_vals.is_some() => scales.ink().null,
                _ => resolve_colour(scales, channel_map, batch, i),
            };
            let cell = kurbo::Rect::new(cx - bw / 2.0, cy - bh / 2.0, cx + bw / 2.0, cy + bh / 2.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &cell);
        }
    }

    /// Build the numeric-fill → colour ramp under [`Channel::Fill`]. Generic
    /// column inference types a numeric fill Linear (the recon's trap), so a
    /// non-colour Fill scale is REPLACED with a Sequential anchored per the v1
    /// rule — `[0, max]` when `min >= 0`, else `[min, max]`. A co-rendered
    /// Sequential unions its domain (keeping the first's stops, mirroring
    /// raster); a categorical Colour Fill (Utf8 fill, or a sibling's swatches)
    /// is left untouched.
    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        _x_range: (f64, f64),
        _y_range: (f64, f64),
    ) {
        let Some(fill_col) = channel_map.get(Channel::Fill) else {
            return;
        };
        // A Utf8 fill reads as None here, leaving the Colour path untouched.
        let Some(vals) = column_as_f64(batch, fill_col) else {
            return;
        };
        let lo = vals.iter().flatten().cloned().fold(f64::INFINITY, f64::min);
        let hi = vals
            .iter()
            .flatten()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        if !(lo.is_finite() && hi.is_finite()) {
            return;
        }
        let (d0, d1) = if lo >= 0.0 { (0.0, hi) } else { (lo, hi) };

        let merged = match scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            }) => Some(Scale::Sequential {
                domain_min: domain_min.min(d0),
                domain_max: domain_max.max(d1),
                stops: stops.clone(),
            }),
            Some(Scale::Colour { .. }) => None, // categorical fill wins
            // Replace the inferred Linear (or synthesise from scratch).
            _ => Some(Scale::Sequential {
                domain_min: d0,
                domain_max: d1,
                stops: self.scheme.stops(),
            }),
        };
        if let Some(scale) = merged {
            scales.insert(Channel::Fill, scale);
        }
    }
}

// ---------------------------------------------------------------------------
// ContourRenderer (contour — iso-lines over the shared KDE grid)
// ---------------------------------------------------------------------------

/// Default iso-level count when the mark declares no `thresholds` (Mosaic's
/// d3-contour-backed fixtures default to ~10 levels).
const DEFAULT_CONTOUR_LEVELS: usize = 10;

/// Renders density iso-lines: marching squares over the same
/// `build_kde_grid` field the heatmap shades, at N evenly-spaced levels
/// (density marks).
///
/// `thresholds` here is the ISO-LEVEL COUNT (Mosaic semantics) — the lowerer
/// registration shields it from the density lowerer's bin-count read, so it
/// never changes the SQL; `bins` still sizes the grid. `bandwidth` overrides
/// Silverman like the heatmap. Stroke is the literal default-colour v1
/// (per-level ramp strokes are deferred).
#[derive(Debug, Clone, Copy, Default)]
pub struct ContourRenderer {
    /// Number of iso-levels (`thresholds:` attr); `DEFAULT_CONTOUR_LEVELS`
    /// when absent.
    pub thresholds: Option<usize>,
    /// Explicit KDE bandwidth in data units (both axes); Silverman per axis
    /// when absent.
    pub bandwidth: Option<f64>,
}

impl MarkRenderer for ContourRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let (Some(x_col), Some(y_col)) = (channel_map.get(Channel::X), channel_map.get(Channel::Y))
        else {
            return;
        };
        let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            return;
        };
        let Some(grid) = build_kde_grid(batch, x_col, y_col, self.bandwidth) else {
            return;
        };

        let levels = crate::contour::iso_levels(
            grid.max_density,
            self.thresholds.unwrap_or(DEFAULT_CONTOUR_LEVELS),
        );
        let stroke = kurbo::Stroke::new(LINE_STROKE_WIDTH);
        let colour = constant_ink(channel_map, scales.ink());
        for level in levels {
            let lines = crate::contour::contour_polylines(
                &grid.density,
                grid.rows(),
                grid.cols(),
                &grid.x_centres,
                &grid.y_centres,
                level,
            );
            for line in lines {
                let mut points = line
                    .iter()
                    .map(|(x, y)| (x_scale.map_f64(*x), y_scale.map_f64(*y)))
                    .filter(|(px, py)| px.is_finite() && py.is_finite());
                let Some(first) = points.next() else { continue };
                let mut path = BezPath::new();
                path.move_to(first);
                for p in points {
                    path.line_to(p);
                }
                scene.stroke(&stroke, Affine::IDENTITY, colour, None, &path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HexbinRenderer (hexbin — pointy-top hexagonal density bins)
// ---------------------------------------------------------------------------

/// Reserved in-band geometry columns the hexbin lowerer emits (constant per
/// row): the hexagon half-width and half-height in DATA units. The six
/// pointy-top vertices are reconstructed from these, so the hex is regular on
/// screen by construction and survives live rebuilds (no bin-step recovery).
/// Must match `brightfield-sql`'s `HEX_DX_COL` / `HEX_DY_COL`.
const HEX_DX_COL: &str = "__bf_hex_dx";
const HEX_DY_COL: &str = "__bf_hex_dy";

/// Reserved RAW-extent columns (constant per row): the raw table min/max of the
/// x/y channels the lowerer binned over. `augment_scales` widens the positional
/// scales from THESE (raw-anchored domain), so the widened domain encodes the
/// exact raw pixel→data pitch and a sibling hexgrid reconstructs the lattice
/// exactly. Must match `brightfield-sql`'s `HEX_X0_COL` … `HEX_Y1_COL`.
const HEX_X0_COL: &str = "__bf_hex_x0";
const HEX_X1_COL: &str = "__bf_hex_x1";
const HEX_Y0_COL: &str = "__bf_hex_y0";
const HEX_Y1_COL: &str = "__bf_hex_y1";

/// Renders one pointy-top hexagon per row (Mosaic's flagship at-scale mark).
/// The hexbin lowerer has already binned in pixel space and emitted, per hex,
/// the centre in DATA units (aliased to the x/y channel columns), the aggregate
/// (`__bf_count` for count, the source column for avg), and the constant hex
/// half-extents `__bf_hex_dx`/`__bf_hex_dy`. This maps each centre through the
/// shared scales and draws the six vertices from the half-extents.
///
/// A count fill ramps the configured Sequential scheme zero-anchored `[0,max]`
/// (with the private `RASTER_MIN_T` visibility floor so the sparsest hex stays
/// visible); an avg fill follows the cell anchoring rule. The scheme rides
/// `MarkInput::renderer_override` (live-rebuild parity — the cfr seam). A
/// missing / non-Sequential Fill scale falls back to alpha-on-default-blue.
#[derive(Debug, Clone, Copy, Default)]
pub struct HexbinRenderer {
    /// The continuous colour scheme (default viridis).
    pub scheme: SequentialScheme,
}

impl HexbinRenderer {
    /// The six pointy-top vertices (in DATA units) of the hex centred at
    /// `(cx, cy)` with half-width `dx` and half-height `dy`, top vertex first.
    fn hex_vertices(cx: f64, cy: f64, dx: f64, dy: f64) -> [(f64, f64); 6] {
        [
            (cx, cy + dy),
            (cx + dx, cy + dy / 2.0),
            (cx + dx, cy - dy / 2.0),
            (cx, cy - dy),
            (cx - dx, cy - dy / 2.0),
            (cx - dx, cy + dy / 2.0),
        ]
    }
}

impl MarkRenderer for HexbinRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let (Some(x_col), Some(y_col)) = (channel_map.get(Channel::X), channel_map.get(Channel::Y))
        else {
            return;
        };
        let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            return;
        };
        let (Some(x_vals), Some(y_vals), Some(dx_vals), Some(dy_vals)) = (
            column_as_f64(batch, x_col),
            column_as_f64(batch, y_col),
            column_as_f64(batch, HEX_DX_COL),
            column_as_f64(batch, HEX_DY_COL),
        ) else {
            return;
        };

        // Fill values + whether this is a count fill (zero-anchored, floored).
        let fill_col = channel_map.get(Channel::Fill);
        let is_count = fill_col == Some(DENSITY_COUNT_COL);
        let fill_vals = fill_col.and_then(|c| column_as_f64(batch, c));
        let max_fill = fill_vals
            .as_ref()
            .map(|v| v.iter().flatten().cloned().fold(0.0_f64, f64::max))
            .unwrap_or(0.0);
        let fill_ramp = match scales.get(Channel::Fill) {
            Some(scale @ Scale::Sequential { .. }) => Some(scale),
            _ => None,
        };
        let [cr, cg, cb, _] = scales.ink().mark_default.components;

        for i in 0..batch.num_rows() {
            let (Some(cx), Some(cy), Some(dx), Some(dy)) =
                (x_vals[i], y_vals[i], dx_vals[i], dy_vals[i])
            else {
                continue;
            };
            let verts = Self::hex_vertices(cx, cy, dx, dy);
            let mut mapped = verts
                .iter()
                .map(|(vx, vy)| (x_scale.map_f64(*vx), y_scale.map_f64(*vy)));
            let Some(first) = mapped.next() else { continue };
            if !(first.0.is_finite() && first.1.is_finite()) {
                continue;
            }
            let mut path = BezPath::new();
            path.move_to(first);
            let mut ok = true;
            for p in mapped {
                if !(p.0.is_finite() && p.1.is_finite()) {
                    ok = false;
                    break;
                }
                path.line_to(p);
            }
            if !ok {
                continue;
            }
            path.close_path();

            let value = fill_vals.as_ref().and_then(|v| v[i]);
            let colour = match (fill_ramp, value) {
                (Some(ramp), Some(v)) => {
                    let dmax = ramp
                        .domain_max()
                        .filter(|d| *d > 0.0)
                        .unwrap_or(max_fill.max(1.0));
                    if is_count {
                        // Zero-anchored count ramp, floored so the sparsest hex
                        // stays visible (raster's RASTER_MIN_T precedent).
                        let pos = (v / dmax).clamp(0.0, 1.0).max(RASTER_MIN_T);
                        Color::new(ramp.map_continuous(pos * dmax))
                    } else {
                        Color::new(ramp.map_continuous(v))
                    }
                }
                // Fallback: single-hue with fill-proportional alpha.
                (None, Some(v)) if max_fill > 0.0 => {
                    let t = (v / max_fill).clamp(0.0, 1.0).max(RASTER_MIN_T) as f32;
                    Color::new([cr, cg, cb, t])
                }
                // A bound numeric fill whose value is NULL at this row is
                // genuinely NULL — NULL ink, never the default colour (which
                // reads as a data value). Other fallthroughs (no fill channel,
                // an all-zero fill) keep the default.
                (_, None) if fill_vals.is_some() => scales.ink().null,
                _ => scales.ink().mark_default,
            };
            scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &path);
        }
    }

    /// Widen the linear x/y domains by half a hex so the outermost hexes fit
    /// inside the plot area, and build the fill → colour ramp under
    /// [`Channel::Fill`]. A count fill is zero-anchored `[0, max]`; an avg fill
    /// follows the cell rule (`[0, max]` when `min >= 0`, else `[min, max]`).
    /// Merge-not-clobber mirrors raster/cell: a co-rendered Sequential unions
    /// its domain (keeping the first's stops); a sibling's categorical Colour
    /// Fill survives untouched.
    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        // Widen each positional scale by one constant half-hex, RAW-ANCHORED:
        // domain = [raw_min - dx, raw_max + dx], where raw_min/raw_max are the
        // lowerer's binning extent carried in-band (__bf_hex_x0/x1/y0/y1) and dx
        // is the constant half-extent (__bf_hex_dx/dy).
        //
        // Raw-anchored — NOT the occupied-centre span — is the contract: it
        // makes the widened domain encode the exact raw pixel→data pitch
        // (span = raw_span·(W+binWidth)/W), which a sibling HexgridRenderer
        // inverts to place its mesh EXACTLY on the hexbin lattice. The
        // occupied-centre span loses that pitch (max_centre - min_centre ≠ raw
        // span, off by up to a hex from quantisation), so the mesh would drift
        // and accumulate multi-pixel error across the lattice — the bug the
        // alignment probe now guards. It also matches every other
        // mark's domain = data extent, and Plot/d3-hexbin (which don't extend
        // the domain for hex overhang at all). The one cost is up to a half-hex
        // clip on the outermost edge hex; bins represent data inside the raw
        // extent, so that is honest. Falls back to the centre-column span only
        // when the raw-extent columns are absent (a non-hexbin batch).
        let first = |c| column_as_f64(batch, c).and_then(|v| v.into_iter().flatten().next());
        let dx = first(HEX_DX_COL);
        let dy = first(HEX_DY_COL);
        for (channel, range, half, lo_col, hi_col) in [
            (Channel::X, x_range, dx, HEX_X0_COL, HEX_X1_COL),
            (Channel::Y, y_range, dy, HEX_Y0_COL, HEX_Y1_COL),
        ] {
            let Some(half) = half else { continue };
            let (lo, hi) = match (first(lo_col), first(hi_col)) {
                (Some(a), Some(b)) => (a, b),
                _ => {
                    // Fallback: occupied-centre span from the channel column.
                    let Some(col) = channel_map.get(channel) else {
                        continue;
                    };
                    let Some(vals) = column_as_f64(batch, col) else {
                        continue;
                    };
                    let lo = vals.iter().flatten().cloned().fold(f64::INFINITY, f64::min);
                    let hi = vals
                        .iter()
                        .flatten()
                        .cloned()
                        .fold(f64::NEG_INFINITY, f64::max);
                    (lo, hi)
                }
            };
            if lo.is_finite() && hi.is_finite() {
                merge_linear_scale(scales, channel, lo - half, hi + half, range);
            }
        }

        // Fill → colour ramp. Count is zero-anchored; avg follows the cell rule.
        let Some(fill_col) = channel_map.get(Channel::Fill) else {
            return;
        };
        let Some(vals) = column_as_f64(batch, fill_col) else {
            return;
        };
        let lo = vals.iter().flatten().cloned().fold(f64::INFINITY, f64::min);
        let hi = vals
            .iter()
            .flatten()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        if !(lo.is_finite() && hi.is_finite()) {
            return;
        }
        let is_count = fill_col == DENSITY_COUNT_COL;
        // Count: [0, max] (counts are ≥ 0). Avg: [0, max] iff min ≥ 0 else [min, max].
        let (d0, d1) = if is_count || lo >= 0.0 {
            (0.0, hi)
        } else {
            (lo, hi)
        };

        let merged = match scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            }) => Some(Scale::Sequential {
                domain_min: domain_min.min(d0),
                domain_max: domain_max.max(d1),
                stops: stops.clone(),
            }),
            Some(Scale::Colour { .. }) => None, // categorical fill wins
            _ => Some(Scale::Sequential {
                domain_min: d0,
                domain_max: d1,
                stops: self.scheme.stops(),
            }),
        };
        if let Some(scale) = merged {
            scales.insert(Channel::Fill, scale);
        }
    }
}

// ---------------------------------------------------------------------------
// HexgridRenderer (hexgrid — decorative dataless hex mesh)
// ---------------------------------------------------------------------------

/// Default `binWidth` (pixels) — matches the hexbin default so a sibling
/// overlays on-lattice.
const DEFAULT_HEX_BIN_WIDTH: f64 = 20.0;

/// Mesh stroke width. The COLOUR comes from
/// [`ChartInk::hexgrid_stroke`](crate::ink::ChartInk::hexgrid_stroke) on the
/// `ScaleSet` this renderer is handed, as the rest of the canvas's paints do —
/// `every_registered_mark_repaints_when_the_mode_changes` is the test that
/// holds that of all of them. Spec-level `stroke`/`strokeOpacity` attrs are
/// still deferred on the literal-colour substrate (the contour precedent).
const HEXGRID_STROKE_WIDTH: f64 = 0.75;

/// Renders a decorative pointy-top hex MESH across the plot area at `binWidth`
/// px — the dataless sibling of hexbin. Ignoring the (singleton) batch, it draws
/// in spec order (before a later hexbin) in one of two modes:
///
/// - **Sibling** — when a hexbin has established RAW-anchored widened data
///   scales, the mesh is the hexbin lattice reconstructed from those scales and
///   drawn through them (the private `sibling_lattice`), so the hexbin overlays
///   it EXACTLY on-lattice (same pitch and phase, not merely the same
///   `binWidth`).
///   Drawing the mesh in raw pixel space at `binWidth` would drift, because the
///   hexbin's centres travel through the half-hex-widened scales.
/// - **Standalone** — a dataless hexgrid-only spec has no widening to invert:
///   [`Self::augment_scales`] synthesises unit x/y scales and the mesh is the
///   plot-corner pixel lattice at `binWidth`.
#[derive(Debug, Clone, Copy)]
pub struct HexgridRenderer {
    /// Hex `binWidth` in pixels (horizontal centre spacing).
    pub bin_width: f64,
}

impl Default for HexgridRenderer {
    fn default() -> Self {
        Self {
            bin_width: DEFAULT_HEX_BIN_WIDTH,
        }
    }
}

/// The pixel range `(start, end)` a scale maps onto, for the positional scales.
fn scale_pixel_range(scale: &Scale) -> Option<(f64, f64)> {
    match scale {
        Scale::Linear {
            range_start,
            range_end,
            ..
        }
        | Scale::Time {
            range_start,
            range_end,
            ..
        }
        | Scale::Band {
            range_start,
            range_end,
            ..
        } => Some((*range_start, *range_end)),
        _ => None,
    }
}

/// `(domain_min, domain_max, range_start, range_end)` of a linear scale — the
/// hexbin-widened positional scales the hexgrid rides. `None` for any other
/// scale kind (the hexgrid then falls back to its plot-corner pixel mesh).
fn linear_parts(scale: &Scale) -> Option<(f64, f64, f64, f64)> {
    match scale {
        Scale::Linear {
            domain_min,
            domain_max,
            range_start,
            range_end,
        } => Some((*domain_min, *domain_max, *range_start, *range_end)),
        _ => None,
    }
}

/// The hexbin lattice recovered in DATA units from a pair of hexbin-WIDENED
/// positional scales, so a sibling hexgrid mesh coincides with the hexbin's
/// hexes exactly rather than drifting in pitch and phase.
///
/// The hexbin lowerer bins in RAW plot-pixel space (`px = (x-xmin)/xspan·W`),
/// anchoring hex (0,0) at data `(xmin, ymin)`, then `augment_scales` widens the
/// domain by one constant half-hex per axis, RAW-anchored: `[xmin - dx,
/// xmax + dx]`. That widening is exact and invertible — given the widened
/// domain, the pixel range `W`/`H`, and `binWidth`, we recover the raw data
/// extent `[xmin, xmax]` the lowerer binned over (bit-exact because the domain
/// is anchored on that extent, NOT on quantised occupied centres), regenerate
/// the raw-pixel lattice anchored at (0,0) — so the row stagger matches the
/// lowerer bit-for-bit — and map each centre back to data. The mesh is then
/// drawn through the SAME scales as the hexes, so they land together. See the
/// hexgrid alignment probe.
///
/// The exactness holds on the FRESH / static-scale path (a plot's initial
/// build, and any rebuild that re-derives the scales from the re-executed
/// batch). It relies on the widened domain matching the batch the mesh is drawn
/// against. Under the live ANCHORED path (`anchor_scales`'s widen-only union,
/// which holds the launch domain while data widens) a rebuild whose binning
/// EXTENT changed while the held domain did not would make the reconstruction
/// invert a stale domain and the mesh drift (~9.6px measured). That is
/// unreachable today — selection cross-filters wrap in an outer `Filter` and do
/// NOT re-bin, `binWidth` is literal-only, and no example composes a
/// param-driven `data.filter` on a hexbin — so it is a documented limit, not a
/// regression; revisit if binWidth/extent ever become param-driven.
struct SiblingLattice {
    bin_width: f64,
    /// Raw data extent (un-widened) the lowerer binned over.
    xmin_raw: f64,
    xspan_raw: f64,
    ymin_raw: f64,
    yspan_raw: f64,
    /// Raw plot pixel extent (the scale range magnitude, unchanged by widening).
    w: f64,
    h: f64,
    /// Constant hex half-extents in data units (equal to the lowerer's emitted
    /// `__bf_hex_dx`/`__bf_hex_dy`).
    dx_data: f64,
    dy_data: f64,
}

impl SiblingLattice {
    /// Hex centres in DATA units, over the raw plot rect (with the one-cell
    /// margin `lattice_centres` adds), ready to map through the shared scales.
    fn data_centres(&self) -> Vec<(f64, f64)> {
        HexgridRenderer::lattice_centres(0.0, self.w, 0.0, self.h, self.bin_width)
            .into_iter()
            .map(|(px, py)| {
                (
                    self.xmin_raw + px * self.xspan_raw / self.w,
                    self.ymin_raw + py * self.yspan_raw / self.h,
                )
            })
            .collect()
    }
}

impl HexgridRenderer {
    /// Pointy-top hex centres (pixel space) covering the rect `[x0,x1]×[y0,y1]`
    /// at `bin_width`, with a one-cell margin so the clip trims cleanly. Rows are
    /// `dy = 1.5·size` apart (`size = bin_width/√3`), odd rows offset by half a
    /// horizontal step (`dx = bin_width`) — the d3-hexbin / Observable Plot mesh.
    fn lattice_centres(x0: f64, x1: f64, y0: f64, y1: f64, bin_width: f64) -> Vec<(f64, f64)> {
        let sqrt3 = 1.732_050_807_568_877_2_f64;
        let size = bin_width / sqrt3;
        let dx = bin_width;
        let dy = 1.5 * size;
        if !(dx > 0.0 && dy > 0.0) {
            return Vec::new();
        }
        let (lo_x, hi_x) = (x0.min(x1), x0.max(x1));
        let (lo_y, hi_y) = (y0.min(y1), y0.max(y1));
        let mut out = Vec::new();
        let j_max = ((hi_y - lo_y) / dy).ceil() as i64 + 1;
        let i_max = ((hi_x - lo_x) / dx).ceil() as i64 + 1;
        for j in -1..=j_max {
            let cy = lo_y + (j as f64) * dy;
            let offset = if j.rem_euclid(2) == 1 { dx / 2.0 } else { 0.0 };
            for i in -1..=i_max {
                let cx = lo_x + offset + (i as f64) * dx;
                out.push((cx, cy));
            }
        }
        out
    }

    /// Recover the sibling-hexbin lattice from the RAW-anchored widened scales,
    /// so the mesh rides the hexbin's exact hexes. Returns `None` — falling back
    /// to the plot-corner pixel mesh — when there is no sibling to align to: the
    /// scales are the synthesised unit `[0,1]` scales of a DATALESS standalone
    /// hexgrid, are non-linear, or the geometry is degenerate.
    ///
    /// KNOWN v1 LIMIT: the trigger is "linear, non-unit, non-degenerate scales",
    /// NOT "a hexbin sibling exists" — the renderer has no cross-mark visibility.
    /// So a hexgrid co-plotted with a NON-hexbin mark (a dot/scatter) that
    /// established a plain data domain would reconstruct a spurious hex lattice
    /// from THAT mark's (un-hex-widened) domain instead of the plot-corner mesh.
    /// The mesh would still be a plausible hex grid, just not the plot-corner
    /// one. Distinguishing a hexbin-widened domain from any other needs a
    /// cross-mark seam we deliberately do not build here; the ratified use is
    /// hexgrid + hexbin, where this is correct. Revisit if hexgrid-over-non-
    /// hexbin becomes a supported composition.
    // clippy::neg_cmp_op_on_partial_ord flags `!(b > 0.0)`. That form is the
    // point: these are f64s that reach here from scale inversion and from a
    // user-supplied `binWidth`, so NaN is reachable. `!(b > 0.0)` rejects NaN;
    // the suggested `b <= 0.0` accepts it and lets a NaN lattice pitch through.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn sibling_lattice(&self, x_scale: &Scale, y_scale: &Scale) -> Option<SiblingLattice> {
        let (x0d, x1d, rsx, rex) = linear_parts(x_scale)?;
        let (y0d, y1d, rsy, rey) = linear_parts(y_scale)?;
        // A dataless standalone hexgrid: augment_scales wrote the exact unit
        // scales below, so bit-exact equality against its own constants is the
        // signal (a hexbin-widened data domain is never exactly [0,1]). Keep the
        // plot-corner pixel mesh — there is no widening to invert.
        let is_unit = |lo: f64, hi: f64| lo == 0.0 && hi == 1.0;
        if is_unit(x0d, x1d) && is_unit(y0d, y1d) {
            return None;
        }
        let b = self.bin_width;
        if !(b > 0.0) {
            return None;
        }
        let sqrt3 = 1.732_050_807_568_877_2_f64;
        let size = b / sqrt3;
        let w = (rex - rsx).abs();
        let h = (rey - rsy).abs();
        let span_x = x1d - x0d;
        let span_y = y1d - y0d;
        if !(w > 0.0 && h > 0.0 && span_x > 0.0 && span_y > 0.0) {
            return None;
        }
        // Un-widen: augment added one half-hex (data units) per side, RAW-
        // anchored — `widened_span = raw_span + 2·half`, half = (px_half/plot)·
        // raw_span (half-width binWidth/2 px on x, half-height size px on y).
        // Solving for the raw span/half recovers the lowerer's raw affine
        // exactly; the recovered `dx_data`/`dy_data` equal its emitted
        // `__bf_hex_dx`/`_dy`, and `xmin_raw`/`ymin_raw` its raw `min`.
        let dx_data = (b / 2.0) * span_x / (w + b);
        let dy_data = size * span_y / (h + 2.0 * size);
        Some(SiblingLattice {
            bin_width: b,
            xmin_raw: x0d + dx_data,
            xspan_raw: span_x - 2.0 * dx_data,
            ymin_raw: y0d + dy_data,
            yspan_raw: span_y - 2.0 * dy_data,
            w,
            h,
            dx_data,
            dy_data,
        })
    }
}

impl MarkRenderer for HexgridRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        _batch: &RecordBatch,
        _channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            return;
        };
        let stroke = kurbo::Stroke::new(HEXGRID_STROKE_WIDTH);
        let colour = scales.ink().hexgrid_stroke;

        if let Some(lat) = self.sibling_lattice(x_scale, y_scale) {
            // Sibling hexbin: draw the mesh in DATA units and map through the
            // shared (widened) scales, so it coincides with the hexbin's hexes.
            for (cx, cy) in lat.data_centres() {
                let verts = HexbinRenderer::hex_vertices(cx, cy, lat.dx_data, lat.dy_data);
                let mut mapped = verts
                    .iter()
                    .map(|(vx, vy)| (x_scale.map_f64(*vx), y_scale.map_f64(*vy)));
                let Some(first) = mapped.next() else { continue };
                if !(first.0.is_finite() && first.1.is_finite()) {
                    continue;
                }
                let mut path = BezPath::new();
                path.move_to(first);
                let mut ok = true;
                for p in mapped {
                    if !(p.0.is_finite() && p.1.is_finite()) {
                        ok = false;
                        break;
                    }
                    path.line_to(p);
                }
                if ok {
                    path.close_path();
                    scene.stroke(&stroke, Affine::IDENTITY, colour, None, &path);
                }
            }
            return;
        }

        // Standalone (dataless) hexgrid: plot-corner pixel mesh at binWidth.
        let (Some((x0, x1)), Some((y0, y1))) =
            (scale_pixel_range(x_scale), scale_pixel_range(y_scale))
        else {
            return;
        };
        let sqrt3 = 1.732_050_807_568_877_2_f64;
        let size = self.bin_width / sqrt3;
        let (dx, dy) = (self.bin_width / 2.0, size); // half-width, half-height
        for (cx, cy) in Self::lattice_centres(x0, x1, y0, y1, self.bin_width) {
            let verts = HexbinRenderer::hex_vertices(cx, cy, dx, dy);
            let mut path = BezPath::new();
            path.move_to(verts[0]);
            for v in &verts[1..] {
                path.line_to(*v);
            }
            path.close_path();
            scene.stroke(&stroke, Affine::IDENTITY, colour, None, &path);
        }
    }

    /// Synthesise unit x/y linear scales when none exist, so a DATALESS
    /// hexgrid-only spec still has a plot-area pixel rect to draw the mesh in.
    /// When a sibling mark (e.g. hexbin) has already established data-driven x/y
    /// scales, this is a no-op — `render` then reconstructs the mesh from those
    /// scales (the private `sibling_lattice`) so it stays exactly on-lattice.
    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        _batch: &RecordBatch,
        _channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        if scales.get(Channel::X).is_none() {
            scales.insert(
                Channel::X,
                Scale::Linear {
                    domain_min: 0.0,
                    domain_max: 1.0,
                    range_start: x_range.0,
                    range_end: x_range.1,
                },
            );
        }
        if scales.get(Channel::Y).is_none() {
            scales.insert(
                Channel::Y,
                Scale::Linear {
                    domain_min: 0.0,
                    domain_max: 1.0,
                    range_start: y_range.0,
                    range_end: y_range.1,
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------
// RegressionRenderer (regressionY / regressionX)
// ---------------------------------------------------------------------------

/// Two-tailed Student-t critical value for confidence level `ci` and sample
/// size `n`. Degrees of freedom is `n - 2` (OLS with one slope + one intercept).
///
/// Implementation: small lookup table for the canonical CIs (0.90, 0.95, 0.99)
/// at common df values, with linear interpolation between bracketing rows.
/// For df ≥ 30 the values approach the normal-distribution z-quantiles
/// (1.645, 1.96, 2.576), and for df ≥ 60 we use those directly. For df < 1
/// the band is undefined; the caller (`band_enabled`) gates this and we
/// return 0 here as a safe fallback.
fn t_critical(ci: f64, n: f64) -> f64 {
    let df = (n - 2.0).max(0.0);

    // Pick the bracketing CI column. We support 0.90 / 0.95 / 0.99 exactly;
    // values in between snap to the nearest standard column. Out-of-range
    // values clamp to 0.95.
    let column = if ci >= 0.99 {
        2 // 0.99
    } else if ci >= 0.95 {
        1 // 0.95
    } else if ci >= 0.90 {
        0 // 0.90
    } else {
        1 // default to 0.95
    };

    // Two-tailed critical values t(α/2, df) for α = 0.10, 0.05, 0.01.
    // Source: standard t-tables; values rounded to 3 decimal places.
    // Each row: (df, t_0.10, t_0.05, t_0.01).
    const ROWS: &[(f64, [f64; 3])] = &[
        (1.0, [6.314, 12.706, 63.657]),
        (2.0, [2.920, 4.303, 9.925]),
        (3.0, [2.353, 3.182, 5.841]),
        (4.0, [2.132, 2.776, 4.604]),
        (5.0, [2.015, 2.571, 4.032]),
        (6.0, [1.943, 2.447, 3.707]),
        (7.0, [1.895, 2.365, 3.499]),
        (8.0, [1.860, 2.306, 3.355]),
        (9.0, [1.833, 2.262, 3.250]),
        (10.0, [1.812, 2.228, 3.169]),
        (12.0, [1.782, 2.179, 3.055]),
        (15.0, [1.753, 2.131, 2.947]),
        (20.0, [1.725, 2.086, 2.845]),
        (25.0, [1.708, 2.060, 2.787]),
        (30.0, [1.697, 2.042, 2.750]),
        (60.0, [1.671, 2.000, 2.660]),
    ];
    const Z_LIMIT: [f64; 3] = [1.645, 1.960, 2.576];

    if df < 1.0 {
        return 0.0;
    }
    if df >= 60.0 {
        return Z_LIMIT[column];
    }

    // Linear interpolation between bracketing rows.
    for w in ROWS.windows(2) {
        let (df_lo, vals_lo) = w[0];
        let (df_hi, vals_hi) = w[1];
        if df >= df_lo && df <= df_hi {
            let t = (df - df_lo) / (df_hi - df_lo);
            return vals_lo[column] + t * (vals_hi[column] - vals_lo[column]);
        }
    }
    // df > 60 handled above; df < 1 handled above; everything in between
    // matched a row. Safe fallback to z.
    Z_LIMIT[column]
}

/// Renders a linear OLS fit line plus a 95% (or configurable) CI band.
///
/// Expects a one-row aggregate batch with columns:
///   - `slope` — regr_slope(y, x)
///   - `intercept` — regr_intercept(y, x)
///   - `n` — regr_count(y, x)
///   - `x_bar` — regr_avgx(y, x)
///   - `sxx` — regr_sxx(y, x)  (sum (x - x_bar)^2)
///   - `sxy` — regr_sxy(y, x)  (sum (x - x_bar)(y - mean_y))
///   - `syy` — regr_syy(y, x)  (sum (y - mean_y)^2)
///
/// The fitted line is sampled at 32 evenly-spaced x values across the
/// x-axis domain. A confidence band is drawn as a filled path between
/// upper and lower bounds at each sample point.
pub struct RegressionRenderer {
    /// Confidence level (e.g. 0.95). The renderer uses the normal-approximation
    /// `z_{1-alpha/2}` for `n >= 30`, defaulting to 1.96 for 95%.
    pub ci: f64,
}

impl Default for RegressionRenderer {
    fn default() -> Self {
        Self { ci: 0.95 }
    }
}

impl RegressionRenderer {
    /// The one drawing routine. `beyond_frame` decides only how the fitted line
    /// and the band's edges are stroked — everything the fit CLAIMS is computed
    /// identically either way, because recomputing it over the visible rows
    /// would be a different answer to a question nobody asked, and it was
    /// considered and not taken.
    ///
    /// # Why the band wears the caveat too
    ///
    /// The band is the interval claim itself, it is computed from the same rows
    /// the frame excludes, and it is by area the larger half of this mark. A
    /// treatment that dashed only the fitted line would leave the bigger object
    /// asserting confidence over a range the fit no longer speaks for — half
    /// the mark saying "this summarises data outside the frame" and half not.
    ///
    /// The band keeps its FILL at full [`BAND_ALPHA`] and gains a dashed edge on
    /// the fit's own [`BEYOND_FRAME_DASH`]/[`BEYOND_FRAME_GAP`] rhythm. Both
    /// halves of that are constrained rather than chosen. Dropping or hollowing
    /// the fill is the "refuse to draw" that was rejected for the whole mark,
    /// applied to half of it — the interval would simply stop being stated,
    /// which is a worse answer than an unqualified one. Thinning or fading
    /// the fill is the desaturation that [`dash_polyline`] argues against at
    /// length, and it fails here for the extra reason that a band's alpha is
    /// read as its confidence level. What is left is texture, and the texture
    /// the fit already uses, so the mark speaks with one vocabulary instead of
    /// growing a second.
    fn draw(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        beyond_frame: bool,
    ) {
        let x_scale = match scales.get(Channel::X) {
            Some(s) => s,
            None => return,
        };
        let y_scale = match scales.get(Channel::Y) {
            Some(s) => s,
            None => return,
        };

        // Read regression aggregates from the (single-row) batch.
        // For multi-group rendering, the batch has multiple rows — one per
        // stroke category.
        let slope_vals = match column_as_f64(batch, "slope") {
            Some(v) => v,
            None => return,
        };
        let intercept_vals = match column_as_f64(batch, "intercept") {
            Some(v) => v,
            None => return,
        };
        let n_vals = match column_as_f64(batch, "n") {
            Some(v) => v,
            None => return,
        };
        let x_bar_vals = match column_as_f64(batch, "x_bar") {
            Some(v) => v,
            None => return,
        };
        let sxx_vals = match column_as_f64(batch, "sxx") {
            Some(v) => v,
            None => return,
        };
        let sxy_vals = match column_as_f64(batch, "sxy") {
            Some(v) => v,
            None => return,
        };
        let syy_vals = match column_as_f64(batch, "syy") {
            Some(v) => v,
            None => return,
        };

        // x sampling domain — full x-axis domain.
        let x_min = match x_scale.domain_min() {
            Some(v) => v,
            None => return,
        };
        let x_max = match x_scale.domain_max() {
            Some(v) => v,
            None => return,
        };

        const SAMPLES: usize = 32;

        // Stroke colour resolution: one row per group (if any), else default.
        for row in 0..batch.num_rows() {
            let slope = match slope_vals[row] {
                Some(v) => v,
                None => continue,
            };
            let intercept = match intercept_vals[row] {
                Some(v) => v,
                None => continue,
            };
            // Spec: render the fitted line for n >= 2; suppress only the CI
            // band when df = n - 2 < 1 (n < 3) — variance estimate undefined.
            let n = match n_vals[row] {
                Some(v) if v >= 2.0 => v,
                _ => continue,
            };
            // band_enabled: whether to draw the CI band on top of the line.
            let band_enabled = n >= 3.0;
            let x_bar = x_bar_vals[row].unwrap_or(0.0);
            let sxx = sxx_vals[row].unwrap_or(0.0);
            let sxy = sxy_vals[row].unwrap_or(0.0);
            let syy = syy_vals[row].unwrap_or(0.0);

            // Residual variance: s² = (Syy - Sxy²/Sxx) / (n - 2)
            // Only meaningful when n >= 3; for n == 2 we still draw the line
            // (the OLS fit is exact through both points).
            let s_sq = if band_enabled && sxx > 0.0 {
                (syy - (sxy * sxy) / sxx) / (n - 2.0)
            } else {
                0.0
            };
            let s = s_sq.max(0.0).sqrt();

            let colour = resolve_stroke_colour(scales, channel_map, batch, row);

            // Sample CI band points.
            let mut upper: Vec<(f64, f64)> = Vec::with_capacity(SAMPLES);
            let mut lower: Vec<(f64, f64)> = Vec::with_capacity(SAMPLES);
            let mut line_pts: Vec<(f64, f64)> = Vec::with_capacity(SAMPLES);
            for i in 0..SAMPLES {
                let t = (i as f64) / ((SAMPLES - 1) as f64);
                let xv = x_min + (x_max - x_min) * t;
                let yhat = slope * xv + intercept;
                // se(ŷ|x) = s · √(1/n + (x - x_bar)² / sxx)
                let se = if sxx > 0.0 {
                    s * (1.0 / n + (xv - x_bar).powi(2) / sxx).sqrt()
                } else {
                    0.0
                };
                let half = t_critical(self.ci, n) * se;

                let px = x_scale.map_f64(xv);
                let py_line = y_scale.map_f64(yhat);
                line_pts.push((px, py_line));
                upper.push((px, y_scale.map_f64(yhat + half)));
                lower.push((px, y_scale.map_f64(yhat - half)));
            }

            // Draw CI band as a filled polygon (upper forward, lower reversed).
            // Suppressed when n < 3 — variance estimate has no degrees of
            // freedom; the line still renders below.
            if band_enabled {
                let mut band = BezPath::new();
                band.move_to(upper[0]);
                for &p in &upper[1..] {
                    band.line_to(p);
                }
                for &p in lower.iter().rev() {
                    band.line_to(p);
                }
                band.close_path();

                let [cr, cg, cb, _] = colour.components;
                let band_colour = Color::new([cr, cg, cb, BAND_ALPHA]);
                scene.fill(Fill::NonZero, Affine::IDENTITY, band_colour, None, &band);

                // …and, when the interval it draws was computed from rows the
                // frame excludes, break its two edges on the fit's own rhythm.
                // The fill stays; only the boundary picks up the texture, at
                // the band's own alpha so it stays the quieter statement.
                if beyond_frame {
                    let edge = kurbo::Stroke::new(LINE_STROKE_WIDTH);
                    for bound in [&upper, &lower] {
                        for run in dash_polyline(bound, BEYOND_FRAME_DASH, BEYOND_FRAME_GAP) {
                            scene.stroke(&edge, Affine::IDENTITY, band_colour, None, &run);
                        }
                    }
                }
            }

            // Draw the fitted line on top — solid when the fit describes what
            // is on screen, dashed when it still summarises rows that are not.
            // Same colour, same width, same geometry: only the texture moves,
            // so the mark keeps its identity while ceasing to look like a fit
            // over the visible points.
            let stroke = kurbo::Stroke::new(LINE_STROKE_WIDTH);
            let segments = if beyond_frame {
                dash_polyline(&line_pts, BEYOND_FRAME_DASH, BEYOND_FRAME_GAP)
            } else {
                line_pts
                    .windows(2)
                    .map(|w| {
                        Line::new(
                            kurbo::Point::new(w[0].0, w[0].1),
                            kurbo::Point::new(w[1].0, w[1].1),
                        )
                    })
                    .collect()
            };
            for line in segments {
                scene.stroke(&stroke, Affine::IDENTITY, colour, None, &line);
            }
        }
    }
}

impl MarkRenderer for RegressionRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        self.draw(scene, batch, channel_map, scales, false);
    }

    /// The fit is dashed and so are the two edges of its confidence band —
    /// same colour, same widths, same geometry, only the texture moves.
    /// `dash_polyline` in this module carries the argument for that treatment
    /// over the two beside it (desaturation, an end-cap); the private `draw`
    /// below carries the argument for the band wearing it as well as the line.
    fn render_beyond_frame(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        self.draw(scene, batch, channel_map, scales, true);
    }

    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        _channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        // The executed batch holds only coefficients (no raw x/y rows), so build
        // the x/y scales the renderer samples over from the emitted data
        // extents. Unioned with any sibling-provided scale via merge_linear_scale.
        if let Some((min, max)) = column_extent(batch, "x_min", "x_max") {
            merge_linear_scale(scales, Channel::X, min, max, x_range);
        }
        if let Some((min, max)) = column_extent(batch, "y_min", "y_max") {
            merge_linear_scale(scales, Channel::Y, min, max, y_range);
        }
    }
}

/// Min of `min_col` and max of `max_col` across all rows (the regression batch
/// has one row, or one per stroke group). `None` if either column is absent or
/// entirely null.
fn column_extent(batch: &RecordBatch, min_col: &str, max_col: &str) -> Option<(f64, f64)> {
    let mins = column_as_f64(batch, min_col)?;
    let maxs = column_as_f64(batch, max_col)?;
    let lo = mins.into_iter().flatten().fold(f64::INFINITY, f64::min);
    let hi = maxs.into_iter().flatten().fold(f64::NEG_INFINITY, f64::max);
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

/// Resolve stroke colour for regression — checks `stroke` channel value first,
/// falls back to fill, then default. A `stroke` bound to a colour CONSTANT wins
/// outright, for the reason [`resolve_colour`] checks its own constant first.
fn resolve_stroke_colour(
    scales: &ScaleSet,
    channel_map: &ChannelMap,
    batch: &RecordBatch,
    row: usize,
) -> Color {
    if let Some(constant) = channel_map.colour(Channel::Stroke) {
        return constant;
    }
    if let Some(stroke_col) = channel_map.get(Channel::Stroke) {
        if let Some(stroke_scale) = scales.get(Channel::Stroke) {
            if let Some(strings) = column_as_string(batch, stroke_col) {
                if let Some(Some(ref cat)) = strings.get(row) {
                    if let Some(components) = stroke_scale.map_colour(cat) {
                        return Color::new(components);
                    }
                }
            }
        }
    }
    resolve_colour(scales, channel_map, batch, row)
}

// ---------------------------------------------------------------------------
// GeoRenderer (geo — projected GeoJSON basemap / choropleth)
// ---------------------------------------------------------------------------

/// Client-side map projection forward math — the pure-math half of the geo
/// semantic split. WHICH projection is a spec decision
/// ([`brightfield_spec::layout::ResolvedProjection`], converted via `From`); the
/// forward transform lives here.
///
/// The projections here output "math-convention" coordinates (`v` increasing
/// NORTHWARD). The renderer feeds them through the plot's inverted Y
/// [`Scale::Linear`] (`ChartLayout::y_range` is `(bottom, top)`), which supplies
/// the screen flip so north renders up — so, unlike a scale-free d3 projection,
/// there is NO `-lat` negation here (it would double-flip).
///
/// # Where the formulas come from
///
/// Each arm is d3-geo's raw projection (`d3-geo/src/projection/*.js`, ISC),
/// transcribed with d3's own default parameters, because Mosaic's
/// `projectionType` vocabulary IS Observable Plot's, and Plot's projections are
/// d3-geo's. `Projection::project` takes DEGREES where a d3 raw takes radians;
/// that conversion is the systematic divergence, and it is per-arm rather than
/// global because [`Self::Identity`] and [`Self::ReflectY`] are planar
/// passthroughs that d3 does not convert either.
///
/// The expected values in this crate's tests were produced by an oracle written
/// independently of this code and cross-checked two ways — see
/// `tests/projection_reference.rs` for the provenance of every literal.
///
/// # Scale and translation are NOT applied
///
/// d3 composes a raw projection with `scale`/`translate`/`rotate`/`center`.
/// Brightfield applies neither the scale nor the translation: the renderer
/// aspect-fits the projected bounding box into the plot rect
/// (`aspect_fit_domains`), so a uniform scale and any translation are absorbed
/// by the fit and cannot change the picture. A ROTATION is not absorbed, which
/// is why `projectionRotate` remains unread and each projection here draws at
/// d3's default rotation — [`Self::Albers`] excepted, because d3 bakes its
/// `rotate([96, 0])` into the projection itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Projection {
    /// `u = lon`, `v = lat` — the identity plate carrée, aspect-fit by the
    /// renderer. The default when `projectionType` is absent / unrecognised.
    #[default]
    Equirectangular,
    /// d3's `geoIdentity` — a planar passthrough, `u = lon`, `v = lat`. Draws
    /// the same picture as [`Self::Equirectangular`] under the fit; a separate
    /// variant because it is a separate name in the spec language.
    Identity,
    /// [`Self::Identity`] with the latitude axis flipped (`v = -lat`).
    ReflectY,
    /// Spherical Mercator. Conformal — local shape survives at each latitude,
    /// which is what an unprojected lon/lat scatter gets wrong away from the
    /// equator. Undefined at the poles: beyond [`MERCATOR_CLIP_LAT`] a
    /// coordinate has no position and `project` returns `None`.
    Mercator,
    /// Transverse spherical Mercator at d3's default `rotate([0, 0, 90])` —
    /// conformal about the prime meridian rather than about the equator.
    TransverseMercator,
    /// Orthographic — the globe seen from infinitely far away. The far
    /// hemisphere has no position.
    Orthographic,
    /// Stereographic — conformal azimuthal. The antipode has no position.
    Stereographic,
    /// Gnomonic — great circles draw straight. Only the near hemisphere has a
    /// position, and it diverges towards the rim.
    Gnomonic,
    /// Lambert azimuthal equal-area.
    AzimuthalEqualArea,
    /// Azimuthal equidistant.
    AzimuthalEquidistant,
    /// Equal Earth (Šavrič, Patterson & Jenny, 2018) — equal-area and
    /// pseudocylindrical, defined at every latitude, so a world point map keeps
    /// its clustering honest without dropping polar rows.
    EqualEarth,
    /// Albers conic equal-area at d3's default standard parallels (0°, 60°).
    ConicEqualArea,
    /// Lambert conic conformal at d3's default standard parallels (30°, 30°).
    ConicConformal,
    /// Conic equidistant at d3's default standard parallels (0°, 60°).
    ConicEquidistant,
    /// US-tuned Albers equal-area conic (fixed standard parallels 29.5°N/45.5°N,
    /// reference (−96°, 23°) — d3-geo's `geoAlbers` US defaults). Contiguous-US
    /// correct; AK/HI render in true geographic position (the albers-usa
    /// composite is deferred, a stated gap).
    Albers,
}

impl From<brightfield_spec::layout::ResolvedProjection> for Projection {
    fn from(p: brightfield_spec::layout::ResolvedProjection) -> Self {
        use brightfield_spec::layout::ResolvedProjection as R;
        match p {
            R::Equirectangular => Self::Equirectangular,
            R::Identity => Self::Identity,
            R::ReflectY => Self::ReflectY,
            R::Mercator => Self::Mercator,
            R::TransverseMercator => Self::TransverseMercator,
            R::Orthographic => Self::Orthographic,
            R::Stereographic => Self::Stereographic,
            R::Gnomonic => Self::Gnomonic,
            R::AzimuthalEqualArea => Self::AzimuthalEqualArea,
            R::AzimuthalEquidistant => Self::AzimuthalEquidistant,
            R::EqualEarth => Self::EqualEarth,
            R::ConicEqualArea => Self::ConicEqualArea,
            R::ConicConformal => Self::ConicConformal,
            R::ConicEquidistant => Self::ConicEquidistant,
            R::Albers => Self::Albers,
        }
    }
}

/// Degrees to radians.
const D2R: f64 = std::f64::consts::PI / 180.0;

/// d3-geo's Mercator clip latitude, `atan(sinh(π))` in degrees, pinned by the
/// test `the_mercator_clip_latitude_is_atan_sinh_pi`. Beyond it
/// [`Projection::Mercator`] diverges, and d3 clips the geometry away rather than
/// drawing it at an arbitrary distance; [`Projection::project`] returns `None`
/// there for the same reason.
pub const MERCATOR_CLIP_LAT: f64 = 85.051_128_779_806_6;

/// The margin by which a coordinate must be on the near side of the horizon for
/// an azimuthal projection to give it a position. Guards the divide-by-zero at
/// the rim (gnomonic) and at the antipode (stereographic, azimuthal).
const HORIZON_EPS: f64 = 1e-6;

impl Projection {
    /// Project `(lon, lat)` in DEGREES to planar `(u, v)`, with `v` increasing
    /// north. Pure — no allocation.
    ///
    /// `None` means this projection has no position for that coordinate: the far
    /// hemisphere under [`Self::Orthographic`] / [`Self::Gnomonic`], the antipode
    /// under [`Self::Stereographic`] / the azimuthals, or a latitude past
    /// [`MERCATOR_CLIP_LAT`] under either Mercator. The caller SKIPS such a
    /// coordinate — the alternative is drawing it at a mirrored or effectively
    /// infinite position, which is a lie rather than an approximation. The
    /// projections that are total (equirectangular, identity, reflect-y, equal
    /// earth, the conics, albers) return a position for each coordinate they are
    /// given, and the test `an_unrepresentable_coordinate_has_no_position` holds
    /// both halves.
    ///
    /// **A spec that predates this can still lose a point**, because a spec that
    /// names `orthographic` was drawing the plate carrée before the catalogue
    /// widened and draws an orthographic now — the vendored
    /// `earthquakes-globe.yaml` is exactly that spec. What is true is narrower:
    /// a spec whose `projectionType` this build ALREADY recognised —
    /// `equirectangular`, `albers`, `albers-usa` — changes nothing it draws,
    /// because those names are total, which the last block of
    /// `an_unrepresentable_coordinate_has_no_position` enumerates.
    #[must_use]
    pub fn project(self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        match self {
            Self::Equirectangular | Self::Identity => Some((lon, lat)),
            Self::ReflectY => Some((lon, -lat)),
            Self::Mercator => {
                (lat.abs() < MERCATOR_CLIP_LAT).then(|| (lon * D2R, mercator_y(lat * D2R)))
            }
            Self::TransverseMercator => {
                // d3's geoTransverseMercator is the raw `[log(tan((π/2+φ)/2)), -λ]`
                // under a default `rotate([0, 0, 90])`. Composing the two gives
                // Snyder's closed form for the spherical transverse Mercator,
                // which is what this computes: it is the same map with no
                // rotation machinery, and `tests/projection_reference.rs` pins
                // the two against each other.
                let (lam, phi) = (lon * D2R, lat * D2R);
                // `b` is the sine of the ROTATED latitude, which is the one
                // Mercator's clip applies to — d3 clips the rotated sphere, not
                // the input coordinate, so a point on the equator at ±90°
                // longitude is what falls off here rather than a polar one.
                let b = phi.cos() * lam.sin();
                (b.abs() < (MERCATOR_CLIP_LAT * D2R).sin()).then(|| {
                    (
                        0.5 * ((1.0 + b) / (1.0 - b)).ln(),
                        phi.tan().atan2(lam.cos()),
                    )
                })
            }
            Self::Orthographic => {
                let (lam, phi) = (lon * D2R, lat * D2R);
                near_side(lam, phi).then(|| (phi.cos() * lam.sin(), phi.sin()))
            }
            Self::Gnomonic => {
                let (lam, phi) = (lon * D2R, lat * D2R);
                let (cy, k) = (phi.cos(), lam.cos() * phi.cos());
                (k > HORIZON_EPS).then(|| (cy * lam.sin() / k, phi.sin() / k))
            }
            Self::Stereographic => {
                let (lam, phi) = (lon * D2R, lat * D2R);
                let cy = phi.cos();
                let k = 1.0 + lam.cos() * cy;
                (k > HORIZON_EPS).then(|| (cy * lam.sin() / k, phi.sin() / k))
            }
            Self::AzimuthalEqualArea => azimuthal(lon, lat, |cxcy| (2.0 / (1.0 + cxcy)).sqrt()),
            Self::AzimuthalEquidistant => azimuthal(lon, lat, |cxcy| {
                let c = cxcy.clamp(-1.0, 1.0).acos();
                if c == 0.0 {
                    1.0
                } else {
                    c / c.sin()
                }
            }),
            Self::EqualEarth => Some(equal_earth_forward(lon * D2R, lat * D2R)),
            Self::ConicEqualArea => Some(conic_equal_area_forward(lon * D2R, lat * D2R)),
            Self::ConicConformal => Some(conic_conformal_forward(lon * D2R, lat * D2R)),
            Self::ConicEquidistant => Some(conic_equidistant_forward(lon * D2R, lat * D2R)),
            Self::Albers => Some(albers_forward(lon, lat)),
        }
    }
}

impl Projection {
    /// The longitude an x-axis planar `u` came from, WITHOUT knowing `v`.
    ///
    /// `None` for a projection whose `u` depends on the latitude as well (the
    /// conics, the azimuthals, Equal Earth). Which names answer is enumerated by
    /// `separability_is_the_claim_the_inverses_keep`; see
    /// [`Self::axes_invert_separately`] for what it costs a brush.
    #[must_use]
    pub fn invert_lon(self, u: f64) -> Option<f64> {
        match self {
            Self::Equirectangular | Self::Identity | Self::ReflectY => Some(u),
            // The forward is `lon * D2R`.
            Self::Mercator => Some(u / D2R),
            _ => None,
        }
    }

    /// The latitude a y-axis planar `v` came from, WITHOUT knowing `u`.
    ///
    /// `None` for a projection whose `v` depends on the longitude as well.
    #[must_use]
    pub fn invert_lat(self, v: f64) -> Option<f64> {
        match self {
            Self::Equirectangular | Self::Identity => Some(v),
            Self::ReflectY => Some(-v),
            // The forward is `log(tan((π/2 + φ)/2))`, whose inverse is the
            // Gudermannian `2·atan(e^v) - π/2`.
            Self::Mercator => Some((2.0 * v.exp().atan() - std::f64::consts::FRAC_PI_2) / D2R),
            _ => None,
        }
    }

    /// Whether both per-axis inverses exist — the render-side half of
    /// [`brightfield_spec::layout::ResolvedProjection::axes_invert_separately`],
    /// which is the claim the parser and `build_brushable_bindings` act on.
    ///
    /// The two are pinned against each other over the whole catalogue by
    /// `separability_is_the_claim_the_inverses_keep`, so neither can move
    /// alone: a projection declared separable whose inverses are missing is a
    /// brush that silently stops filtering, and one declared curved whose
    /// inverses exist is a brush needlessly refused.
    #[must_use]
    pub fn axes_invert_separately(self) -> bool {
        self.invert_lon(0.0).is_some() && self.invert_lat(0.0).is_some()
    }
}

/// d3's `mercatorRaw` y half: `log(tan((π/2 + φ) / 2))`, φ in radians.
fn mercator_y(phi: f64) -> f64 {
    ((std::f64::consts::FRAC_PI_2 + phi) / 2.0).tan().ln()
}

/// Whether `(λ, φ)` in radians is on the visible hemisphere of an azimuthal
/// projection centred at (0, 0) — d3 clips the far side away rather than drawing
/// it mirrored.
fn near_side(lam: f64, phi: f64) -> bool {
    lam.cos() * phi.cos() > HORIZON_EPS
}

/// d3's `azimuthalRaw(scale)` — the shared body of the two azimuthal arms.
/// Returns `None` at the antipode, where the scale factor diverges.
fn azimuthal(lon: f64, lat: f64, scale: impl Fn(f64) -> f64) -> Option<(f64, f64)> {
    let (lam, phi) = (lon * D2R, lat * D2R);
    let (cx, cy) = (lam.cos(), phi.cos());
    if 1.0 + cx * cy <= HORIZON_EPS {
        return None;
    }
    let k = scale(cx * cy);
    k.is_finite().then(|| (k * cy * lam.sin(), k * phi.sin()))
}

/// d3's `equalEarthRaw` — the Šavrič, Patterson & Jenny (2018) polynomial, λ/φ
/// in radians.
///
/// `A3` is `0.000893`, the coefficient the paper publishes and d3-geo carries.
/// Noted because the `d3_geo_rs` port (3.2.4) writes `0.008_93` here, which
/// moves a projected coordinate in the third decimal place; that discrepancy is
/// what `tests/projection_reference.rs` records, and it is why these formulas
/// are transcribed from d3-geo rather than taken from the port.
fn equal_earth_forward(lam: f64, phi: f64) -> (f64, f64) {
    const A1: f64 = 1.340_264;
    const A2: f64 = -0.081_106;
    const A3: f64 = 0.000_893;
    const A4: f64 = 0.003_796;
    let m = 3.0_f64.sqrt() / 2.0;
    let l = (m * phi.sin()).asin();
    let l2 = l * l;
    let l6 = l2 * l2 * l2;
    (
        lam * l.cos() / (m * (A1 + 3.0 * A2 * l2 + l6 * (7.0 * A3 + 9.0 * A4 * l2))),
        l * (A1 + A2 * l2 + l6 * (A3 + A4 * l2)),
    )
}

/// d3's `conicEqualAreaRaw(y0, y1)` at `conicProjection`'s default standard
/// parallels (0°, 60°), λ/φ in radians.
///
/// The constants d3 closes over are derived here on each call rather than
/// written down, for the reason [`albers_forward`] derives its own: a
/// transcribed constant is a claim no check reddens on, and the trigonometry
/// costs less than the mistake.
fn conic_equal_area_forward(lam: f64, phi: f64) -> (f64, f64) {
    let (y0, y1) = (0.0_f64, std::f64::consts::FRAC_PI_3);
    let sy0 = y0.sin();
    let n = (sy0 + y1.sin()) / 2.0;
    if n.abs() < 1e-12 {
        // d3 degrades to cylindrical equal area when the parallels are
        // symmetric about the equator. Unreachable at these fixed parallels,
        // and kept so the math is total rather than dividing by zero.
        let c = y0.cos();
        return (lam * c, phi.sin() / c);
    }
    let c = 1.0 + sy0 * (2.0 * n - sy0);
    let r0 = c.sqrt() / n;
    let r = (c - 2.0 * n * phi.sin()).max(0.0).sqrt() / n;
    let nx = n * lam;
    (r * nx.sin(), r0 - r * nx.cos())
}

/// d3's `conicEquidistantRaw(y0, y1)` at `conicProjection`'s default standard
/// parallels (0°, 60°), λ/φ in radians.
fn conic_equidistant_forward(lam: f64, phi: f64) -> (f64, f64) {
    let (y0, y1) = (0.0_f64, std::f64::consts::FRAC_PI_3);
    let cy0 = y0.cos();
    let n = (cy0 - y1.cos()) / (y1 - y0);
    if n.abs() < 1e-12 {
        return (lam, phi); // d3 degrades to equirectangular.
    }
    let g = cy0 / n + y0;
    let gy = g - phi;
    let nx = n * lam;
    (gy * nx.sin(), g - gy * nx.cos())
}

/// d3's `conicConformalRaw(y0, y1)` at `geoConicConformal`'s default standard
/// parallels (30°, 30°), λ/φ in radians.
fn conic_conformal_forward(lam: f64, phi: f64) -> (f64, f64) {
    let y0 = 30.0 * D2R;
    // The parallels are equal, so d3 takes `n = sin(y0)` rather than the log
    // ratio its unequal-parallel branch computes.
    let n = y0.sin();
    let f = y0.cos() * tany(y0).powf(n) / n;
    // d3 clamps the latitude off one pole rather than dividing by zero, picking
    // which by the sign of `f`. `f` is positive at these parallels, so it is the
    // south pole; the branch is written out because the sign is a property of
    // the parallels and not of this projection.
    let phi = if f > 0.0 {
        phi.max(-std::f64::consts::FRAC_PI_2 + 1e-6)
    } else {
        phi.min(std::f64::consts::FRAC_PI_2 - 1e-6)
    };
    let r = f / tany(phi).powf(n);
    let nx = n * lam;
    (r * nx.sin(), f - r * nx.cos())
}

/// d3's `tany`: `tan((π/2 + y) / 2)`.
fn tany(y: f64) -> f64 {
    ((std::f64::consts::FRAC_PI_2 + y) / 2.0).tan()
}

/// US Albers equal-area conic forward. Standard parallels φ1=29.5°, φ2=45.5°,
/// reference longitude λ0=−96°, reference latitude φ0=23°. Returns `(x, y)` with
/// `y` increasing north (see [`Projection`]). Unit sphere — the renderer's
/// aspect-fit rescales, so the absolute radius is immaterial, and so is the
/// choice of φ0, which shifts `y` by a constant and does no more than that.
fn albers_forward(lon: f64, lat: f64) -> (f64, f64) {
    let d2r = std::f64::consts::PI / 180.0;
    let (phi1, phi2) = (29.5 * d2r, 45.5 * d2r);
    let (lon0, lat0) = (-96.0 * d2r, 23.0 * d2r);
    let lam = lon * d2r;
    let phi = lat * d2r;
    let n = (phi1.sin() + phi2.sin()) / 2.0;
    // Degenerate n≈0 (parallels symmetric about the equator) — not our fixed US
    // parallels, but keep the math total rather than dividing by zero.
    if n.abs() < 1e-12 {
        return (lam - lon0, phi - lat0);
    }
    let c = phi1.cos().powi(2) + 2.0 * n * phi1.sin();
    let rho = (c - 2.0 * n * phi.sin()).max(0.0).sqrt() / n;
    let rho0 = (c - 2.0 * n * lat0.sin()).max(0.0).sqrt() / n;
    let theta = n * (lam - lon0);
    (rho * theta.sin(), rho0 - rho * theta.cos())
}

// ---------------------------------------------------------------------------
// Graticule — the meridians and parallels a projected mark draws behind itself
// ---------------------------------------------------------------------------

/// The geographic rectangle a projected mark is showing, in DEGREES.
///
/// This is the graticule's whole input besides the projection: which lines exist
/// and how finely they are spaced is decided from the span, and where they land
/// is decided by the projection. Constructed by
/// [`GeoRenderer`]/[`DotRenderer`] from the coordinates they are about to draw,
/// so a mark showing one country gets that country's graticule and not the
/// world's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoExtent {
    /// West edge, degrees.
    pub lon_min: f64,
    /// East edge, degrees.
    pub lon_max: f64,
    /// South edge, degrees.
    pub lat_min: f64,
    /// North edge, degrees.
    pub lat_max: f64,
}

impl GeoExtent {
    /// Order the bounds and clamp them to the sphere. A degenerate span (one
    /// coordinate, or a column of constants) is widened to
    /// [`MIN_EXTENT_DEGREES`] so the step ladder has something to divide.
    #[must_use]
    pub fn new(lon_a: f64, lon_b: f64, lat_a: f64, lat_b: f64) -> Self {
        let (lon_min, lon_max) = widen(lon_a.min(lon_b), lon_a.max(lon_b), -180.0, 180.0);
        let (lat_min, lat_max) = widen(lat_a.min(lat_b), lat_a.max(lat_b), -90.0, 90.0);
        Self {
            lon_min,
            lon_max,
            lat_min,
            lat_max,
        }
    }

    /// East–west span in degrees.
    #[must_use]
    pub fn lon_span(&self) -> f64 {
        self.lon_max - self.lon_min
    }

    /// North–south span in degrees.
    #[must_use]
    pub fn lat_span(&self) -> f64 {
        self.lat_max - self.lat_min
    }
}

/// The narrowest span [`GeoExtent`] will represent. A single coordinate has no
/// span to space a graticule across, and dividing by it produces either one line
/// or an unbounded number of them.
pub const MIN_EXTENT_DEGREES: f64 = 1e-4;

fn widen(lo: f64, hi: f64, floor: f64, ceil: f64) -> (f64, f64) {
    let (lo, hi) = (lo.clamp(floor, ceil), hi.clamp(floor, ceil));
    if hi - lo >= MIN_EXTENT_DEGREES {
        return (lo, hi);
    }
    let mid = (lo + hi) / 2.0;
    let half = MIN_EXTENT_DEGREES / 2.0;
    ((mid - half).max(floor), (mid + half).min(ceil))
}

/// Whether a graticule line holds a constant longitude or a constant latitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraticuleKind {
    /// Constant longitude, running north–south.
    Meridian,
    /// Constant latitude, running east–west.
    Parallel,
}

/// One graticule line: the degree it stands at, and its projected polyline.
///
/// A line the projection cannot represent along its whole length comes back as
/// SEVERAL entries at the same `degrees` — one per contiguous run — rather than
/// as one polyline with a chord jumping the gap, which is what joining the runs
/// would draw.
#[derive(Debug, Clone, PartialEq)]
pub struct GraticuleLine {
    /// Meridian or parallel.
    pub kind: GraticuleKind,
    /// The longitude (meridian) or latitude (parallel) it stands at, in degrees.
    pub degrees: f64,
    /// The projected `(u, v)` polyline, in the same planar units
    /// [`Projection::project`] returns, at least two points long.
    pub points: Vec<(f64, f64)>,
}

/// The spacings a graticule may take, coarsest first — nine whole-degree rungs
/// from 90° to 1°, then six fractional ones down to 0.01° (about a kilometre),
/// so a map of a city and a map of the world both land on a number a reader
/// recognises rather than on a computed spacing like 7.3°.
const GRATICULE_STEPS: [f64; 15] = [
    90.0, 45.0, 30.0, 20.0, 15.0, 10.0, 5.0, 2.0, 1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01,
];

/// How many intervals a span must be divided into before a step is fine enough.
/// Six intervals is seven lines on the axis that is fully spanned, which is
/// dense enough to read a position off and sparse enough not to hatch the plot.
const GRATICULE_MIN_INTERVALS: f64 = 6.0;

/// How many points each graticule line is sampled at before projection. A
/// meridian under a conic or azimuthal projection is a curve, and this is what
/// decides how smoothly it draws; a cylindrical projection would be exact at two.
const GRATICULE_SAMPLES: usize = 33;

/// The coarsest spacing that divides `span` into at least six intervals, or the
/// finest the ladder offers when no coarser one manages it.
///
/// The ladder runs 90° down to 0.01° (about a kilometre) — nine whole-degree
/// rungs and six fractional ones (0.5°, 0.2°, 0.1°, 0.05°, 0.02°, 0.01°) — so a
/// map of a city and a map of the world both land on a number a reader
/// recognises rather than on a computed spacing like 7.3°. Six intervals is
/// seven lines on a fully spanned axis — dense enough to read a position off,
/// sparse enough not to hatch the plot.
#[must_use]
pub fn graticule_step(span: f64) -> f64 {
    let span = span.abs();
    for step in GRATICULE_STEPS {
        if span / step >= GRATICULE_MIN_INTERVALS {
            return step;
        }
    }
    GRATICULE_STEPS[GRATICULE_STEPS.len() - 1]
}

/// The meridians and parallels visible in `extent`, projected.
///
/// Both halves of the answer come from the two arguments: the EXTENT decides
/// which whole-degree lines exist and how far apart they are
/// ([`graticule_step`]), and the PROJECTION decides where each sampled point
/// lands. There is no data dependency and no network — this is the reason a
/// graticule is what a projected mark draws behind itself rather than a basemap;
/// the tests `the_graticule_lines_are_the_whole_degrees_the_extent_contains` and
/// `narrowing_the_extent_changes_the_graticule_rather_than_redrawing_it` hold
/// the extent's half of it.
///
/// Meridians come first, then parallels, each in ascending degree order, so two
/// runs over the same extent produce the same list in the same order.
#[must_use]
pub fn graticule(projection: Projection, extent: GeoExtent) -> Vec<GraticuleLine> {
    let mut out = Vec::new();
    let lat_samples = samples(extent.lat_min, extent.lat_max);
    let lon_samples = samples(extent.lon_min, extent.lon_max);
    for lon in ticks(
        extent.lon_min,
        extent.lon_max,
        graticule_step(extent.lon_span()),
    ) {
        push_runs(
            &mut out,
            GraticuleKind::Meridian,
            lon,
            projection,
            lat_samples.iter().map(|lat| (lon, *lat)),
        );
    }
    for lat in ticks(
        extent.lat_min,
        extent.lat_max,
        graticule_step(extent.lat_span()),
    ) {
        push_runs(
            &mut out,
            GraticuleKind::Parallel,
            lat,
            projection,
            lon_samples.iter().map(|lon| (*lon, lat)),
        );
    }
    out
}

/// The multiples of `step` inside `[lo, hi]`, ascending. Snapped to the step so
/// the lines a reader sees are whole numbers of degrees.
fn ticks(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let first = (lo / step).ceil();
    let last = (hi / step).floor();
    let mut out = Vec::new();
    let mut i = first;
    while i <= last {
        // Multiplied back rather than accumulated, so the hundredth tick is not
        // the hundredth rounding error.
        out.push(i * step);
        i += 1.0;
    }
    out
}

/// [`GRATICULE_SAMPLES`] evenly spaced values across `[lo, hi]`, endpoints
/// included.
fn samples(lo: f64, hi: f64) -> Vec<f64> {
    let n = GRATICULE_SAMPLES;
    #[allow(clippy::cast_precision_loss)]
    let last = (n - 1) as f64;
    (0..n)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f64 / last;
            lo + (hi - lo) * t
        })
        .collect()
}

/// Project each coordinate and append each contiguous run of at least two
/// representable points as its own [`GraticuleLine`].
fn push_runs(
    out: &mut Vec<GraticuleLine>,
    kind: GraticuleKind,
    degrees: f64,
    projection: Projection,
    coords: impl Iterator<Item = (f64, f64)>,
) {
    let mut run: Vec<(f64, f64)> = Vec::new();
    let flush = |run: &mut Vec<(f64, f64)>, out: &mut Vec<GraticuleLine>| {
        if run.len() >= 2 {
            out.push(GraticuleLine {
                kind,
                degrees,
                points: std::mem::take(run),
            });
        } else {
            run.clear();
        }
    };
    for (lon, lat) in coords {
        match projection.project(lon, lat) {
            Some(p) => run.push(p),
            None => flush(&mut run, out),
        }
    }
    flush(&mut run, out);
}

/// Graticule hairline width in pixels — thinner than a gridline's mark, because
/// it is scaffolding under the data and there are more of them.
const GRATICULE_STROKE_WIDTH: f64 = 0.5;

/// Stroke `lines` through the plot's two scales in the grid ink, clipped to the
/// pixel rect those scales map onto.
///
/// The clip is what keeps a graticule inside its plot: the lines are generated
/// over a geographic rectangle, and the projection of that rectangle's corners
/// can fall outside the projection of the DATA the scales were fitted to — a
/// conic bulges its corners outward. Clipping here rather than in [`graticule`]
/// keeps that function about geography and this one about drawing.
fn stroke_graticule(
    scene: &mut Scene,
    lines: &[GraticuleLine],
    x_scale: &Scale,
    y_scale: &Scale,
    ink: Color,
) {
    let Some(rect) = scale_rect(x_scale, y_scale) else {
        return;
    };
    let stroke = kurbo::Stroke::new(GRATICULE_STROKE_WIDTH);
    for line in lines {
        let pixels: Vec<(f64, f64)> = line
            .points
            .iter()
            .map(|(u, v)| (x_scale.map_f64(*u), y_scale.map_f64(*v)))
            .collect();
        for run in clip_polyline(&pixels, rect) {
            let mut path = BezPath::new();
            let mut pts = run.into_iter();
            if let Some((x0, y0)) = pts.next() {
                path.move_to((x0, y0));
                for (x, y) in pts {
                    path.line_to((x, y));
                }
            }
            scene.stroke(&stroke, Affine::IDENTITY, ink, None, &path);
        }
    }
}

/// The pixel rectangle two linear scales map onto, as `(x0, y0, x1, y1)` with
/// `x0 <= x1` and `y0 <= y1`. `None` for a non-linear axis, which is not a shape
/// a projected mark takes.
fn scale_rect(x_scale: &Scale, y_scale: &Scale) -> Option<(f64, f64, f64, f64)> {
    let (
        Scale::Linear {
            range_start: xs,
            range_end: xe,
            ..
        },
        Scale::Linear {
            range_start: ys,
            range_end: ye,
            ..
        },
    ) = (x_scale, y_scale)
    else {
        return None;
    };
    Some((xs.min(*xe), ys.min(*ye), xs.max(*xe), ys.max(*ye)))
}

/// Split a pixel polyline into the runs that lie inside `rect`, interpolating a
/// crossing segment to the boundary. Liang–Barsky per segment.
fn clip_polyline(points: &[(f64, f64)], rect: (f64, f64, f64, f64)) -> Vec<Vec<(f64, f64)>> {
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut run: Vec<(f64, f64)> = Vec::new();
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let Some((ca, cb)) = clip_segment(a, b, rect) else {
            if run.len() >= 2 {
                runs.push(std::mem::take(&mut run));
            } else {
                run.clear();
            }
            continue;
        };
        // A segment that re-enters the rect starts a new run; one that continues
        // the last extends it.
        match run.last() {
            Some(last) if same_point(*last, ca) => run.push(cb),
            _ => {
                if run.len() >= 2 {
                    runs.push(std::mem::take(&mut run));
                } else {
                    run.clear();
                }
                run.push(ca);
                run.push(cb);
            }
        }
    }
    if run.len() >= 2 {
        runs.push(run);
    }
    runs
}

fn same_point(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
}

/// Liang–Barsky: the portion of segment `a`→`b` inside `rect`, or `None`.
fn clip_segment(
    a: (f64, f64),
    b: (f64, f64),
    rect: (f64, f64, f64, f64),
) -> Option<((f64, f64), (f64, f64))> {
    let (x0, y0, x1, y1) = rect;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-dx, a.0 - x0),
        (dx, x1 - a.0),
        (-dy, a.1 - y0),
        (dy, y1 - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None; // parallel to this edge and outside it
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            if r > t1 {
                return None;
            }
            t0 = t0.max(r);
        } else {
            if r < t0 {
                return None;
            }
            t1 = t1.min(r);
        }
    }
    if t0 > t1 {
        return None;
    }
    Some((
        (a.0 + t0 * dx, a.1 + t0 * dy),
        (a.0 + t1 * dx, a.1 + t1 * dy),
    ))
}

/// Basemap outline width for a stroke-only (no-fill) geo mark. The COLOUR is
/// [`ChartInk::geo_stroke`](crate::ink::ChartInk::geo_stroke), read off the
/// `ScaleSet`, so a basemap drawn in dark is drawn in dark ink.
const GEO_STROKE_WIDTH: f64 = 0.75;

/// Renders the geo mark: projected GeoJSON Polygon/MultiPolygon features as a
/// filled choropleth or stroked basemap (the last mark of the family).
///
/// The geometry column is always the canonical [`DEFAULT_GEOMETRY_COLUMN`]
/// (`geom`) — the `GeoLowerer` canonicalises the author's `geometry:` column to
/// it (like the reserved `__bf_count` idiom), so the renderer reads one fixed
/// name and never depends on the source column's name. It holds GeoJSON text
/// (`ST_AsGeoJSON` for a spatial source; inline `VARCHAR` otherwise). Each
/// vertex is projected client-side ([`Projection`]); [`Self::augment_scales`]
/// bboxes the projected coordinates and aspect-fits them (equal px-per-unit,
/// centred) into two synthesized [`Scale::Linear`]; [`Self::render`] maps each
/// ring vertex through those scales and draws one [`BezPath`] per feature
/// (`Fill::NonZero` so RFC-7946-wound holes subtract). A `fill:` channel fills
/// each feature — a numeric fill through the sequential ramp (choropleth), else
/// the categorical colour path; a mark with NO fill strokes a basemap outline.
///
/// v1 draws Polygon / MultiPolygon only (LineString / Point deferred). Geo is a
/// STATIC, sole-in-plot mark (the `highlight` arg is ignored — no cross-filter
/// dimming).
///
/// **The projection is not a field here.** It reaches this renderer the way it
/// reaches a projected `dot` — on the [`ChannelMap`], put there by
/// `ChannelMap::from_mark_in` from the owning plot's `projectionType`. A geo
/// mark on a plot naming nothing draws the plate carrée, which
/// `MarkProjection::of` supplies. That a geo mark and a dot mark on one plot
/// draw in the same coordinate system is held by
/// `the_land_is_drawn_where_the_orthographic_puts_it` and
/// `an_earthquake_lands_where_the_orthographic_puts_it` (brightfield-ui's
/// `vendored_globe_one_coordinate_system`), over the vendored globe spec.
#[derive(Debug, Clone, Copy, Default)]
pub struct GeoRenderer {
    /// Sequential scheme for a numeric `fill:` choropleth (default viridis).
    pub scheme: SequentialScheme,
}

impl GeoRenderer {
    /// The projected bounding box `(u_min, u_max, v_min, v_max)` over every
    /// vertex of every feature, or `None` when the geometry column is absent /
    /// empty / unparseable (the caller then synthesizes no scales).
    fn projected_bbox(
        &self,
        batch: &RecordBatch,
        projection: Projection,
    ) -> Option<(f64, f64, f64, f64)> {
        let geoms = column_as_string(batch, DEFAULT_GEOMETRY_COLUMN)?;
        let (mut umin, mut umax) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY);
        for geom in geoms.iter().flatten() {
            for ring in parse_geojson_rings(geom) {
                for (lon, lat) in ring {
                    let Some((u, v)) = projection.project(lon, lat) else {
                        continue;
                    };
                    umin = umin.min(u);
                    umax = umax.max(u);
                    vmin = vmin.min(v);
                    vmax = vmax.max(v);
                }
            }
        }
        (umin.is_finite() && umax.is_finite() && vmin.is_finite() && vmax.is_finite())
            .then_some((umin, umax, vmin, vmax))
    }
}

impl MarkRenderer for GeoRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        _highlight: Option<&HighlightState>,
    ) {
        let (Some(x_scale), Some(y_scale)) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            return;
        };
        let Some(geoms) = column_as_string(batch, DEFAULT_GEOMETRY_COLUMN) else {
            return;
        };
        let projection = channel_map.projection().unwrap_or_default();

        // A `fill:` channel fills each feature (numeric → the ramp built by
        // augment_scales; else the categorical colour path). No fill → a
        // stroke-only basemap outline.
        let has_fill = channel_map.get(Channel::Fill).is_some();
        let fill_vals = channel_map
            .get(Channel::Fill)
            .and_then(|c| column_as_f64(batch, c));
        let fill_ramp = match scales.get(Channel::Fill) {
            Some(scale @ Scale::Sequential { .. }) => Some(scale),
            _ => None,
        };
        let stroke = kurbo::Stroke::new(GEO_STROKE_WIDTH);

        for (row, geom) in geoms.iter().enumerate() {
            let Some(text) = geom.as_deref() else {
                continue;
            };
            let rings = parse_geojson_rings(text);
            if rings.is_empty() {
                continue;
            }
            let mut path = BezPath::new();
            for ring in &rings {
                // A ring is drawn whole or not at all. A projection with a
                // horizon (orthographic, gnomonic) has no position for a vertex
                // on the far side, and joining the vertices that remain draws a
                // chord across the globe rather than the shape that was asked
                // for. Proper clipping — cutting the ring at the horizon and
                // closing it along the rim — is d3's `clipCircle`, and this
                // build has no equivalent, so it declines rather than
                // approximates. The three names this build recognised before the
                // catalogue widened are total, so a spec naming one of those
                // loses no ring; a spec naming `orthographic` was getting a
                // plate carrée and now gets a globe, which is a changed picture
                // and is the point of the change.
                let Some(pixels) = ring
                    .iter()
                    .map(|(lon, lat)| {
                        projection
                            .project(*lon, *lat)
                            .map(|(u, v)| (x_scale.map_f64(u), y_scale.map_f64(v)))
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let mut pts = pixels.into_iter();
                if let Some((x0, y0)) = pts.next() {
                    path.move_to((x0, y0));
                    for (x, y) in pts {
                        path.line_to((x, y));
                    }
                    path.close_path();
                }
            }
            if has_fill {
                let colour = match (&fill_ramp, fill_vals.as_ref().and_then(|v| v[row])) {
                    (Some(ramp), Some(value)) => Color::new(ramp.map_continuous(value)),
                    // A choropleth feature whose metric is NULL renders NULL
                    // ink — a warm gray no ramp value produces — instead of
                    // impersonating a high value (the NULL-reads-as-high bug).
                    (Some(_), None) if fill_vals.is_some() => scales.ink().null,
                    _ => resolve_colour(scales, channel_map, batch, row),
                };
                scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &path);
            } else {
                scene.stroke(
                    &stroke,
                    Affine::IDENTITY,
                    scales.ink().geo_stroke,
                    None,
                    &path,
                );
            }
        }
    }

    /// Aspect-fit the projected geometry into two synthesized [`Scale::Linear`]
    /// (shared px-per-unit `k`, centred), and build the numeric-fill → colour
    /// ramp for a choropleth. There is no inferable positional column (the geom
    /// column is a string), so `merge_linear_scale` CREATES the x/y scales here.
    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        if let Some(bbox) = self.projected_bbox(batch, channel_map.projection().unwrap_or_default())
        {
            let ((x0, x1), (y0, y1)) = aspect_fit_domains(bbox, x_range, y_range);
            merge_linear_scale(scales, Channel::X, x0, x1, x_range);
            merge_linear_scale(scales, Channel::Y, y0, y1, y_range);
        }
        build_geo_fill_ramp(scales, batch, channel_map, self.scheme);
    }

    /// Geo draws no cartesian frame — it projects its own coordinate space and
    /// reads as a map. The scene builders skip grid + axes for it.
    fn suppresses_frame(&self) -> bool {
        true
    }
}

/// Aspect-fit a projected bbox `(u0, u1, v0, v1)` into the plot pixel ranges,
/// returning the centred `((x_dom_min, x_dom_max), (y_dom_min, y_dom_max))` two
/// [`Scale::Linear`] domains that, mapped through `x_range` / `y_range`, give an
/// EQUAL px-per-unit `k` on both axes (so the map is aspect-correct) and centre
/// the data in the plot rect. `y_range` is `(bottom, top)` — inverted — so
/// north (larger `v`) maps up. Degenerate spans are floored to avoid div-by-0.
fn aspect_fit_domains(
    bbox: (f64, f64, f64, f64),
    x_range: (f64, f64),
    y_range: (f64, f64),
) -> ((f64, f64), (f64, f64)) {
    let (u0, u1, v0, v1) = bbox;
    let du = (u1 - u0).max(1e-9);
    let dv = (v1 - v0).max(1e-9);
    let wp = (x_range.1 - x_range.0).abs().max(1e-9);
    let hp = (y_range.0 - y_range.1).abs().max(1e-9);
    // Equal px-per-unit — the smaller of the two axis fits centres the map.
    let k = (wp / du).min(hp / dv);
    let span_x = wp / k;
    let span_y = hp / k;
    let ucx = (u0 + u1) / 2.0;
    let vcx = (v0 + v1) / 2.0;
    (
        (ucx - span_x / 2.0, ucx + span_x / 2.0),
        (vcx - span_y / 2.0, vcx + span_y / 2.0),
    )
}

/// Build the numeric-fill → colour ramp under [`Channel::Fill`] for a geo
/// choropleth (`fill: <numeric col>`), mirroring [`CellRenderer::augment_scales`]:
/// a Utf8/absent fill reads as `None` and keeps the categorical colour path; a
/// numeric fill REPLACES the inferred Linear with a [`Scale::Sequential`]
/// anchored `[0, max]` when `min >= 0`, else `[min, max]`.
fn build_geo_fill_ramp(
    scales: &mut ScaleSet,
    batch: &RecordBatch,
    channel_map: &ChannelMap,
    scheme: SequentialScheme,
) {
    let Some(fill_col) = channel_map.get(Channel::Fill) else {
        return;
    };
    let Some(vals) = column_as_f64(batch, fill_col) else {
        return; // Utf8 fill → categorical colour path, untouched.
    };
    let lo = vals.iter().flatten().cloned().fold(f64::INFINITY, f64::min);
    let hi = vals
        .iter()
        .flatten()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    if !(lo.is_finite() && hi.is_finite()) {
        return;
    }
    let (d0, d1) = if lo >= 0.0 { (0.0, hi) } else { (lo, hi) };
    scales.insert(
        Channel::Fill,
        Scale::Sequential {
            domain_min: d0,
            domain_max: d1,
            stops: scheme.stops(),
        },
    );
}

/// Parse one GeoJSON geometry string into a list of rings, each a `Vec` of
/// `(lon, lat)`. A `Polygon` yields its rings; a `MultiPolygon` yields every
/// sub-polygon's rings flattened (all drawn into one feature [`BezPath`]); a
/// `Feature` is unwrapped to its geometry. Non-Polygon geometries
/// (Point/LineString) and malformed JSON yield an empty list — the v1 contract
/// is Polygon / MultiPolygon only.
fn parse_geojson_rings(text: &str) -> Vec<Vec<(f64, f64)>> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut geom = &value;
    if geom.get("type").and_then(|t| t.as_str()) == Some("Feature") {
        match geom.get("geometry") {
            Some(g) => geom = g,
            None => return Vec::new(),
        }
    }
    let ty = geom.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let Some(coords) = geom.get("coordinates") else {
        return Vec::new();
    };
    let mut rings = Vec::new();
    match ty {
        "Polygon" => collect_polygon_rings(coords, &mut rings),
        "MultiPolygon" => {
            if let Some(polys) = coords.as_array() {
                for poly in polys {
                    collect_polygon_rings(poly, &mut rings);
                }
            }
        }
        _ => {}
    }
    rings
}

/// Append each ring of a GeoJSON Polygon `coordinates` (an array of rings, each
/// an array of `[lon, lat]` positions) to `out`. Rings with fewer than 2 points
/// are dropped.
fn collect_polygon_rings(coords: &serde_json::Value, out: &mut Vec<Vec<(f64, f64)>>) {
    let Some(ring_arr) = coords.as_array() else {
        return;
    };
    for ring in ring_arr {
        let Some(points) = ring.as_array() else {
            continue;
        };
        let mut r = Vec::with_capacity(points.len());
        for p in points {
            if let Some(pair) = p.as_array() {
                if pair.len() >= 2 {
                    if let (Some(lon), Some(lat)) = (pair[0].as_f64(), pair[1].as_f64()) {
                        r.push((lon, lat));
                    }
                }
            }
        }
        if r.len() >= 2 {
            out.push(r);
        }
    }
}

// ---------------------------------------------------------------------------
// Renderer registry
// ---------------------------------------------------------------------------

/// Build the default renderer registry mapping mark kinds to renderers.
///
/// This replaces the silent `_ => DotRenderer` fallback the retired gpui
/// shell once carried. Unknown / unimplemented mark kinds return
/// `None` from `find_renderer` so the caller can decide what to do
/// (typically: skip the mark and log a tracing event).
///
/// TODO(card-runtime-reactivity): downstream registry will own per-mark
/// lifecycle and re-render policy; for now this is a stateless lookup.
pub fn default_renderers() -> Vec<(MarkKind, Box<dyn MarkRenderer + Send + Sync>)> {
    vec![
        (MarkKind::Dot, Box::new(DotRenderer)),
        (MarkKind::DotX, Box::new(DotRenderer)),
        (MarkKind::DotY, Box::new(DotRenderer)),
        (MarkKind::Circle, Box::new(DotRenderer)),
        (MarkKind::BarX, Box::new(BarRenderer { axis: BarAxis::X })),
        (MarkKind::BarY, Box::new(BarRenderer { axis: BarAxis::Y })),
        (MarkKind::Line, Box::new(LineRenderer)),
        (MarkKind::LineX, Box::new(LineRenderer)),
        (MarkKind::LineY, Box::new(LineRenderer)),
        (
            MarkKind::AreaY,
            Box::new(AreaRenderer { axis: AreaAxis::Y }),
        ),
        (
            MarkKind::AreaX,
            Box::new(AreaRenderer { axis: AreaAxis::X }),
        ),
        (
            MarkKind::RuleX,
            Box::new(RuleRenderer { axis: RuleAxis::X }),
        ),
        (
            MarkKind::RuleY,
            Box::new(RuleRenderer { axis: RuleAxis::Y }),
        ),
        (
            MarkKind::Rect,
            Box::new(RectRenderer { kind: RectKind::Xy }),
        ),
        (
            MarkKind::RectX,
            Box::new(RectRenderer { kind: RectKind::X }),
        ),
        (
            MarkKind::RectY,
            Box::new(RectRenderer { kind: RectKind::Y }),
        ),
        (MarkKind::Text, Box::new(TextRenderer)),
        (
            MarkKind::DensityX,
            Box::new(Density1DRenderer {
                axis: DensityAxis::X,
            }),
        ),
        (
            MarkKind::DensityY,
            Box::new(Density1DRenderer {
                axis: DensityAxis::Y,
            }),
        ),
        (MarkKind::Density, Box::new(Density2DRenderer)),
        (MarkKind::Raster, Box::new(RasterRenderer::default())),
        (MarkKind::Heatmap, Box::new(HeatmapRenderer::default())),
        (MarkKind::Cell, Box::new(CellRenderer::default())),
        (MarkKind::Contour, Box::new(ContourRenderer::default())),
        (MarkKind::Hexbin, Box::new(HexbinRenderer::default())),
        (MarkKind::Hexgrid, Box::new(HexgridRenderer::default())),
        (
            MarkKind::RegressionY,
            Box::new(RegressionRenderer::default()),
        ),
        (
            MarkKind::RegressionX,
            Box::new(RegressionRenderer::default()),
        ),
        // Geo — projected GeoJSON basemap / choropleth. Its projection comes off
        // the mark's `ChannelMap`, so the registry default carries that
        // behaviour and the colour scheme is what `configured_renderer` adds.
        (MarkKind::Geo, Box::new(GeoRenderer::default())),
    ]
}

/// Build the scheme/attribute-configured renderer for a ramp-fill or contour
/// mark, or `None` for a mark that renders through the shared registry
/// ([`default_renderers`]) unchanged.
///
/// The ONE construction site both the app's first render and the cross-filter
/// coordinator's live rebuild dispatch through (renderer seam):
/// raster/heatmap/cell/hexbin carry the plot's `colorScheme`, heatmap/contour
/// carry the mark's `bandwidth`, contour carries its iso-level `thresholds`,
/// hexgrid carries its `binWidth`, and geo carries the `colorScheme` for a
/// choropleth ramp. A mark rebuilt through the same configured renderer its
/// first render used keeps its scheme, bandwidth, thresholds and binWidth across
/// every gesture. The match must stay identical to the app-assembly resolution
/// that feeds it.
///
/// **A projection is deliberately not among these.** It rides on the
/// [`ChannelMap`], which a rebuild path constructs from the mark and its plot —
/// held for the count-changing rebuild by
/// `findings124_geo_projection_survives_count_changing_rebuild` — so it cannot
/// be dropped by a rebuild that forgets to thread it, which is what a colour
/// cycle over a geo choropleth used to do, reverting an Albers
/// basemap to the plate carrée until restart.
pub fn configured_renderer(
    kind: MarkKind,
    scheme: SequentialScheme,
    bandwidth: Option<f64>,
    thresholds: Option<usize>,
    bin_width: Option<f64>,
) -> Option<Box<dyn MarkRenderer + Send + Sync>> {
    match kind {
        MarkKind::Raster => Some(Box::new(RasterRenderer { scheme })),
        MarkKind::Heatmap => Some(Box::new(HeatmapRenderer { scheme, bandwidth })),
        MarkKind::Cell => Some(Box::new(CellRenderer { scheme })),
        MarkKind::Hexbin => Some(Box::new(HexbinRenderer { scheme })),
        MarkKind::Hexgrid => Some(Box::new(HexgridRenderer {
            bin_width: bin_width.unwrap_or(DEFAULT_HEX_BIN_WIDTH),
        })),
        MarkKind::Contour => Some(Box::new(ContourRenderer {
            thresholds,
            bandwidth,
        })),
        MarkKind::Geo => Some(Box::new(GeoRenderer { scheme })),
        _ => None,
    }
}

/// Wrap a mark's renderer to apply a plot-level explicit
/// `colorDomain`/`colorRange` override (card: design phase 4 PR B — the
/// parsed-but-ignored consumption chore). Pure delegation, except
/// `augment_scales` re-applies [`apply_colour_override`] AFTER the inner
/// mark's augment — so the author's explicit domain/range wins over both
/// column inference and the density-family ramp builders, on the first render
/// AND on every live rebuild (the wrapper rides `MarkInput.renderer_override`
/// like `configured_renderer`'s output).
///
/// KNOWN in-session limitation (documented, matches the cycled-scheme
/// precedent): the verbs that REBUILD a renderer override from scratch — the
/// transient `c` colour-cycle and a command-log mark retype — reconstruct
/// through `configured_renderer` and drop this wrapper until restart.
pub struct ColourOverrideRenderer {
    /// The wrapped renderer (the mark's configured or registry-default one).
    pub inner: Box<dyn MarkRenderer + Send + Sync>,
    /// The plot's resolved explicit colour override.
    pub override_: ColourOverride,
}

impl MarkRenderer for ColourOverrideRenderer {
    fn render(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        highlight: Option<&HighlightState>,
    ) {
        self.inner
            .render(scene, batch, channel_map, scales, highlight);
    }

    fn render_interpolated(
        &self,
        scene: &mut Scene,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        scales: &ScaleSet,
        prev_positions: &[(f64, f64)],
        t: f64,
        highlight: Option<&HighlightState>,
    ) {
        self.inner.render_interpolated(
            scene,
            batch,
            channel_map,
            scales,
            prev_positions,
            t,
            highlight,
        );
    }

    fn zero_baseline_channel(&self) -> Option<Channel> {
        self.inner.zero_baseline_channel()
    }

    fn augment_scales(
        &self,
        scales: &mut ScaleSet,
        batch: &RecordBatch,
        channel_map: &ChannelMap,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) {
        self.inner
            .augment_scales(scales, batch, channel_map, x_range, y_range);
        apply_colour_override(scales, &self.override_);
    }

    fn suppresses_frame(&self) -> bool {
        self.inner.suppresses_frame()
    }
}

/// An OWNED registry-default renderer for `kind` (the boxed twin of
/// [`find_renderer`]) — for callers that must wrap a mark that has no
/// configured renderer, e.g. [`ColourOverrideRenderer`] around a plain dot.
#[must_use]
pub fn owned_default_renderer(kind: MarkKind) -> Option<Box<dyn MarkRenderer + Send + Sync>> {
    default_renderers()
        .into_iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, r)| r)
}

/// Look up a renderer for a mark kind.
///
/// Returns `None` for kinds with no registered renderer — caller should
/// log and skip rather than silently falling back to a default.
pub fn find_renderer(
    registry: &[(MarkKind, Box<dyn MarkRenderer + Send + Sync>)],
    kind: MarkKind,
) -> Option<&(dyn MarkRenderer + Send + Sync)> {
    registry
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, r)| r.as_ref())
}

/// Return the number of path-producing draw operations in a scene
/// (for testing).
///
/// Reads `vello_encoding::Encoding::n_paths`, which is incremented once per
/// `Scene::fill` and once per `Scene::stroke` call. So a regression mark
/// that emits one fill (the CI band) and one stroke (the fit line) reports
/// `count_scene_paths == 2`. Density2D in a 3×3 grid reports `count_scene_paths
/// >= 9` (one circle fill per cell). A renderer that early-returns and
/// produces no geometry reports `0`.
///
/// This does NOT distinguish fills from strokes — vello's encoding routes
/// both through `n_paths`. Tests that need to assert "fill exists AND
/// stroke exists" can pair this with a `path_tags` length check or split
/// the rendering into separate scenes.
pub fn count_scene_paths(scene: &Scene) -> usize {
    scene.encoding().n_paths as usize
}

/// Return the number of positioned glyphs in a scene (for testing text marks).
///
/// Glyphs are encoded into `resources.glyphs`, NOT through `n_paths`, so a text
/// mark reports `count_scene_paths == 0` but a non-zero `count_scene_glyphs`.
pub fn count_scene_glyphs(scene: &Scene) -> usize {
    scene.encoding().resources.glyphs.len()
}

/// Backward-compatible alias for the historical stub name. Despite "fills"
/// in the name, this counts any path-producing draw op (fill OR stroke).
/// Prefer [`count_scene_paths`] in new code.
#[deprecated(note = "use count_scene_paths — counts fills+strokes, not just fills")]
pub fn count_scene_fills(scene: &Scene) -> usize {
    count_scene_paths(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{Channel, ChannelMap};
    use crate::scale::{infer_scales, Scale};
    use arrow::array::{Float64Array, StringArray, TimestampMicrosecondArray};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    #[test]
    fn dot_renderer_positions_circles() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);

        // Scene should be non-empty after rendering 3 dots.
        // Vello's Scene encoding grows with each fill operation.
        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "scene should have path tags after rendering 3 dots"
        );
    }

    #[test]
    fn dot_renderer_with_colour() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("species", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
                Arc::new(StringArray::from(vec!["a", "b", "a"])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "species".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "scene should have path tags after rendering 3 coloured dots"
        );
    }

    /// **`aspectRatio: 1` widens the narrower domain until both axes share one
    /// px-per-unit** — the same equal-px-per-unit fit
    /// `augment_scales_aspect_fits_and_suppresses_frame` measures for
    /// [`GeoRenderer`], read here off [`DotRenderer`] instead.
    ///
    /// `x` spans 10 units and `y` spans 1, over a SQUARE pixel range, so a plain
    /// column-inferred scale gives x ten times y's px-per-unit. The fit widens
    /// y's domain — the narrower fit — until the two slopes match, and leaves
    /// x's domain alone, since x was already the wider fit.
    #[test]
    fn augment_scales_equal_aspect_widens_the_narrower_axis() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 10.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.set_equal_aspect(true);

        let (x_range, y_range) = ((0.0, 500.0), (500.0, 0.0));
        let mut scales = infer_scales(&batch, &cm, x_range, y_range);
        DotRenderer.augment_scales(&mut scales, &batch, &cm, x_range, y_range);

        let (
            Some(Scale::Linear {
                domain_min: x0,
                domain_max: x1,
                range_start: xr0,
                range_end: xr1,
            }),
            Some(Scale::Linear {
                domain_min: y0,
                domain_max: y1,
                range_start: yr0,
                range_end: yr1,
            }),
        ) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            panic!("equal-aspect augment_scales must keep both x/y as Linear scales");
        };
        assert_eq!((*x0, *x1), (0.0, 10.0), "x was already the wider fit");
        let slope_x = (xr1 - xr0) / (x1 - x0);
        let slope_y = (yr1 - yr0) / (y1 - y0);
        assert!(
            (slope_x.abs() - slope_y.abs()).abs() < 1e-9,
            "equal px-per-unit: |{slope_x}| vs |{slope_y}|"
        );
        assert!(
            y1 - y0 > 1.0 + 1e-9,
            "y's domain (still {} wide) was not widened past its own data span",
            y1 - y0
        );
        // Centred on the data's own mean, not shifted toward one edge.
        assert!((*y0 + *y1 - 1.0).abs() < 1e-9, "y0={y0} y1={y1}");
    }

    /// **A `dot` mark that does not ask draws exactly as before** — the
    /// no-op half of the same feature, over the same fixture.
    #[test]
    fn augment_scales_without_the_flag_leaves_scales_untouched() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 10.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        assert!(!cm.equal_aspect(), "a plain dot mark asked for nothing");

        let (x_range, y_range) = ((0.0, 500.0), (500.0, 0.0));
        let before = infer_scales(&batch, &cm, x_range, y_range);
        let mut after = infer_scales(&batch, &cm, x_range, y_range);
        DotRenderer.augment_scales(&mut after, &batch, &cm, x_range, y_range);
        assert_eq!(
            (
                before.get(Channel::X).unwrap().domain_min(),
                before.get(Channel::X).unwrap().domain_max()
            ),
            (
                after.get(Channel::X).unwrap().domain_min(),
                after.get(Channel::X).unwrap().domain_max()
            ),
            "augment_scales moved the x domain of a mark that never asked for \
             an equal-aspect frame"
        );
        assert_eq!(
            (
                before.get(Channel::Y).unwrap().domain_min(),
                before.get(Channel::Y).unwrap().domain_max()
            ),
            (
                after.get(Channel::Y).unwrap().domain_min(),
                after.get(Channel::Y).unwrap().domain_max()
            ),
            "augment_scales moved the y domain of a mark that never asked for \
             an equal-aspect frame"
        );
    }

    #[test]
    fn bar_renderer_rects() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = BarRenderer { axis: BarAxis::Y };
        renderer.render(&mut scene, &batch, &cm, &scales, None);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "scene should have path tags after rendering 2 bar rects"
        );
    }

    #[test]
    fn bar_renderer_band_width_proportional() {
        // Verify that band widths are proportional to the category count.
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string()],
            range_start: 0.0,
            range_end: 200.0,
            padding: 0.1,
        };
        let bw = scale.band_width().expect("should compute band width");
        // 2 categories in 200px: each band is 100px, with 10% padding = 90px
        assert!(
            (bw - 90.0).abs() < f64::EPSILON,
            "band width should be 90.0, got {bw}"
        );
    }

    #[test]
    fn line_renderer_connected_path() {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "ts",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampMicrosecondArray::from(vec![
                    1_000_000, 2_000_000, 3_000_000, 4_000_000,
                ])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 25.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "ts".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = LineRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);

        // Line renderer should produce stroke operations for 3 line segments (4 points).
        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "scene should have path tags after rendering 4-point line"
        );
    }

    // --- mark breadth: areaY ---

    #[test]
    fn area_renderer_fills_one_band_to_baseline() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 25.0, 15.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let area_y = AreaRenderer { axis: AreaAxis::Y };
        area_y.render(&mut scene, &batch, &cm, &scales, None);

        // The area is a single filled path (baseline → value line → baseline).
        assert_eq!(count_scene_paths(&scene), 1, "areaY emits one filled path");
        // The value axis must include zero so the baseline sits on-plot.
        assert_eq!(area_y.zero_baseline_channel(), Some(Channel::Y));

        // areaX is the mirror: it fills to the x=0 baseline and anchors x.
        let mut scene_x = Scene::new();
        let area_x = AreaRenderer { axis: AreaAxis::X };
        area_x.render(&mut scene_x, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene_x),
            1,
            "areaX emits one filled path"
        );
        assert_eq!(area_x.zero_baseline_channel(), Some(Channel::X));
    }

    #[test]
    fn area_renderer_needs_two_points() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0])),
                Arc::new(Float64Array::from(vec![10.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        AreaRenderer { axis: AreaAxis::Y }.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            0,
            "a single point can't form an area"
        );
    }

    // --- mark breadth: rule (with literal channel values) ---

    #[test]
    fn rule_renderer_literal_y_draws_one_line() {
        // x column gives the span scale; y is a constant literal (the position).
        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Float64, false)]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0]))],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert_literal(Channel::Y, 5.0);

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        assert!(
            scales.get(Channel::Y).is_some(),
            "a literal y synthesises a y-scale so the rule can position"
        );

        let mut scene = Scene::new();
        RuleRenderer { axis: RuleAxis::Y }.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            1,
            "a literal ruleY draws exactly one horizontal line"
        );
    }

    #[test]
    fn rule_renderer_column_x_draws_one_per_row() {
        // ruleX positioned by a column → one vertical line per row; y column
        // provides the perpendicular span.
        let schema = Arc::new(Schema::new(vec![
            Field::new("t", DataType::Float64, false),
            Field::new("v", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "t".to_string());
        cm.insert(Channel::Y, "v".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        let mut scene = Scene::new();
        RuleRenderer { axis: RuleAxis::X }.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            3,
            "a column-positioned ruleX draws one vertical line per row"
        );
    }

    #[test]
    fn rule_renderer_needs_perpendicular_scale() {
        // ruleY with only a y literal and no x scale: can't span, draws nothing.
        let schema = Arc::new(Schema::new(vec![Field::new("z", DataType::Float64, false)]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Float64Array::from(vec![1.0]))]).unwrap();
        let mut cm = ChannelMap::new();
        cm.insert_literal(Channel::Y, 5.0); // y position, but no x channel anywhere
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        let mut scene = Scene::new();
        RuleRenderer { axis: RuleAxis::Y }.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            0,
            "no perpendicular (x) scale → the rule can't span, so nothing draws"
        );
    }

    // --- mark breadth: text labels ---

    #[test]
    fn text_renderer_draws_a_glyph_per_character() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("label", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
                Arc::new(StringArray::from(vec!["AB", "CDE"])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Text, "label".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        TextRenderer.render(&mut scene, &batch, &cm, &scales, None);
        // "AB" + "CDE" = 5 glyphs. Glyphs encode into resources, not n_paths.
        assert_eq!(
            count_scene_glyphs(&scene),
            5,
            "one glyph per label character"
        );
    }

    // --- HighlightState ---

    #[test]
    fn highlight_state_predicate() {
        let hs = HighlightState {
            predicate: Box::new(|row| row == 1),
            otherwise: HighlightStyle {
                opacity: Some(0.15),
                ..Default::default()
            },
        };
        assert!(!(hs.predicate)(0), "row 0 should not match");
        assert!((hs.predicate)(1), "row 1 should match");
        assert!(!(hs.predicate)(2), "row 2 should not match");
        assert!((hs.otherwise.opacity.unwrap() - 0.15).abs() < f64::EPSILON);
    }

    #[test]
    fn highlight_state_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        // This won't compile if HighlightState's predicate isn't Send+Sync
        assert_send_sync::<Box<dyn Fn(usize) -> bool + Send + Sync>>();
    }

    // --- MarkRenderer with highlight ---

    #[test]
    fn dot_renderer_with_highlight() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let hs = HighlightState {
            predicate: Box::new(|row| row == 1),
            otherwise: HighlightStyle {
                opacity: Some(0.15),
                ..Default::default()
            },
        };

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, Some(&hs));

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "dot scene with highlight should have content"
        );
    }

    // --- the in-bar label ---

    /// **What a label SAYS**, for both forms and both states. This is the
    /// content pin the raster gate cites: that gate measures where the label is
    /// drawn and how much of it there is, which cannot tell `6` from `9`.
    ///
    /// The `part / whole` form is what makes the label the in-bar reading of
    /// the geometry rather than a restatement of the axis: it is the same two
    /// numbers the deemphasised whole and the solid part are drawn from.
    #[test]
    fn a_label_reads_the_whole_at_rest_and_part_of_whole_under_a_selection() {
        let count = |v, s, share| bar_label(LabelForm::Count, v, s, share);
        assert_eq!(count(1234.0, None, None).as_deref(), Some("1234"));
        assert_eq!(
            count(1234.0, Some(567.0), None).as_deref(),
            Some("567 / 1234")
        );
        // A group with nothing selected says so. Zero and "not selected at all"
        // are different answers, and only the first has a numerator to print.
        assert_eq!(count(12.0, Some(0.0), None).as_deref(), Some("0 / 12"));
        // A count is integral; a mean is not, and neither is printed as the
        // other.
        assert_eq!(count(12.5, None, None).as_deref(), Some("12.5"));

        let pct = |v, s, share| bar_label(LabelForm::Percent, v, s, share);
        assert_eq!(pct(25.0, None, Some(100.0)).as_deref(), Some("25%"));
        assert_eq!(
            pct(25.0, Some(5.0), Some(100.0)).as_deref(),
            Some("5% / 25%")
        );
        // Both percentages are of the SAME denominator — the values the mark
        // drew — so the pair reads as a part of a whole on one base rather than
        // as two unrelated fractions.
        assert_eq!(
            pct(50.0, Some(25.0), Some(200.0)).as_deref(),
            Some("12% / 25%")
        );
    }

    /// A percentage of nothing is not printed as `NaN%` or as `0%`: with no
    /// denominator there is no percentage, and the label is dropped.
    #[test]
    fn a_percentage_with_no_denominator_is_not_printed() {
        assert_eq!(bar_label(LabelForm::Percent, 1.0, None, None), None);
        assert_eq!(bar_label(LabelForm::Percent, 1.0, None, Some(0.0)), None);
        // The count form has no denominator to want, so it survives.
        assert!(bar_label(LabelForm::Count, 1.0, None, None).is_some());
    }

    /// **A label too long for its bar goes outside it, in the bar's own ink.**
    ///
    /// The fallback is what keeps a ranked chart honest: ranking is what makes
    /// the last bars short, so a label that vanished when it did not fit would
    /// leave unlabelled exactly the rows a reader most needs a number for.
    ///
    /// The extents are literals rather than read off [`BAR_LABEL_PAD`], for the
    /// reason the sub-pixel selection test gives: a test taking its expectation
    /// from the constant it checks passes at every value of that constant.
    #[test]
    fn a_label_that_does_not_fit_is_drawn_past_the_tip_instead_of_inside_it() {
        let ink = Color::new([0.0, 0.5, 0.8, 1.0]);
        // A 200px bar and a 40px label: it fits, so the label is knocked out of
        // the fill and anchored at the tip end.
        let inside = place_bar_label(40.0, 100.0, 300.0, ink);
        assert!(
            inside.at < 300.0 && inside.at > 260.0,
            "an inside label sits at the tip, inset: {}",
            inside.at
        );
        assert!(matches!(inside.anchor, TextAnchor::End));
        assert_ne!(
            inside.colour.components, ink.components,
            "a label on the fill is knocked out of it, not drawn in it"
        );

        // The same label on a 20px bar does not fit, so it goes past the tip in
        // the bar's own ink — which reads against the plot surface.
        let outside = place_bar_label(40.0, 100.0, 120.0, ink);
        assert!(
            outside.at > 120.0,
            "an outside label is past the tip: {}",
            outside.at
        );
        assert!(matches!(outside.anchor, TextAnchor::Start));
        assert_eq!(outside.colour.components, ink.components);

        // A bar growing the other way takes the same treatment mirrored — a
        // `barY` runs up the frame, where pixel y DECREASES.
        let down = place_bar_label(40.0, 300.0, 280.0, ink);
        assert!(
            down.at < 280.0,
            "the fallback follows the bar's direction: {}",
            down.at
        );
        assert!(matches!(down.anchor, TextAnchor::End));
    }

    /// **A bar under a live selection draws twice.**
    ///
    /// Two rects per row where an unselected chart draws one: the deemphasised
    /// whole, and the selected part over it. Counted through the scene's path
    /// tags, so what is asserted is that the second fill was emitted rather
    /// than that some code ran.
    #[test]
    fn a_bar_carrying_selected_counts_draws_the_part_over_the_whole() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
            Field::new(SELECTED_COUNT_COLUMN, DataType::Float64, true),
        ]));
        let rows = |selected: Vec<Option<f64>>| {
            RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(StringArray::from(vec!["a", "b"])),
                    Arc::new(Float64Array::from(vec![10.0, 20.0])),
                    Arc::new(Float64Array::from(selected)),
                ],
            )
            .unwrap()
        };
        let mut cm = ChannelMap::new();
        cm.insert(Channel::Y, "category".to_string());
        cm.insert(Channel::X, "value".to_string());

        let renderer = BarRenderer { axis: BarAxis::X };
        let tags = |batch: &RecordBatch| {
            let scales = infer_scales(batch, &cm, (40.0, 600.0), (450.0, 20.0));
            let mut scene = Scene::new();
            renderer.render(&mut scene, batch, &cm, &scales, None);
            scene.encoding().path_tags.len()
        };

        // Both groups partly selected: two wholes and two parts.
        let both = tags(&rows(vec![Some(4.0), Some(9.0)]));
        // Neither selected: two wholes and no part at all — a zero count is a
        // group the selection did not reach, and there is nothing to overdraw.
        let neither = tags(&rows(vec![Some(0.0), Some(0.0)]));
        assert!(
            both > neither,
            "a selected part is a second fill per bar: {both} path tags against \
             {neither} with nothing selected"
        );
        // And a group with no count at all is the same as one with a zero.
        let null = tags(&rows(vec![None, None]));
        assert_eq!(
            null, neither,
            "a NULL selected count draws no part, exactly as a zero does"
        );
    }

    #[test]
    fn bar_renderer_with_highlight() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let hs = HighlightState {
            predicate: Box::new(|row| row == 0),
            otherwise: HighlightStyle::default(),
        };

        let mut scene = Scene::new();
        let renderer = BarRenderer { axis: BarAxis::Y };
        renderer.render(&mut scene, &batch, &cm, &scales, Some(&hs));

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "bar scene with highlight should have content"
        );
    }

    // --- conditional encoding (highlight) ---

    /// an empty `otherwise` style deemphasises to the Mosaic default
    /// alpha × 0.2; a matching row is untouched.
    #[test]
    fn deemphasise_default_alpha() {
        let base = Color::new([0.3, 0.5, 0.7, 1.0]);
        let out = deemphasise(base, &HighlightStyle::default());
        let [r, g, b, a] = out.components;
        assert!((r - 0.3).abs() < 1e-6 && (g - 0.5).abs() < 1e-6 && (b - 0.7).abs() < 1e-6);
        assert!((a - 0.2).abs() < 1e-6, "default deemphasis is alpha × 0.2");
    }

    /// `opacity` scales the resolved alpha (splom's `opacity: 0.1`).
    #[test]
    fn deemphasise_opacity_scales_alpha() {
        let base = Color::new([0.3, 0.5, 0.7, 1.0]);
        let style = HighlightStyle {
            opacity: Some(0.1),
            ..Default::default()
        };
        let a = deemphasise(base, &style).components[3];
        assert!((a - 0.1).abs() < 1e-6);
    }

    /// `fill` replaces the RGB and `fillOpacity` sets the alpha
    /// (weather's `fill: '#ccc', fillOpacity: 0.2`).
    #[test]
    fn deemphasise_fill_and_fill_opacity() {
        let base = Color::new([0.3, 0.5, 0.7, 1.0]);
        let ccc = parse_css_hex("#ccc").unwrap();
        let style = HighlightStyle {
            fill: Some(ccc),
            fill_opacity: Some(0.2),
            ..Default::default()
        };
        let out = deemphasise(base, &style);
        let [r, g, b, a] = out.components;
        assert!(
            (r - 0.8).abs() < 1e-2 && (g - 0.8).abs() < 1e-2 && (b - 0.8).abs() < 1e-2,
            "#ccc grey"
        );
        assert!((a - 0.2).abs() < 1e-6, "fillOpacity sets alpha");
    }

    /// A selection too small to fill a pixel is still DRAWN. The floor is what
    /// separates "almost none of this bar" from "none of it", which are
    /// different answers and must not look the same.
    ///
    /// The extents below are written as literals rather than read off
    /// `MIN_SELECTED_EXTENT_PX`: a test that takes its expectation from the
    /// constant it is checking passes at every value of it, including zero.
    #[test]
    fn a_sub_pixel_selection_is_a_hairline_not_nothing() {
        // A 200px bar growing upward (pixel y decreases), one row in a thousand
        // selected: 0.2px of extent before any floor.
        let (base, tip) = (300.0, 100.0);
        let edge = selected_tip(base, tip, 0.001);
        assert!(
            edge < base,
            "the part grows the way the bar does: {edge} is not above {base}"
        );
        assert!(
            (base - edge) >= 0.25,
            "a non-empty selection is drawn at least a quarter of a pixel: {}px",
            base - edge
        );
        // A bar shorter than that gets its own extent, never more.
        let short = selected_tip(300.0, 299.9, 0.001);
        assert!(
            (299.9..=300.0).contains(&short),
            "the part never runs past the bar it is part of: {short}"
        );
        // A downward bar (a negative value) floors downward.
        let down = selected_tip(100.0, 300.0, 0.001);
        assert!(
            down - 100.0 >= 0.25,
            "the floor follows the bar's direction: {down}"
        );
        // Above the floor the fraction is honoured exactly.
        let half = selected_tip(300.0, 100.0, 0.5);
        assert!(
            (half - 200.0).abs() < 1e-9,
            "half a 200px bar is 100px: {half}"
        );
    }

    /// The fraction is a proportion of the group's own drawn value, clamped to
    /// the bar. No count, a zero count, or a group with nothing to be a
    /// fraction of yields nothing to draw.
    #[test]
    fn selected_fraction_is_a_proportion_of_the_group() {
        let counts = vec![Some(3.0), Some(0.0), None, Some(9.0)];
        assert_eq!(selected_fraction_of(Some(&counts), 0, 12.0), Some(0.25));
        assert_eq!(selected_fraction_of(Some(&counts), 1, 12.0), None);
        assert_eq!(selected_fraction_of(Some(&counts), 2, 12.0), None);
        assert_eq!(selected_fraction_of(Some(&counts), 3, 0.0), None);
        // A count exceeding the drawn total cannot make a part longer than its
        // whole.
        assert_eq!(selected_fraction_of(Some(&counts), 3, 4.0), Some(1.0));
        assert_eq!(selected_fraction_of(None, 0, 12.0), None);
        assert_eq!(selected_fraction_of(Some(&counts), 9, 12.0), None);
    }

    /// a matching row (predicate true) is returned unchanged.
    #[test]
    fn apply_highlight_matching_row_untouched() {
        let base = Color::new([0.3, 0.5, 0.7, 1.0]);
        let hs = HighlightState {
            predicate: Box::new(|row| row == 0),
            otherwise: HighlightStyle {
                opacity: Some(0.1),
                ..Default::default()
            },
        };
        // Row 0 matches → untouched; row 1 does not → dimmed.
        assert_eq!(apply_highlight(base, 0, Some(&hs)).components[3], 1.0);
        assert!((apply_highlight(base, 1, Some(&hs)).components[3] - 0.1).abs() < 1e-6);
        // No highlight → untouched.
        assert_eq!(apply_highlight(base, 1, None).components[3], 1.0);
    }

    #[test]
    fn ce_parse_css_hex_forms() {
        assert_eq!(
            parse_css_hex("#000000").unwrap().components,
            [0.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            parse_css_hex("#ffffff").unwrap().components,
            [1.0, 1.0, 1.0, 1.0]
        );
        // #ccc expands to #cccccc = 0xcc/255.
        let g = 0xCC as f32 / 255.0;
        assert_eq!(parse_css_hex("#ccc").unwrap().components, [g, g, g, 1.0]);
        assert!(parse_css_hex("none").is_none());
        assert!(parse_css_hex("#gg").is_none());
    }

    /// `build_highlight_state` reads the reserved `__bf_selected`
    /// boolean and drives a per-row dim — and the CI-covered end of the feature:
    /// a scene rendered WITH a non-trivial membership differs (some rows dimmed)
    /// from the same scene rendered without highlight.
    #[test]
    fn membership_column_drives_dim() {
        use arrow::array::BooleanArray;
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new(
                brightfield_spec::analysis::SELECTED_COLUMN,
                DataType::Boolean,
                false,
            ),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
                // Only row 0 is selected; rows 1 and 2 dim.
                Arc::new(BooleanArray::from(vec![true, false, false])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let style = HighlightStyle {
            opacity: Some(0.1),
            ..Default::default()
        };
        let hs = build_highlight_state(&batch, &style).expect("membership column present");
        assert!((hs.predicate)(0), "row 0 selected");
        assert!(!(hs.predicate)(1), "row 1 not selected");

        let draw_of = |highlight: Option<&HighlightState>| {
            let mut scene = Scene::new();
            DotRenderer.render(&mut scene, &batch, &cm, &scales, highlight);
            scene.encoding().draw_data.clone()
        };
        // The dimmed scene's paint data must differ from the undimmed one.
        assert_ne!(
            draw_of(Some(&hs)),
            draw_of(None),
            "a non-trivial __bf_selected must visibly dim the non-matching rows"
        );

        // An all-true membership (empty-selection edge) leaves every row lit — a
        // batch with no membership column yields no state at all.
        let no_col_schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let no_col = RecordBatch::try_new(
            no_col_schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0])),
                Arc::new(Float64Array::from(vec![10.0])),
            ],
        )
        .unwrap();
        assert!(
            build_highlight_state(&no_col, &style).is_none(),
            "no __bf_selected column → no highlight state (at-rest look)"
        );
    }

    // --- render_interpolated ---

    #[test]
    fn dot_render_interpolated_at_zero() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let prev_positions = vec![(100.0, 100.0), (200.0, 200.0), (300.0, 300.0)];

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render_interpolated(&mut scene, &batch, &cm, &scales, &prev_positions, 0.0, None);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "interpolated scene at t=0 should have content"
        );
    }

    #[test]
    fn dot_render_interpolated_at_one() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let prev_positions = vec![(100.0, 100.0), (200.0, 200.0), (300.0, 300.0)];

        let mut scene = Scene::new();
        let renderer = DotRenderer;
        renderer.render_interpolated(&mut scene, &batch, &cm, &scales, &prev_positions, 1.0, None);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "interpolated scene at t=1 should have content"
        );
    }

    // -----------------------------------------------------------------------
    // Statistical-mark tests
    // -----------------------------------------------------------------------

    fn density_1d_batch() -> RecordBatch {
        // 8 bins centred at 0..7; counts form a roughly Gaussian shape.
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![
                    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    1.0, 4.0, 10.0, 20.0, 20.0, 10.0, 4.0, 1.0,
                ])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn density1d_x_renders_filled_path() {
        let batch = density_1d_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        // Perpendicular density axis + padded bin axis come from augment_scales,
        // mirroring the real pipeline (the batch has no perpendicular column).
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        let renderer = Density1DRenderer {
            axis: DensityAxis::X,
        };
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        // The spec requires at least one fill (the density curve).
        // count_scene_paths reads vello's n_paths counter — incremented
        // once per fill or stroke.
        assert!(
            count_scene_paths(&scene) >= 1,
            "Density1DRenderer (X) must emit at least one filled path"
        );
    }

    #[test]
    fn density1d_y_renders_filled_path() {
        // For DensityY, y is the binned axis; x is density magnitude.
        let schema = Arc::new(Schema::new(vec![
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0])),
                Arc::new(Float64Array::from(vec![1.0, 5.0, 12.0, 5.0, 1.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::Y, "y_bin".to_string());
        // Perpendicular density axis (x) synthesised by augment_scales.
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        let renderer = Density1DRenderer {
            axis: DensityAxis::Y,
        };
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert!(
            count_scene_paths(&scene) >= 1,
            "Density1DRenderer (Y) must emit at least one filled path"
        );
    }

    #[test]
    fn density2d_renders_circle_grid() {
        // 3x3 bin grid with peak in centre.
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        let xs = vec![0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 1.0, 2.0];
        let ys = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0];
        let counts = vec![1.0, 4.0, 1.0, 4.0, 16.0, 4.0, 1.0, 4.0, 1.0];
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(xs)),
                Arc::new(Float64Array::from(ys)),
                Arc::new(Float64Array::from(counts)),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut scene = Scene::new();
        let renderer = Density2DRenderer;
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        // The spec requires one circle per non-empty cell. With a 3×3 grid
        // of all-positive counts, the renderer must produce at least 9 fills.
        // count_scene_paths gives a real count via vello's n_paths counter.
        assert!(
            count_scene_paths(&scene) >= 9,
            "Density2DRenderer on 3×3 grid must emit ≥9 path operations, got {}",
            count_scene_paths(&scene)
        );
    }

    // Density marks: the shared KDE-grid helper reproduces
    // exactly the values the inline Density2D path produced — same histogram
    // reconstruction (row order and gaps included), same Silverman bandwidths,
    // same kde_2d call — pinned by replaying the inline formula over the fixture
    // and asserting bitwise equality. (The byte-identical density example PNGs
    // are the end-to-end gate; this pins the seam headlessly.)
    #[test]
    fn kde_grid_helper_matches_inline_path() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        // 3×3 grid with a hot centre, deliberately in SCRAMBLED row order so the
        // helper's centre-sorting + position mapping is exercised, with one cell
        // (2, 0) omitted so an unoccupied bin stays zero in the histogram.
        let xs = vec![1.0, 0.0, 2.0, 0.0, 1.0, 2.0, 1.0, 0.0];
        let ys = vec![1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 2.0, 2.0];
        let counts = vec![16.0, 1.0, 1.0, 4.0, 4.0, 4.0, 4.0, 1.0];
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(xs)),
                Arc::new(Float64Array::from(ys)),
                Arc::new(Float64Array::from(counts)),
            ],
        )
        .unwrap();

        let grid = build_kde_grid(&batch, "x_bin", "y_bin", None).expect("grid builds");
        assert_eq!(grid.x_centres, vec![0.0, 1.0, 2.0], "x centres sorted");
        assert_eq!(grid.y_centres, vec![0.0, 1.0, 2.0], "y centres sorted");
        assert_eq!(grid.dx, 1.0);
        assert_eq!(grid.dy, 1.0);
        assert_eq!((grid.rows(), grid.cols()), (3, 3));

        // Replay the inline path's formula: dense row-major histogram (row = y),
        // Silverman per-axis over the expanded samples, kde_2d.
        #[rustfmt::skip]
        let bins: Vec<u32> = vec![
            1, 4, 1,  // y = 0
            4, 16, 4, // y = 1
            1, 4, 0,  // y = 2 — (2, 2) unoccupied
        ];
        let mut xs_samples: Vec<f64> = Vec::new();
        let mut ys_samples: Vec<f64> = Vec::new();
        for r in 0..3 {
            for c in 0..3 {
                for _ in 0..bins[r * 3 + c] {
                    xs_samples.push(c as f64);
                    ys_samples.push(r as f64);
                }
            }
        }
        let (h_x, h_y) = silverman_2d_per_axis(&xs_samples, &ys_samples);
        let expected = kde_2d(&bins, (3, 3), (h_x, h_y), (1.0, 1.0));
        assert_eq!(
            grid.density, expected,
            "helper grid must be bitwise-identical to the inline path's kde_2d output"
        );
        let expected_max = expected.iter().cloned().fold(0.0_f64, f64::max);
        assert_eq!(grid.max_density, expected_max);

        // An explicit bandwidth overrides Silverman on both axes.
        let with_bw = build_kde_grid(&batch, "x_bin", "y_bin", Some(0.5)).expect("grid builds");
        assert_eq!(
            with_bw.density,
            kde_2d(&bins, (3, 3), (0.5, 0.5), (1.0, 1.0)),
            "explicit bandwidth reaches kde_2d on both axes"
        );
        assert!(
            with_bw.density != grid.density,
            "the explicit bandwidth actually changes the field"
        );

        // Degenerate inputs return None exactly where the inline path returned.
        assert!(build_kde_grid(&batch, "missing", "y_bin", None).is_none());
        assert!(build_kde_grid(&batch, "x_bin", "y_bin", Some(0.0)).is_none());
    }

    // build_kde_grid materialises a DENSE first..last lattice at the
    // recovered pitch, so unoccupied INTERIOR bins are present (zero mass) and
    // kde_2d smooths over the true geometry. A contiguous grid is unchanged
    // (the byte-identity guard for the shipped heatmap/contour examples); a
    // gapped axis densifies to fill the interior.
    #[test]
    fn build_kde_grid_dense_lattice_fills_interior_gaps() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        let make = |xs: Vec<f64>, ys: Vec<f64>, cs: Vec<f64>| {
            RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(Float64Array::from(xs)) as _,
                    Arc::new(Float64Array::from(ys)) as _,
                    Arc::new(Float64Array::from(cs)) as _,
                ],
            )
            .unwrap()
        };

        // Contiguous 2×2 grid — the dense lattice equals the occupied lattice, so
        // nothing moves (this is why cell-dense examples stay byte-identical).
        let dense = make(
            vec![0.0, 1.0, 0.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            vec![1.0, 2.0, 3.0, 4.0],
        );
        let g = build_kde_grid(&dense, "x_bin", "y_bin", None).expect("grid builds");
        assert_eq!(
            g.x_centres,
            vec![0.0, 1.0],
            "contiguous x lattice unchanged"
        );
        assert_eq!(
            g.y_centres,
            vec![0.0, 1.0],
            "contiguous y lattice unchanged"
        );

        // Gapped x axis: bins occupied at 0, 1, 4 (interior 2 and 3 empty). The
        // pitch recovers to 1 and the lattice fills 0..4 — five columns, the two
        // interior gap bins materialised with zero raw mass.
        let gapped = make(
            vec![0.0, 1.0, 4.0, 0.0, 1.0, 4.0],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        );
        let g = build_kde_grid(&gapped, "x_bin", "y_bin", None).expect("grid builds");
        assert_eq!(g.dx, 1.0, "recovered pitch is the GCD of the gaps 1 and 3");
        assert_eq!(
            g.x_centres,
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            "dense first..last lattice materialises the interior gap bins"
        );
        assert_eq!(g.cols(), 5, "five columns span the gapped axis");
        assert_eq!(g.rows(), 2, "the ungapped y axis is untouched");
    }

    // bin_step recovers the true pitch as the GCD of the gaps, even when NO two
    // occupied bins are adjacent (the min-gap-only approach would over-estimate).
    #[test]
    fn raster_bin_step_recovers_pitch_from_sparse_centres() {
        // Occupied bins at 0, 2, 3 (bin at 1 empty): gaps {2, 1}, GCD 1.
        assert_eq!(bin_step(&[0.0, 2.0, 3.0]), Some(1.0));
        // No adjacent occupied pair: centres 2 & 3 buckets apart → gaps {0.9, 1.35}.
        // Min-gap alone would say 0.9 (2× too wide); the GCD recovers the true 0.45.
        let step = bin_step(&[0.0, 0.9, 2.25]).expect("recovers pitch");
        assert!(
            (step - 0.45).abs() < 1e-9,
            "GCD of {{0.9, 1.35}} is 0.45, got {step}"
        );
        assert_eq!(bin_step(&[10.0]), None, "one centre → no step");
        assert_eq!(
            sorted_unique(&[Some(2.0), None, Some(2.0), Some(1.0)]),
            vec![1.0, 2.0]
        );
    }

    // A raster draws one filled cell per occupied bin — a 3×3 grid → ≥9 fills.
    #[test]
    fn raster_renders_one_cell_per_bin() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        let xs = vec![0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 1.0, 2.0];
        let ys = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0];
        let counts = vec![1.0, 4.0, 1.0, 4.0, 16.0, 4.0, 1.0, 4.0, 1.0];
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(xs)),
                Arc::new(Float64Array::from(ys)),
                Arc::new(Float64Array::from(counts)),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        RasterRenderer::default().augment_scales(
            &mut scales,
            &batch,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );

        let mut scene = Scene::new();
        RasterRenderer::default().render(&mut scene, &batch, &cm, &scales, None);
        assert!(
            count_scene_paths(&scene) >= 9,
            "raster on a 3×3 grid must emit ≥9 filled cells, got {}",
            count_scene_paths(&scene)
        );
    }

    /// Pack a peniko colour exactly as vello encodes a solid fill into
    /// `draw_data` (premultiplied little-endian RGBA8), so a test can compare a
    /// rendered fill against an expected colour byte-for-byte.
    fn packed(colour: [f32; 4]) -> u32 {
        Color::new(colour).premultiply().to_rgba8().to_u32()
    }

    // each occupied cell is coloured through the Fill Sequential ramp
    // (count → map_continuous) at full alpha, so the colours ACTUALLY ENCODED into
    // the scene are the ramp samples — probed via draw_data, not re-derived from
    // the Scale. With no Fill scale it falls back to the legacy alpha path.
    #[test]
    fn raster_colours_cells_through_ramp() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        // Two occupied bins on a diagonal (so each axis has ≥2 distinct centres
        // for the bin-pitch recovery) with very different counts (1 vs 100).
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
                Arc::new(Float64Array::from(vec![1.0, 100.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        RasterRenderer::default().augment_scales(
            &mut scales,
            &batch,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );

        // Expected ramp samples, computed the way render does: floor the position
        // at RASTER_MIN_T in the ramp's own domain, then map_continuous.
        let ramp = scales.get(Channel::Fill).expect("fill ramp built");
        let dmax = ramp.domain_max().expect("sequential has a domain max");
        let sample = |count: f64| {
            let pos = (count / dmax).clamp(0.0, 1.0).max(RASTER_MIN_T);
            ramp.map_continuous(pos * dmax)
        };
        let expect_low = sample(1.0);
        let expect_high = sample(100.0);
        // Both ramp samples carry full alpha (byte 255) and DIFFER in RGB.
        assert_eq!(expect_low[3], 1.0, "ramp colours are full-alpha");
        assert_eq!(expect_high[3], 1.0, "ramp colours are full-alpha");
        assert!(
            expect_low != expect_high,
            "ramp maps the two counts to distinct colours"
        );

        // Probe the colours ACTUALLY encoded into the scene: exactly the two cell
        // fills, and they equal the expected ramp samples byte-for-byte.
        let mut scene = Scene::new();
        RasterRenderer::default().render(&mut scene, &batch, &cm, &scales, None);
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn,
            std::collections::HashSet::from([packed(expect_low), packed(expect_high)]),
            "the two cell fills are the ramp samples, not a constant hue or the fallback"
        );

        // Fallback: with the Fill scale removed, cells render through the legacy
        // default-blue path — same hue, count-proportional (floored) alpha.
        let mut no_fill = ScaleSet::new();
        no_fill.insert(Channel::X, scales.get(Channel::X).unwrap().clone());
        no_fill.insert(Channel::Y, scales.get(Channel::Y).unwrap().clone());
        let mut scene2 = Scene::new();
        RasterRenderer::default().render(&mut scene2, &batch, &cm, &no_fill, None);
        let [cr, cg, cb, _] = ChartInk::LIGHT.mark_default.components;
        let alpha_low = (1.0_f64 / 100.0).clamp(0.0, 1.0).max(RASTER_MIN_T) as f32;
        let fallback: std::collections::HashSet<u32> =
            scene2.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            fallback,
            std::collections::HashSet::from([
                packed([cr, cg, cb, alpha_low]),
                packed([cr, cg, cb, 1.0])
            ]),
            "fallback keeps the default-blue hue with count-proportional alpha"
        );
    }

    // -----------------------------------------------------------------------
    // HexbinRenderer scene probes + augment_scales
    // -----------------------------------------------------------------------

    /// A hexbin batch: two hexes, `(x, y)` centres, a fill column, and the
    /// constant in-band half-extents `__bf_hex_dx`/`__bf_hex_dy`.
    fn hexbin_batch(fill_col: &str, fills: Vec<f64>) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new(fill_col, DataType::Float64, false),
            Field::new(HEX_DX_COL, DataType::Float64, false),
            Field::new(HEX_DY_COL, DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 10.0])),
                Arc::new(Float64Array::from(vec![0.0, 10.0])),
                Arc::new(Float64Array::from(fills)),
                Arc::new(Float64Array::from(vec![2.0, 2.0])),
                Arc::new(Float64Array::from(vec![2.0, 2.0])),
            ],
        )
        .unwrap()
    }

    fn hexbin_cm(fill_col: &str) -> ChannelMap {
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, fill_col.to_string());
        cm
    }

    /// a COUNT fill ramps through the zero-anchored Sequential (with
    /// the RASTER_MIN_T floor), and the colours ACTUALLY encoded into the scene
    /// are those ramp samples — probed via draw_data, not re-derived. One filled
    /// hexagon per row.
    #[test]
    fn count_fills_ramp_through_sequential() {
        let batch = hexbin_batch(DENSITY_COUNT_COL, vec![1.0, 100.0]);
        let cm = hexbin_cm(DENSITY_COUNT_COL);
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        HexbinRenderer::default().augment_scales(
            &mut scales,
            &batch,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );

        let ramp = scales.get(Channel::Fill).expect("count ramp built");
        assert_eq!(
            ramp.domain_max(),
            Some(100.0),
            "count ramp zero-anchored [0,max]"
        );
        let dmax = ramp.domain_max().unwrap();
        let sample = |count: f64| {
            let pos = (count / dmax).clamp(0.0, 1.0).max(RASTER_MIN_T);
            ramp.map_continuous(pos * dmax)
        };

        let mut scene = Scene::new();
        HexbinRenderer::default().render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(count_scene_paths(&scene), 2, "one hexagon fill per row");
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn,
            std::collections::HashSet::from([packed(sample(1.0)), packed(sample(100.0))]),
            "hex fills are the zero-anchored ramp samples"
        );
    }

    /// an AVG fill follows the cell anchoring rule and maps through
    /// the ramp WITHOUT the count floor.
    #[test]
    fn avg_fills_follow_cell_anchoring() {
        let batch = hexbin_batch("v", vec![15.0, 100.0]);
        let cm = hexbin_cm("v");
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        HexbinRenderer::default().augment_scales(
            &mut scales,
            &batch,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );
        let ramp = scales.get(Channel::Fill).expect("avg ramp built");
        // min ≥ 0 ⇒ [0, max].
        assert_eq!(ramp.domain_max(), Some(100.0));
        let mut scene = Scene::new();
        HexbinRenderer::default().render(&mut scene, &batch, &cm, &scales, None);
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        // No RASTER_MIN_T floor for avg — direct map_continuous.
        assert_eq!(
            drawn,
            std::collections::HashSet::from([
                packed(ramp.map_continuous(15.0)),
                packed(ramp.map_continuous(100.0)),
            ]),
        );
    }

    /// augment_scales widens x/y by half a hex (the constant in-band
    /// half-extents) and applies the cell anchoring rule for a signed avg fill.
    #[test]
    fn augment_scales_widens_and_anchors() {
        // Signed avg fill ([-5, 10]) exercises the [min, max] branch.
        let batch = hexbin_batch("v", vec![-5.0, 10.0]);
        let cm = hexbin_cm("v");
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        HexbinRenderer::default().augment_scales(
            &mut scales,
            &batch,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );

        // x/y centres [0,10] widened by dx=dy=2 → [-2, 12].
        let x = scales.get(Channel::X).unwrap();
        assert!(
            (x.domain_min().unwrap() - (-2.0)).abs() < 1e-9,
            "x widened lo"
        );
        assert!(
            (x.domain_max().unwrap() - 12.0).abs() < 1e-9,
            "x widened hi"
        );
        let y = scales.get(Channel::Y).unwrap();
        assert!((y.domain_min().unwrap() - (-2.0)).abs() < 1e-9);
        assert!((y.domain_max().unwrap() - 12.0).abs() < 1e-9);

        // Signed avg ⇒ [min, max], not zero-anchored.
        let fill = scales.get(Channel::Fill).unwrap();
        assert_eq!(fill.domain_max(), Some(10.0));
        match fill {
            Scale::Sequential { domain_min, .. } => assert!((domain_min - (-5.0)).abs() < 1e-9),
            other => panic!("expected Sequential fill, got {other:?}"),
        }
    }

    /// augment_scales MERGES the Fill scale — a sibling's categorical
    /// Colour Fill survives untouched (merge-not-clobber, raster/cell precedent).
    #[test]
    fn augment_scales_merges_not_clobber() {
        let batch = hexbin_batch(DENSITY_COUNT_COL, vec![1.0, 100.0]);
        let cm = hexbin_cm(DENSITY_COUNT_COL);
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["a".to_string(), "b".to_string()],
                palette: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
            },
        );
        HexbinRenderer::default().augment_scales(
            &mut scales,
            &batch,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );
        assert!(
            matches!(scales.get(Channel::Fill), Some(Scale::Colour { .. })),
            "a sibling's categorical Colour fill must survive"
        );
    }

    /// the configured renderer (the cfr `renderer_override` seam)
    /// draws a rebuild byte-identically — the same override renderer is used for
    /// the first render and every live rebuild, so output is stable.
    #[test]
    fn configured_renderer_rebuild_parity() {
        let batch = hexbin_batch(DENSITY_COUNT_COL, vec![1.0, 100.0]);
        let cm = hexbin_cm(DENSITY_COUNT_COL);
        let renderer =
            configured_renderer(MarkKind::Hexbin, SequentialScheme::Turbo, None, None, None)
                .expect("hexbin has a configured renderer");
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let mut a = Scene::new();
        renderer.render(&mut a, &batch, &cm, &scales, None);
        let mut b = Scene::new();
        renderer.render(&mut b, &batch, &cm, &scales, None);
        let da: Vec<u32> = a.encoding().draw_data.to_vec();
        let db: Vec<u32> = b.encoding().draw_data.to_vec();
        assert_eq!(
            da, db,
            "the override renderer draws the rebuild identically"
        );
        // Turbo scheme actually rides through (distinct from the viridis default).
        let mut viridis_scene = Scene::new();
        let mut vscales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        HexbinRenderer::default().augment_scales(
            &mut vscales,
            &batch,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );
        HexbinRenderer::default().render(&mut viridis_scene, &batch, &cm, &vscales, None);
        let dv: Vec<u32> = viridis_scene.encoding().draw_data.to_vec();
        assert_ne!(
            da, dv,
            "the configured scheme (turbo) differs from the viridis default"
        );
    }

    // -----------------------------------------------------------------------
    // geo — GeoRenderer + Projection (last mark)
    // -----------------------------------------------------------------------

    /// A one-square-polygon batch, optionally with a numeric `rate` fill column.
    fn geo_batch(geoms: Vec<&str>, fill: Option<Vec<f64>>) -> RecordBatch {
        let mut fields = vec![Field::new("geom", DataType::Utf8, true)];
        let mut cols: Vec<Arc<dyn Array>> = vec![Arc::new(StringArray::from(geoms))];
        if let Some(f) = fill {
            fields.push(Field::new("rate", DataType::Float64, true));
            cols.push(Arc::new(Float64Array::from(f)));
        }
        RecordBatch::try_new(Arc::new(Schema::new(fields)), cols).unwrap()
    }

    const SQUARE: &str =
        r#"{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]]]}"#;

    #[test]
    fn projection_equirect_identity_and_albers_reference() {
        // Equirectangular is the identity (u=lon, v=lat).
        assert_eq!(
            Projection::Equirectangular.project(12.0, -34.0),
            Some((12.0, -34.0))
        );

        // Albers reference point (−96°, 23°) projects to ≈ (0, 0): λ=λ0 → θ=0 → x=0;
        // φ=φ0 → ρ=ρ0 → y=0. (Structurally insensitive to the parallels on its
        // own — the non-reference point below pins the conic constants.)
        let (rx, ry) = Projection::Albers
            .project(-96.0, 23.0)
            .expect("albers is total");
        assert!(
            rx.abs() < 1e-9 && ry.abs() < 1e-9,
            "reference point ≈ origin: ({rx}, {ry})"
        );

        // A NON-reference point pins the standard-parallel / conic math. The
        // expected value is computed INDEPENDENTLY by hand from the d3-geo
        // conicEqualArea forward (parallels 29.5°/45.5°, reference −96°/23°),
        // NOT by running `albers_forward`, so a regression in the constants (e.g.
        // to 20°/60°) fails here. For (lon −80°, lat 40°):
        //   n=0.6028370, C=1.3512237, ρ0=1.5562005, ρ=1.2591903, θ=0.1683544
        //   x=ρ·sinθ=0.211010, y=ρ0−ρ·cosθ=0.314810.
        let (ax, ay) = Projection::Albers
            .project(-80.0, 40.0)
            .expect("albers is total");
        assert!(
            (ax - 0.211010).abs() < 1e-4 && (ay - 0.314810).abs() < 1e-4,
            "albers(−80,40) must match the independent conic value (0.211010, 0.314810); got ({ax}, {ay})"
        );

        // North is up in math coords: a more-northern point has a LARGER v.
        let (_, y_south) = Projection::Albers
            .project(-96.0, 30.0)
            .expect("albers is total");
        let (_, y_north) = Projection::Albers
            .project(-96.0, 45.0)
            .expect("albers is total");
        assert!(
            y_north > y_south,
            "albers v increases north: {y_north} !> {y_south}"
        );

        // ResolvedProjection → Projection conversion is faithful.
        use brightfield_spec::layout::ResolvedProjection as R;
        assert_eq!(
            Projection::from(R::Equirectangular),
            Projection::Equirectangular
        );
        assert_eq!(Projection::from(R::Albers), Projection::Albers);
    }

    #[test]
    fn augment_scales_aspect_fits_and_suppresses_frame() {
        let batch = geo_batch(vec![SQUARE], None);
        let cm = ChannelMap::new(); // basemap — no fill channel
        let renderer = GeoRenderer::default();
        let (x_range, y_range) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, x_range, y_range);
        renderer.augment_scales(&mut scales, &batch, &cm, x_range, y_range);

        // augment_scales CREATES the x/y linear scales (no inferable column).
        let (
            Some(Scale::Linear {
                domain_min: xd0,
                domain_max: xd1,
                range_start: xr0,
                range_end: xr1,
            }),
            Some(Scale::Linear {
                domain_min: yd0,
                domain_max: yd1,
                range_start: yr0,
                range_end: yr1,
            }),
        ) = (scales.get(Channel::X), scales.get(Channel::Y))
        else {
            panic!("geo augment_scales must synthesize linear x/y scales");
        };
        // Equal px-per-unit on both axes (aspect-correct): |slope_x| == |slope_y|.
        let slope_x = (xr1 - xr0) / (xd1 - xd0);
        let slope_y = (yr1 - yr0) / (yd1 - yd0);
        assert!(
            (slope_x.abs() - slope_y.abs()).abs() < 1e-6,
            "equal px-per-unit: |{slope_x}| vs |{slope_y}|"
        );
        // Centred: the data centroid (5, 5) maps to the plot-rect centre.
        let cx = scales.get(Channel::X).unwrap().map_f64(5.0);
        let cy = scales.get(Channel::Y).unwrap().map_f64(5.0);
        assert!(
            (cx - (x_range.0 + x_range.1) / 2.0).abs() < 1e-6,
            "x centred: {cx}"
        );
        assert!(
            (cy - (y_range.0 + y_range.1) / 2.0).abs() < 1e-6,
            "y centred: {cy}"
        );

        // Geo suppresses the cartesian frame.
        assert!(renderer.suppresses_frame(), "geo drops grid + axes");

        // A basemap (no fill) strokes each feature — non-empty scene.
        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert!(
            count_scene_paths(&scene) > 0,
            "basemap strokes the polygon outline"
        );
    }

    #[test]
    fn choropleth_builds_sequential_fill_ramp() {
        let batch = geo_batch(vec![SQUARE, SQUARE], Some(vec![2.0, 8.0]));
        let mut cm = ChannelMap::new();
        cm.insert(Channel::Fill, "rate".to_string());
        let renderer = GeoRenderer::default();
        let (x_range, y_range) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, x_range, y_range);
        renderer.augment_scales(&mut scales, &batch, &cm, x_range, y_range);

        // A numeric fill builds a Sequential ramp anchored [0, max] (min >= 0).
        match scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                ..
            }) => {
                assert_eq!(*domain_min, 0.0);
                assert_eq!(*domain_max, 8.0);
            }
            other => panic!("expected a Sequential fill ramp, got {other:?}"),
        }
        // Render fills the features (does not early-return).
        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert!(
            count_scene_paths(&scene) > 0,
            "choropleth fills the features"
        );
    }

    #[test]
    fn geo_multipolygon_and_malformed_geojson_are_handled() {
        // MultiPolygon parses to multiple rings; malformed / non-polygon yields
        // an empty ring set (the feature draws nothing, no panic).
        let multi = r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]],[[[2,2],[3,2],[3,3],[2,2]]]]}"#;
        assert_eq!(parse_geojson_rings(multi).len(), 2, "two sub-polygon rings");
        assert!(parse_geojson_rings("not json").is_empty());
        assert!(
            parse_geojson_rings(r#"{"type":"Point","coordinates":[0,0]}"#).is_empty(),
            "a Point is not a v1 geometry"
        );
        // A Feature is unwrapped to its geometry.
        let feature = r#"{"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]},"properties":{}}"#;
        assert_eq!(parse_geojson_rings(feature).len(), 1);
    }

    // -----------------------------------------------------------------------
    // HexgridRenderer (dataless mesh) + lattice alignment
    // -----------------------------------------------------------------------

    /// A singleton batch, as the hexgrid lowerer emits (one row, no positional
    /// columns) — the renderer draws from the plot extent, not this batch.
    fn hexgrid_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "__bf_hexgrid",
            DataType::Int64,
            false,
        )]));
        RecordBatch::try_new(
            schema,
            vec![Arc::new(arrow::array::Int64Array::from(vec![1]))],
        )
        .unwrap()
    }

    /// A ScaleSet with linear x/y scales over a known plot-area pixel rect.
    fn plot_scales(x: (f64, f64), y: (f64, f64)) -> ScaleSet {
        let mut s = ScaleSet::new();
        s.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 1.0,
                range_start: x.0,
                range_end: x.1,
            },
        );
        s.insert(
            Channel::Y,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 1.0,
                range_start: y.0,
                range_end: y.1,
            },
        );
        s
    }

    /// the mesh covers the plot rect with the right hex count for a
    /// known extent + binWidth (one stroked outline per lattice centre).
    #[test]
    fn hexgrid_mesh_covers_plot_extent() {
        let renderer = HexgridRenderer { bin_width: 20.0 };
        // Plot rect 200×150 px (x range 40..240, y range 170..20).
        let scales = plot_scales((40.0, 240.0), (170.0, 20.0));
        let expected_centres =
            HexgridRenderer::lattice_centres(40.0, 240.0, 170.0, 20.0, 20.0).len();
        assert!(expected_centres > 0, "lattice must cover the rect");

        let mut scene = Scene::new();
        renderer.render(
            &mut scene,
            &hexgrid_batch(),
            &ChannelMap::new(),
            &scales,
            None,
        );
        assert_eq!(
            count_scene_paths(&scene),
            expected_centres,
            "one stroked hex outline per lattice centre"
        );
    }

    /// Faithfully replicate the hexbin lowerer's lattice: for a plot pixel
    /// extent `W×H`, `binWidth` `b`, and raw data extent, the hex `(q, r)` centre
    /// in DATA units and the constant half-extents — the exact expressions from
    /// `build_hexbin_plan`. Returns the occupied centres (those whose data centre
    /// falls in the extent) plus `(dx_data, dy_data)`.
    fn lowerer_hex_centres(
        w: f64,
        h: f64,
        b: f64,
        xmin: f64,
        xmax: f64,
        ymin: f64,
        ymax: f64,
    ) -> (Vec<(f64, f64)>, f64, f64) {
        let sqrt3 = 1.732_050_807_568_877_2_f64;
        let size = b / sqrt3;
        let dx_data = (b / 2.0) / w * (xmax - xmin);
        let dy_data = size / h * (ymax - ymin);
        let mut out = Vec::new();
        for r in -40..=40 {
            for q in -40..=40 {
                let cx_px = size * (sqrt3 * q as f64 + (sqrt3 / 2.0) * r as f64);
                let cy_px = size * 1.5 * r as f64;
                let cx = xmin + cx_px / w * (xmax - xmin);
                let cy = ymin + cy_px / h * (ymax - ymin);
                if cx >= xmin && cx <= xmax && cy >= ymin && cy <= ymax {
                    out.push((cx, cy));
                }
            }
        }
        (out, dx_data, dy_data)
    }

    /// F1 alignment probe — the reinstated one. A sibling hexbin
    /// overlays the hexgrid mesh EXACTLY on-lattice. Map a faithful hexbin
    /// batch's centres through the REAL render pipeline (`infer_scales` +
    /// `HexbinRenderer::augment_scales`, which now widens RAW-anchored from the
    /// in-band `__bf_hex_x0/x1/y0/y1` extent), then generate the `HexgridRenderer`
    /// mesh at the same `binWidth` and assert every hexbin centre coincides with
    /// a mesh centre — in PIXELS, tolerance 1e-6. This is the probe whose absence
    /// let the pitch/phase drift ship; the earlier mesh-only pitch self-check
    /// never touched hexbin output.
    /// The probe body: build a faithful hexbin batch for `(w, h, binWidth, raw
    /// extent)`, run it through the real render pipeline, and assert every hexbin
    /// centre coincides with a mesh centre in pixels (1e-6).
    fn assert_hexbin_mesh_coincides(
        w: f64,
        h: f64,
        b: f64,
        raw_x0: f64,
        raw_x1: f64,
        raw_y0: f64,
        raw_y1: f64,
    ) {
        let x_range = (40.0, 40.0 + w);
        let y_range = (20.0 + h, 20.0); // inverted, as the render pipeline builds it
        let (centres, dx_data, dy_data) =
            lowerer_hex_centres(w, h, b, raw_x0, raw_x1, raw_y0, raw_y1);
        assert!(centres.len() > 20, "enough occupied hexes to probe");

        // Build the hexbin batch exactly as the lowerer emits it — INCLUDING the
        // raw-extent columns augment_scales widens from.
        let f = |v: Vec<f64>| Arc::new(Float64Array::from(v)) as arrow::array::ArrayRef;
        let n = centres.len();
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new(HEX_DX_COL, DataType::Float64, false),
            Field::new(HEX_DY_COL, DataType::Float64, false),
            Field::new(HEX_X0_COL, DataType::Float64, false),
            Field::new(HEX_X1_COL, DataType::Float64, false),
            Field::new(HEX_Y0_COL, DataType::Float64, false),
            Field::new(HEX_Y1_COL, DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                f(centres.iter().map(|c| c.0).collect()),
                f(centres.iter().map(|c| c.1).collect()),
                f(vec![dx_data; n]),
                f(vec![dy_data; n]),
                f(vec![raw_x0; n]),
                f(vec![raw_x1; n]),
                f(vec![raw_y0; n]),
                f(vec![raw_y1; n]),
                f(vec![1.0; n]),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, DENSITY_COUNT_COL.to_string());

        let mut scales = infer_scales(&batch, &cm, x_range, y_range);
        HexbinRenderer::default().augment_scales(&mut scales, &batch, &cm, x_range, y_range);
        let x_scale = scales.get(Channel::X).unwrap();
        let y_scale = scales.get(Channel::Y).unwrap();

        // Mesh centres in pixels, via the same reconstruction render uses — with
        // the SAME binWidth the hexbin was binned at (so a binWidth-plumbing bug
        // would surface).
        let hexgrid = HexgridRenderer { bin_width: b };
        let lat = hexgrid
            .sibling_lattice(x_scale, y_scale)
            .expect("a widened data scale yields the sibling lattice");
        let mesh_px: Vec<(f64, f64)> = lat
            .data_centres()
            .into_iter()
            .map(|(cx, cy)| (x_scale.map_f64(cx), y_scale.map_f64(cy)))
            .collect();

        // Every hexbin centre (in pixels) must coincide with a mesh centre.
        let mut worst = 0.0_f64;
        for (cx, cy) in &centres {
            let (hx, hy) = (x_scale.map_f64(*cx), y_scale.map_f64(*cy));
            let nearest = mesh_px
                .iter()
                .map(|(mx, my)| ((mx - hx).powi(2) + (my - hy).powi(2)).sqrt())
                .fold(f64::INFINITY, f64::min);
            worst = worst.max(nearest);
        }
        assert!(
            worst < 1e-6,
            "every hexbin centre must sit on a mesh centre (w={w}, h={h}, binWidth={b}); \
             worst offset {worst} px"
        );
    }

    #[test]
    fn lattice_pitch_matches_hexbin_geometry() {
        // (a) Default binWidth, ISOTROPIC data-per-pixel (both axes ≈ 0.02).
        assert_hexbin_mesh_coincides(460.0, 370.0, 20.0, 0.0, 9.2, 0.0, 7.4);
        // (b) NON-default binWidth (30) on an ANISOTROPIC domain: x maps
        // 50 units over 500 px (0.10/px), y maps 6 units over 300 px (0.02/px) —
        // a 5× axis-ratio difference. This exercises binWidth plumbing and
        // independent per-axis scaling; a bug in either would break 1e-6.
        assert_hexbin_mesh_coincides(500.0, 300.0, 30.0, 0.0, 50.0, 0.0, 6.0);
    }

    /// a DATALESS hexgrid renders headlessly with NO data-driven
    /// scales — augment_scales synthesises the unit x/y scales from the plot
    /// ranges, and render then produces mesh geometry.
    #[test]
    fn dataless_hexgrid_renders_headlessly() {
        let renderer = HexgridRenderer::default();
        let batch = hexgrid_batch();
        let cm = ChannelMap::new();
        // No x/y scales to begin with (no data columns).
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        assert!(scales.get(Channel::X).is_none(), "no data-driven x scale");
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));
        assert!(scales.get(Channel::X).is_some(), "unit x scale synthesised");
        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert!(count_scene_paths(&scene) > 0, "dataless mesh still renders");
    }

    /// augment_scales does NOT clobber a sibling's data-driven scale
    /// (so a hexgrid + hexbin plot keeps the hexbin's real domain and the mesh
    /// rides it).
    #[test]
    fn hexgrid_augment_preserves_existing_scales() {
        let renderer = HexgridRenderer::default();
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 5.0,
                domain_max: 50.0,
                range_start: 40.0,
                range_end: 600.0,
            },
        );
        renderer.augment_scales(
            &mut scales,
            &hexgrid_batch(),
            &ChannelMap::new(),
            (40.0, 600.0),
            (450.0, 20.0),
        );
        // The existing x scale's data domain survives (not reset to [0,1]).
        assert_eq!(scales.get(Channel::X).unwrap().domain_max(), Some(50.0));
    }

    // Regression (review finding, major): a raster's augment_scales MERGES into the
    // shared Fill scale instead of clobbering it — a sibling's categorical Colour
    // survives, and two rasters union their zero-anchored domains.
    #[test]
    fn raster_augment_scales_merges_fill_not_clobber() {
        let make = |counts: Vec<f64>| {
            let schema = Arc::new(Schema::new(vec![
                Field::new("x_bin", DataType::Float64, false),
                Field::new("y_bin", DataType::Float64, false),
                Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
            ]));
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Float64Array::from(vec![0.0, 1.0])),
                    Arc::new(Float64Array::from(vec![0.0, 1.0])),
                    Arc::new(Float64Array::from(counts)),
                ],
            )
            .unwrap()
        };
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());

        // (a) A sibling mark's categorical Colour Fill is left untouched.
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["a".to_string(), "b".to_string()],
                palette: vec![[0.1, 0.2, 0.3, 1.0], [0.4, 0.5, 0.6, 1.0]],
            },
        );
        RasterRenderer::default().augment_scales(
            &mut scales,
            &make(vec![1.0, 9.0]),
            &cm,
            (0.0, 100.0),
            (100.0, 0.0),
        );
        match scales.get(Channel::Fill) {
            Some(Scale::Colour { categories, .. }) => assert_eq!(categories, &["a", "b"]),
            other => panic!("categorical Fill must survive a raster augment_scales, got {other:?}"),
        }

        // (b) Two rasters union their zero-anchored domains (maxes 10 and 100).
        let mut scales = ScaleSet::new();
        RasterRenderer::default().augment_scales(
            &mut scales,
            &make(vec![3.0, 10.0]),
            &cm,
            (0.0, 100.0),
            (100.0, 0.0),
        );
        RasterRenderer::default().augment_scales(
            &mut scales,
            &make(vec![50.0, 100.0]),
            &cm,
            (0.0, 100.0),
            (100.0, 0.0),
        );
        match scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                ..
            }) => {
                assert!((domain_min - 0.0).abs() < f64::EPSILON, "zero-anchored");
                assert!(
                    (domain_max - 100.0).abs() < f64::EPSILON,
                    "union to the larger max"
                );
            }
            other => panic!("expected a unioned Sequential Fill, got {other:?}"),
        }
    }

    // Regression (review finding): render samples the ramp position in the Fill
    // ramp's OWN domain, not the local batch max — so the RASTER_MIN_T floor and
    // the colours hold when a smaller raster shares a larger unioned domain.
    #[test]
    fn raster_render_samples_against_shared_ramp_domain() {
        let make = |counts: Vec<f64>| {
            let schema = Arc::new(Schema::new(vec![
                Field::new("x_bin", DataType::Float64, false),
                Field::new("y_bin", DataType::Float64, false),
                Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
            ]));
            RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Float64Array::from(vec![0.0, 1.0])),
                    Arc::new(Float64Array::from(vec![0.0, 1.0])),
                    Arc::new(Float64Array::from(counts)),
                ],
            )
            .unwrap()
        };
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());

        // Shared domain [0, 100] from a large raster; render the SMALL raster
        // (local max 40) against it.
        let big = make(vec![50.0, 100.0]);
        let small = make(vec![1.0, 40.0]);
        let mut scales = infer_scales(&small, &cm, (40.0, 600.0), (450.0, 20.0));
        RasterRenderer::default().augment_scales(
            &mut scales,
            &big,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );
        RasterRenderer::default().augment_scales(
            &mut scales,
            &small,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );
        let ramp = scales.get(Channel::Fill).expect("shared ramp");
        assert_eq!(ramp.domain_max(), Some(100.0), "domain unioned to 100");

        // The small raster's count-1 cell floors at RASTER_MIN_T in the SHARED
        // domain → sample 0.15·100 = 15, NOT the local-max 0.15·40 = 6; its
        // count-40 cell samples 40 (0.4·100). Both against the shared domain.
        let mut scene = Scene::new();
        RasterRenderer::default().render(&mut scene, &small, &cm, &scales, None);
        let expect_floor = ramp.map_continuous(RASTER_MIN_T * 100.0);
        let expect_hi = ramp.map_continuous(40.0);
        // Guard against a domain that would collapse the two cells to one colour.
        assert!(
            expect_floor != expect_hi,
            "the two cells are distinct under the shared domain"
        );
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn,
            std::collections::HashSet::from([packed(expect_floor), packed(expect_hi)]),
            "floor + colours sample against the shared domain, not the local max"
        );
    }

    // augment_scales widens the linear x/y domains by half a bin so the edge
    // cells (which extend ±half a bin past their centres) fit in the plot.
    #[test]
    fn raster_augment_scales_widens_domain_by_half_bin() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        // Centres 0,1,2 on both axes → bin pitch 1.0.
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])),
                Arc::new(Float64Array::from(vec![1.0, 1.0, 1.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        RasterRenderer::default().augment_scales(
            &mut scales,
            &batch,
            &cm,
            (40.0, 600.0),
            (450.0, 20.0),
        );

        match scales.get(Channel::X) {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                ..
            }) => {
                assert!(
                    (domain_min - (-0.5)).abs() < 1e-9,
                    "x domain min widened to -0.5"
                );
                assert!(
                    (domain_max - 2.5).abs() < 1e-9,
                    "x domain max widened to 2.5"
                );
            }
            other => panic!("expected a widened linear x scale, got {other:?}"),
        }
    }

    // augment_scales builds a Fill Sequential zero-anchored at
    // [0, max_count] with the scheme's stops, alongside the x/y half-bin widening.
    #[test]
    fn raster_augment_scales_builds_fill_sequential() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        // Centres 0,1,2 on both axes; counts up to 7 → domain [0, 7].
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])),
                Arc::new(Float64Array::from(vec![3.0, 7.0, 1.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        let renderer = RasterRenderer {
            scheme: SequentialScheme::Blues,
        };
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));

        match scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            }) => {
                assert!(
                    (domain_min - 0.0).abs() < f64::EPSILON,
                    "domain zero-anchored"
                );
                assert!(
                    (domain_max - 7.0).abs() < f64::EPSILON,
                    "domain_max == max count"
                );
                assert_eq!(
                    stops,
                    &SequentialScheme::Blues.stops(),
                    "stops match the scheme"
                );
            }
            other => panic!("expected a Fill Sequential scale, got {other:?}"),
        }
        // The x/y half-bin widening still holds.
        assert_eq!(scales.get(Channel::X).unwrap().domain_min(), Some(-0.5));
        assert_eq!(scales.get(Channel::Y).unwrap().domain_max(), Some(2.5));
    }

    /// The shared 3×3 density-lowerer fixture for the heatmap probes: a hot
    /// centre so the smoothed field has clearly distinct cell values.
    fn heatmap_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![
                    0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 1.0, 2.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    1.0, 4.0, 1.0, 4.0, 16.0, 4.0, 1.0, 4.0, 1.0,
                ])),
            ],
        )
        .unwrap()
    }

    // every KDE grid cell is coloured through the Fill Sequential ramp
    // (density → map_continuous) — the colours ACTUALLY ENCODED into the scene
    // are the ramp samples of the smoothed field (probed via draw_data, the #36
    // precedent), cells with different densities encode different colours, and
    // with no Fill scale the render falls back to alpha-on-default-blue. Driven by
    // the fixture — 8 SCRAMBLED rows with cell (2, 2) OMITTED — so the
    // "every cell" claim is falsifiable: an occupied-bins-only regression draws
    // 8 cells and misses the unoccupied cell's smoothed colour.
    // `1 * 3 + 1` is `row * width + col` for a 3-wide grid, not arithmetic to be
    // folded. Collapsing the `1 *` (clippy::identity_op) hides which cell the
    // "centre" is.
    #[allow(clippy::identity_op)]
    #[test]
    fn heatmap_colours_cells_through_ramp() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![
                    1.0, 0.0, 2.0, 0.0, 1.0, 2.0, 1.0, 0.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 2.0, 2.0,
                ])),
                Arc::new(Float64Array::from(vec![
                    16.0, 1.0, 1.0, 4.0, 4.0, 4.0, 4.0, 1.0,
                ])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let renderer = HeatmapRenderer::default();
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));

        // Expected ramp samples over the smoothed field, exactly as render maps.
        let grid = build_kde_grid(&batch, "x_bin", "y_bin", None).expect("grid builds");
        let ramp = scales.get(Channel::Fill).expect("fill ramp built");
        let expected: std::collections::HashSet<u32> = grid
            .density
            .iter()
            .map(|v| packed(ramp.map_continuous(*v)))
            .collect();
        let centre = packed(ramp.map_continuous(grid.density[1 * 3 + 1]));
        let corner = packed(ramp.map_continuous(grid.density[0]));
        assert_ne!(
            centre, corner,
            "distinct densities encode distinct ramp colours"
        );
        // The UNOCCUPIED bin (2, 2): zero count, but the smoothed field is
        // positive there, so its ramp colour must be drawn like any other cell.
        let unoccupied_density = grid.density[2 * 3 + 2];
        assert!(
            unoccupied_density > 0.0,
            "the smoothed field is positive at the unoccupied bin"
        );
        let unoccupied = packed(ramp.map_continuous(unoccupied_density));

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            9,
            "heatmap fills EVERY grid cell (9 on a 3×3 with only 8 occupied bins), \
             not just occupied bins"
        );
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn, expected,
            "the cell fills are the ramp samples of the smoothed field"
        );
        assert!(
            drawn.contains(&unoccupied),
            "the unoccupied bin's smoothed ramp colour is drawn — an \
             occupied-bins-only regression fails here"
        );

        // Fallback: no Fill scale → the default blue with density-proportional alpha.
        let mut no_fill = ScaleSet::new();
        no_fill.insert(Channel::X, scales.get(Channel::X).unwrap().clone());
        no_fill.insert(Channel::Y, scales.get(Channel::Y).unwrap().clone());
        let mut scene2 = Scene::new();
        renderer.render(&mut scene2, &batch, &cm, &no_fill, None);
        let [cr, cg, cb, _] = ChartInk::LIGHT.mark_default.components;
        let fallback_expected: std::collections::HashSet<u32> = grid
            .density
            .iter()
            .map(|v| packed([cr, cg, cb, (v / grid.max_density).clamp(0.0, 1.0) as f32]))
            .collect();
        let fallback: std::collections::HashSet<u32> =
            scene2.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            fallback, fallback_expected,
            "fallback keeps the default-blue hue with density-proportional alpha"
        );
    }

    // augment_scales builds a Fill Sequential zero-anchored at
    // [0, max_density] with the scheme's stops, widens x/y by half a bin to the
    // grid extent, and merges rather than clobbers (a sibling's categorical
    // Colour Fill survives) — mirroring raster's augment_scales contract.
    #[test]
    fn heatmap_augment_scales_builds_zero_anchored_fill() {
        let batch = heatmap_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let renderer = HeatmapRenderer {
            scheme: SequentialScheme::Blues,
            bandwidth: None,
        };
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let grid = build_kde_grid(&batch, "x_bin", "y_bin", None).expect("grid builds");
        match scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            }) => {
                assert!(
                    (domain_min - 0.0).abs() < f64::EPSILON,
                    "domain zero-anchored"
                );
                assert!(
                    (domain_max - grid.max_density).abs() < f64::EPSILON,
                    "domain_max == max smoothed density"
                );
                assert_eq!(
                    stops,
                    &SequentialScheme::Blues.stops(),
                    "stops match the scheme"
                );
            }
            other => panic!("expected a Fill Sequential scale, got {other:?}"),
        }
        // x/y widened by half a bin past the outermost centres (0..2, pitch 1).
        assert_eq!(scales.get(Channel::X).unwrap().domain_min(), Some(-0.5));
        assert_eq!(scales.get(Channel::X).unwrap().domain_max(), Some(2.5));
        assert_eq!(scales.get(Channel::Y).unwrap().domain_min(), Some(-0.5));
        assert_eq!(scales.get(Channel::Y).unwrap().domain_max(), Some(2.5));

        // A sibling mark's categorical Colour Fill is left untouched.
        let mut with_colour = ScaleSet::new();
        with_colour.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["a".to_string(), "b".to_string()],
                palette: vec![[0.1, 0.2, 0.3, 1.0], [0.4, 0.5, 0.6, 1.0]],
            },
        );
        renderer.augment_scales(&mut with_colour, &batch, &cm, (40.0, 600.0), (450.0, 20.0));
        match with_colour.get(Channel::Fill) {
            Some(Scale::Colour { categories, .. }) => assert_eq!(categories, &["a", "b"]),
            other => {
                panic!("categorical Fill must survive a heatmap augment_scales, got {other:?}")
            }
        }
    }

    // the mark's `bandwidth` attribute reaches kde_2d through the
    // renderer — an explicit bandwidth renders a DIFFERENT field than the
    // Silverman fallback, and exactly the field build_kde_grid produces for it.
    #[test]
    fn heatmap_bandwidth_attr_reaches_kde() {
        let batch = heatmap_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());

        let render_with = |renderer: &HeatmapRenderer| {
            let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
            renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));
            let mut scene = Scene::new();
            renderer.render(&mut scene, &batch, &cm, &scales, None);
            let drawn: std::collections::HashSet<u32> =
                scene.encoding().draw_data.iter().copied().collect();
            (drawn, scales)
        };

        let explicit = HeatmapRenderer {
            scheme: SequentialScheme::default(),
            bandwidth: Some(0.5),
        };
        let (drawn_explicit, scales_explicit) = render_with(&explicit);
        let (drawn_silverman, _) = render_with(&HeatmapRenderer::default());
        assert_ne!(
            drawn_explicit, drawn_silverman,
            "an explicit bandwidth changes the rendered field vs the Silverman fallback"
        );

        // The explicit render equals the ramp samples of build_kde_grid(Some(0.5)).
        let grid = build_kde_grid(&batch, "x_bin", "y_bin", Some(0.5)).expect("grid builds");
        let ramp = scales_explicit.get(Channel::Fill).expect("fill ramp built");
        let expected: std::collections::HashSet<u32> = grid
            .density
            .iter()
            .map(|v| packed(ramp.map_continuous(*v)))
            .collect();
        assert_eq!(
            drawn_explicit, expected,
            "bandwidth threads through to the drawn field"
        );
    }

    // build_kde_grid materialises a DENSE first..last lattice at the
    // recovered pitch — unoccupied interior bins carry zero mass. x centres
    // {0.5, 15.5, 16.5} have a TRUE pitch of 1 (the GCD of the gaps {15, 1}),
    // so the grid now spans {0.5, 1.5, ..., 16.5} (17 columns) at grid.dx == 1
    // rather than the old three-column grid whose naive first-gap pitch read 15.
    // The draw already recovered the pitch; the fix moves the SMOOTHED lattice
    // onto the same true geometry (the deliberate density-family re-baseline).
    #[test]
    fn heatmap_gapped_centres_cells_drawn_at_recovered_pitch() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x_bin", DataType::Float64, false),
            Field::new("y_bin", DataType::Float64, false),
            Field::new(DENSITY_COUNT_COL, DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.5, 15.5, 16.5, 0.5])),
                Arc::new(Float64Array::from(vec![0.5, 1.5, 0.5, 1.5])),
                Arc::new(Float64Array::from(vec![1.0, 4.0, 2.0, 3.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());

        let grid = build_kde_grid(&batch, "x_bin", "y_bin", None).expect("grid builds");
        assert_eq!(
            grid.dx, 1.0,
            "the dense lattice carries the recovered pitch 1"
        );
        assert_eq!(
            grid.x_centres.len(),
            17,
            "dense first..last lattice materialises the gap bins"
        );
        assert_eq!(
            bin_step(&grid.x_centres),
            Some(1.0),
            "the dense lattice is uniform at the true pitch"
        );

        // Identity scales (domain == pixel range), so drawn coordinates ARE data
        // units and the encoded f32 coordinate stream can be read back directly.
        let identity = |hi: f64| Scale::Linear {
            domain_min: 0.0,
            domain_max: hi,
            range_start: 0.0,
            range_end: hi,
        };
        let mut scales = ScaleSet::new();
        scales.insert(Channel::X, identity(20.0));
        scales.insert(Channel::Y, identity(20.0));
        let mut scene = Scene::new();
        HeatmapRenderer::default().render(&mut scene, &batch, &cm, &scales, None);
        // path_data is the packed f32 coordinate stream (vello 0.9 stores it as
        // u32 bit patterns); quantise to quarter-units for exact set membership.
        let coords: std::collections::HashSet<i64> = scene
            .encoding()
            .path_data
            .iter()
            .map(|w| (f32::from_bits(*w) as f64 * 4.0).round() as i64)
            .collect();
        let has = |v: f64| coords.contains(&((v * 4.0).round() as i64));
        // Every cell spans its centre ± half the recovered pitch: the first cell
        // (centre 0.5) has edges 0 and 1, the sparse cells keep 1-wide edges too.
        assert!(
            has(0.0) && has(1.0),
            "first cell drawn at the recovered pitch 1"
        );
        assert!(
            has(15.0) && has(17.0),
            "sparse cells drawn at the recovered pitch 1"
        );
        // The dense lattice tiles the whole span at unit pitch: interior gap bins
        // are materialised (zero mass) and drawn, so an interior edge like 8.0
        // is a genuine cell boundary now — but nothing spills past the [0, 17]
        // lattice bounds (the old gap-naive smear reached 0.5 ± 7.5 → -7 and 8).
        assert!(
            has(8.0),
            "interior gap bins are materialised in the dense lattice"
        );
        assert!(
            !has(-7.0) && !has(24.0),
            "no cell spills past the dense lattice bounds"
        );

        // augment_scales widens the axes by half the SAME recovered pitch, so
        // the domain hugs the drawn cells: x [0, 17], y [0, 2].
        let mut aug = ScaleSet::new();
        HeatmapRenderer::default().augment_scales(&mut aug, &batch, &cm, (0.0, 20.0), (0.0, 20.0));
        assert_eq!(aug.get(Channel::X).unwrap().domain_min(), Some(0.0));
        assert_eq!(aug.get(Channel::X).unwrap().domain_max(), Some(17.0));
        assert_eq!(aug.get(Channel::Y).unwrap().domain_min(), Some(0.0));
        assert_eq!(aug.get(Channel::Y).unwrap().domain_max(), Some(2.0));
    }

    // the renderer strokes one path per chained iso-line, and the
    // `thresholds` attr drives the LEVEL count — 5 levels stroke strictly more
    // iso-lines than 2 over the same grid, and the stroked path count equals a
    // replay of contour_polylines over the same shared KDE grid at the same
    // levels (the SQL-side half of the shield lives in brightfield-sql's
    // regression test).
    #[test]
    fn contour_iso_line_count_follows_thresholds() {
        let batch = heatmap_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x_bin".to_string());
        cm.insert(Channel::Y, "y_bin".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let paths_at = |thresholds: usize| {
            let renderer = ContourRenderer {
                thresholds: Some(thresholds),
                bandwidth: None,
            };
            let mut scene = Scene::new();
            renderer.render(&mut scene, &batch, &cm, &scales, None);
            count_scene_paths(&scene)
        };

        // Expected per-level line counts, replayed over the same grid.
        let grid = build_kde_grid(&batch, "x_bin", "y_bin", None).expect("grid builds");
        let expected_at = |thresholds: usize| -> usize {
            crate::contour::iso_levels(grid.max_density, thresholds)
                .iter()
                .map(|level| {
                    crate::contour::contour_polylines(
                        &grid.density,
                        grid.rows(),
                        grid.cols(),
                        &grid.x_centres,
                        &grid.y_centres,
                        *level,
                    )
                    .len()
                })
                .sum()
        };

        let (two, five) = (paths_at(2), paths_at(5));
        assert_eq!(two, expected_at(2), "one stroked path per chained iso-line");
        assert_eq!(
            five,
            expected_at(5),
            "one stroked path per chained iso-line"
        );
        assert!(
            five > two && two >= 2,
            "thresholds drives the iso-line count ({two} at 2 vs {five} at 5)"
        );
    }

    /// Pre-aggregated cell fixture: 2 days × 2 slots with a numeric value and a
    /// categorical grade per pair.
    fn cell_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Utf8, false),
            Field::new("slot", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
            Field::new("grade", DataType::Utf8, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["Mon", "Mon", "Tue", "Tue"])),
                Arc::new(StringArray::from(vec!["am", "pm", "am", "pm"])),
                Arc::new(Float64Array::from(vec![1.0, 4.0, 2.0, 8.0])),
                Arc::new(StringArray::from(vec!["a", "b", "a", "b"])),
            ],
        )
        .unwrap()
    }

    // one rect per occupied (x category, y category) pair, positioned
    // on the two Band scales, with distinct numeric fill values encoding
    // distinct ramp colours (probed via draw_data per the #36 precedent).
    #[test]
    fn cell_renders_rect_per_category_pair() {
        let batch = cell_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "slot".to_string());
        cm.insert(Channel::Y, "day".to_string());
        cm.insert(Channel::Fill, "value".to_string());
        let renderer = CellRenderer::default();
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));
        assert!(
            matches!(scales.get(Channel::X), Some(Scale::Band { .. }))
                && matches!(scales.get(Channel::Y), Some(Scale::Band { .. })),
            "cell rides the existing per-channel Band inference on both axes"
        );

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            4,
            "one rect per occupied category pair"
        );
        let ramp = scales.get(Channel::Fill).expect("fill ramp built");
        let expected: std::collections::HashSet<u32> = [1.0, 4.0, 2.0, 8.0]
            .iter()
            .map(|v| packed(ramp.map_continuous(*v)))
            .collect();
        assert_eq!(
            expected.len(),
            4,
            "the four values encode four distinct colours"
        );
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn, expected,
            "cell fills are the ramp samples of the values"
        );
    }

    // augment_scales anchors the Fill Sequential domain per the v1
    // rule — [0, max] when min >= 0, else [min, max] — REPLACING the Linear a
    // numeric fill otherwise infers (the trap), unioning with a co-rendered
    // Sequential, and leaving a categorical Colour fill untouched.
    #[test]
    fn cell_augment_scales_anchors_sequential_domain() {
        let batch = cell_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "slot".to_string());
        cm.insert(Channel::Y, "day".to_string());
        cm.insert(Channel::Fill, "value".to_string());
        let renderer = CellRenderer {
            scheme: SequentialScheme::Blues,
        };

        // min >= 0 (values 1..8): the inferred Linear is replaced by a
        // zero-anchored Sequential with the scheme's stops.
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        assert!(
            matches!(scales.get(Channel::Fill), Some(Scale::Linear { .. })),
            "precondition: generic inference types the numeric fill Linear"
        );
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));
        match scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            }) => {
                assert!(
                    (domain_min - 0.0).abs() < f64::EPSILON,
                    "min >= 0 anchors at zero"
                );
                assert!((domain_max - 8.0).abs() < f64::EPSILON);
                assert_eq!(
                    stops,
                    &SequentialScheme::Blues.stops(),
                    "stops match the scheme"
                );
            }
            other => panic!("expected a Fill Sequential, got {other:?}"),
        }

        // min < 0: the domain is [min, max], not forced through zero.
        let neg_schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Utf8, false),
            Field::new("slot", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let neg = RecordBatch::try_new(
            neg_schema,
            vec![
                Arc::new(StringArray::from(vec!["Mon", "Tue"])),
                Arc::new(StringArray::from(vec!["am", "pm"])),
                Arc::new(Float64Array::from(vec![-3.0, 5.0])),
            ],
        )
        .unwrap();
        let mut neg_scales = infer_scales(&neg, &cm, (40.0, 600.0), (450.0, 20.0));
        renderer.augment_scales(&mut neg_scales, &neg, &cm, (40.0, 600.0), (450.0, 20.0));
        match neg_scales.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                ..
            }) => {
                assert!(
                    (domain_min - (-3.0)).abs() < f64::EPSILON,
                    "min < 0 keeps the data min"
                );
                assert!((domain_max - 5.0).abs() < f64::EPSILON);
            }
            other => panic!("expected a Fill Sequential, got {other:?}"),
        }

        // A co-rendered Sequential unions (keeping the first's stops); a
        // categorical Colour fill survives untouched.
        let mut union = ScaleSet::new();
        union.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 0.0,
                domain_max: 100.0,
                stops: SequentialScheme::Viridis.stops(),
            },
        );
        renderer.augment_scales(&mut union, &batch, &cm, (40.0, 600.0), (450.0, 20.0));
        match union.get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_max, stops, ..
            }) => {
                assert_eq!(*domain_max, 100.0, "union keeps the wider domain");
                assert_eq!(
                    stops,
                    &SequentialScheme::Viridis.stops(),
                    "first scale's stops win"
                );
            }
            other => panic!("expected a unioned Sequential, got {other:?}"),
        }
        let mut colour = ScaleSet::new();
        colour.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["a".to_string()],
                palette: vec![[0.1, 0.2, 0.3, 1.0]],
            },
        );
        renderer.augment_scales(&mut colour, &batch, &cm, (40.0, 600.0), (450.0, 20.0));
        assert!(
            matches!(colour.get(Channel::Fill), Some(Scale::Colour { .. })),
            "a categorical Colour fill wins over the numeric ramp"
        );
    }

    // a Utf8 fill keeps the existing categorical Colour behaviour —
    // augment_scales leaves the inferred Colour scale alone and the rects draw
    // in palette colours through resolve_colour, exactly as before.
    #[test]
    fn cell_utf8_fill_keeps_colour_path() {
        let batch = cell_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "slot".to_string());
        cm.insert(Channel::Y, "day".to_string());
        cm.insert(Channel::Fill, "grade".to_string());
        let renderer = CellRenderer::default();
        let mut scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        renderer.augment_scales(&mut scales, &batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let (cat_a, cat_b) = match scales.get(Channel::Fill) {
            Some(scale @ Scale::Colour { .. }) => (
                scale.map_colour("a").expect("category a"),
                scale.map_colour("b").expect("category b"),
            ),
            other => panic!("Utf8 fill must keep the categorical Colour scale, got {other:?}"),
        };

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(count_scene_paths(&scene), 4, "one rect per category pair");
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn,
            std::collections::HashSet::from([packed(cat_a), packed(cat_b)]),
            "cells draw in the categorical palette colours"
        );
    }

    #[test]
    fn regression_renders_line_and_ci_band() {
        // Anscombe Quartet I (the canonical OLS dataset).
        // n=11, slope=0.5, intercept=3, x_bar=9, sxx=110.
        // We compute syy and sxy from the data.
        let xs = [10.0, 8.0, 13.0, 9.0, 11.0, 14.0, 6.0, 4.0, 12.0, 7.0, 5.0];
        let ys = [
            8.04, 6.95, 7.58, 8.81, 8.33, 9.96, 7.24, 4.26, 10.84, 4.82, 5.68,
        ];
        let n = xs.len() as f64;
        let x_bar = xs.iter().sum::<f64>() / n;
        let mean_y = ys.iter().sum::<f64>() / n;
        let sxx: f64 = xs.iter().map(|x| (x - x_bar).powi(2)).sum();
        let syy: f64 = ys.iter().map(|y| (y - mean_y).powi(2)).sum();
        let sxy: f64 = xs
            .iter()
            .zip(ys.iter())
            .map(|(x, y)| (x - x_bar) * (y - mean_y))
            .sum();
        let slope = sxy / sxx;
        let intercept = mean_y - slope * x_bar;

        // Build a one-row aggregate batch.
        let schema = Arc::new(Schema::new(vec![
            Field::new("slope", DataType::Float64, false),
            Field::new("intercept", DataType::Float64, false),
            Field::new("n", DataType::Float64, false),
            Field::new("x_bar", DataType::Float64, false),
            Field::new("sxx", DataType::Float64, false),
            Field::new("sxy", DataType::Float64, false),
            Field::new("syy", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![slope])),
                Arc::new(Float64Array::from(vec![intercept])),
                Arc::new(Float64Array::from(vec![n])),
                Arc::new(Float64Array::from(vec![x_bar])),
                Arc::new(Float64Array::from(vec![sxx])),
                Arc::new(Float64Array::from(vec![sxy])),
                Arc::new(Float64Array::from(vec![syy])),
            ],
        )
        .unwrap();

        let cm = ChannelMap::new();
        // Build scales manually with a known x domain so x_min/x_max are non-default.
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 20.0,
                range_start: 40.0,
                range_end: 600.0,
            },
        );
        scales.insert(
            Channel::Y,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 12.0,
                range_start: 450.0,
                range_end: 20.0,
            },
        );

        let mut scene = Scene::new();
        let renderer = RegressionRenderer { ci: 0.95 };
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        // The spec requires both a fitted line (stroke) AND a CI band
        // (fill). vello's n_paths counter increments once per fill or
        // stroke, so the regression renderer must produce ≥2 paths.
        assert!(
            count_scene_paths(&scene) >= 2,
            "RegressionRenderer must emit ≥2 paths (fitted line + CI band), got {}",
            count_scene_paths(&scene)
        );
        // Sanity-check the slope/intercept on Anscombe I.
        assert!(
            (slope - 0.5).abs() < 0.01,
            "Anscombe I slope ≈ 0.5 ({slope})"
        );
        assert!(
            (intercept - 3.0).abs() < 0.05,
            "Anscombe I intercept ≈ 3.0 ({intercept})"
        );
    }

    /// **The dash is intermittent, it inks about six pixels in every ten, and
    /// its rhythm survives a segment join.**
    ///
    /// Asserted on total drawn LENGTH, not on a count of segments: a "dash"
    /// that emitted a hundred runs butted end to end would satisfy a count and
    /// look exactly like a solid line.
    ///
    /// The polyline is deliberately sampled at 7 px against a 10 px period, so
    /// the phase does not line up with the joins. Under a walker that reset its
    /// phase at each segment — the natural wrong implementation — every dash
    /// would start at a join and none would continue across one, which is what
    /// the last assertion catches.
    #[test]
    fn a_dash_draws_gaps_and_carries_its_rhythm_across_joins() {
        let pts: Vec<(f64, f64)> = (0..=10).map(|i| (f64::from(i) * 7.0, 50.0)).collect();
        let runs = dash_polyline(&pts, BEYOND_FRAME_DASH, BEYOND_FRAME_GAP);
        let inked: f64 = runs.iter().map(|l| (l.p1 - l.p0).hypot()).sum();
        let total = 70.0;

        assert!(!runs.is_empty(), "a dashed 70 px run drew nothing");
        assert!(
            (inked / total - 0.6).abs() < 0.08,
            "a 6-on/4-off dash inked {:.0}% of the path, expected about 60% — a pattern \
             that inks all of it is a solid line with extra steps in it",
            inked / total * 100.0
        );

        let joined = runs
            .windows(2)
            .filter(|w| (w[0].p1.x - w[1].p0.x).abs() < 1e-6)
            .count();
        assert!(
            joined > 0,
            "no dash continued across a segment join, so the phase is being reset at each \
             one — the dashes would bunch wherever the sampling happens to be dense"
        );
    }

    /// **A fit that could not rescope is drawn differently from one that did**,
    /// through the two entry points the scene builder chooses between.
    ///
    /// This is a structural check and says so: it holds that the dashed call
    /// emits more, shorter strokes over the same path from the same batch. What
    /// holds that the difference reaches a PICTURE is the shell's
    /// `the_unrescoped_fit_is_dashed_in_the_exported_picture`, which counts
    /// runs of mark ink in an exported PNG. Name the test that fails.
    #[test]
    fn the_two_entry_points_draw_the_fit_differently() {
        let (batch, cm, scales) = anscombe_fit();
        let renderer = RegressionRenderer::default();

        let mut solid = Scene::new();
        renderer.render(&mut solid, &batch, &cm, &scales, None);
        let mut dashed = Scene::new();
        renderer.render_beyond_frame(&mut dashed, &batch, &cm, &scales, None);

        let (solid_paths, dashed_paths) = (count_scene_paths(&solid), count_scene_paths(&dashed));
        assert!(
            dashed_paths > solid_paths,
            "the dashed fit emitted {dashed_paths} paths against the solid fit's \
             {solid_paths} — if they are equal, `render_beyond_frame` is still the \
             default forward and the picture says nothing"
        );

        // …and the extra paths are gaps, not subdivision: the dashed geometry
        // covers well under the whole path.
        let path = sampled_fit_path(&scales);
        let full: f64 = path
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum();
        let inked: f64 = dash_polyline(&path, BEYOND_FRAME_DASH, BEYOND_FRAME_GAP)
            .iter()
            .map(|l| (l.p1 - l.p0).hypot())
            .sum();
        assert!(
            inked < full * 0.8,
            "the dashed fit covers {:.0}% of the path — too close to solid to read as a \
             different treatment",
            inked / full * 100.0
        );
    }

    /// A census of the ink a scene draws in: packed premultiplied RGBA8 word →
    /// how many path-producing ops were drawn with it.
    ///
    /// `Scene::fill` and `Scene::stroke` with a solid brush each push exactly
    /// one such word onto the encoding's draw-data stream (`encode_color`), so
    /// this is a faithful account of what was drawn AND in what ink — the ALPHA
    /// channel included, which is the channel this treatment is forbidden from
    /// touching. The word is little-endian with red in the low byte, so alpha
    /// is the top one.
    fn ink_census(scene: &Scene) -> std::collections::BTreeMap<u32, usize> {
        let mut out = std::collections::BTreeMap::new();
        for &word in &scene.encoding().draw_data {
            *out.entry(word).or_insert(0usize) += 1;
        }
        out
    }

    /// Split a regression scene's paths into (band paths, fit-line paths) by
    /// the alpha they were drawn at.
    ///
    /// The band is the only thing `RegressionRenderer` draws below full
    /// opacity, so a sub-alpha path is a band path BY CONSTRUCTION and the
    /// fitted line's dashes — full alpha, every one — cannot inflate that
    /// count. That is the whole point of measuring this way: a test that
    /// counted the scene's paths would be satisfied by a treatment applied to
    /// the line alone, which is exactly the state this work is fixing.
    ///
    /// The two-ink requirement is asserted rather than assumed. If a third ever
    /// appears the premise is gone, and this fails loudly instead of quietly
    /// counting the wrong thing.
    fn band_and_fit_paths(scene: &Scene) -> (usize, usize) {
        let census = ink_census(scene);
        assert_eq!(
            census.len(),
            2,
            "a one-group regression is supposed to draw in exactly two inks — the \
             fit at full alpha and its band below it — but the scene holds {}: \
             {census:?}. Until that is understood, sub-alpha no longer means band",
            census.len()
        );
        let mut band = 0usize;
        let mut fit = 0usize;
        for (word, count) in census {
            if (word >> 24) as u8 == 0xff {
                fit += count;
            } else {
                band += count;
            }
        }
        (band, fit)
    }

    /// **A confidence band belonging to a fit that could not rescope is drawn
    /// differently from one that did**, and the difference is the BAND's, not
    /// borrowed from the fitted line beside it.
    ///
    /// The band is the interval claim itself and by area the larger half of the
    /// mark. Before this, it was filled identically either way: half the mark
    /// said "computed from rows outside this frame" and half went on asserting
    /// a confidence interval over exactly those rows.
    ///
    /// Counted on sub-alpha paths, so the fit line's own dashes cannot pay for
    /// the band's caveat — see [`band_and_fit_paths`].
    #[test]
    fn the_band_of_an_unrescoped_fit_is_drawn_differently_from_one_that_rescoped() {
        let (batch, cm, scales) = anscombe_fit();
        let renderer = RegressionRenderer::default();

        let mut solid = Scene::new();
        renderer.render(&mut solid, &batch, &cm, &scales, None);
        let mut dashed = Scene::new();
        renderer.render_beyond_frame(&mut dashed, &batch, &cm, &scales, None);

        let (solid_band, _) = band_and_fit_paths(&solid);
        let (dashed_band, _) = band_and_fit_paths(&dashed);

        assert_eq!(
            solid_band, 1,
            "a fit that rescoped draws its band as one filled polygon and nothing \
             else; this scene drew {solid_band} band paths, so the baseline the \
             comparison below rests on has moved"
        );
        assert!(
            dashed_band > solid_band,
            "the band drew {dashed_band} paths whether or not the fit outlived its \
             frame, so the caveat sits on the fitted line alone — the larger half of \
             the mark still asserts an interval over rows that are off screen"
        );
    }

    /// **The band's caveat is the fit's own dash, not a second vocabulary** —
    /// asserted as two separate claims, because they fail separately.
    ///
    /// The RHYTHM. Each band edge is a slightly bowed copy of the fitted line
    /// over the same x span, so at one shared 6-on/4-off period the two edges
    /// together must ink about twice the runs the line does. A period twice as
    /// coarse lands near 1×, one twice as fine near 4×, and a solid outline at
    /// two paths for the pair — none of those survive the bound.
    ///
    /// The INK. The set of colours the scene draws in does not change. The
    /// treatment moves texture and nothing else: `dash_polyline`'s own comment
    /// records why desaturation was refused for the line, and a band answering
    /// the caveat by fading, by thinning to some third alpha, or by reaching
    /// for a status hue would put a word in this set that the untreated scene
    /// does not hold. [`BAND_ALPHA`] records what else rides on the edge
    /// keeping the fill's own alpha, and why that one is not guarded by a test
    /// that fails.
    #[test]
    fn the_bands_caveat_is_the_fits_own_dash_and_not_a_second_vocabulary() {
        let (batch, cm, scales) = anscombe_fit();
        let renderer = RegressionRenderer::default();

        let mut solid = Scene::new();
        renderer.render(&mut solid, &batch, &cm, &scales, None);
        let mut dashed = Scene::new();
        renderer.render_beyond_frame(&mut dashed, &batch, &cm, &scales, None);

        let (solid_band, _) = band_and_fit_paths(&solid);
        let (dashed_band, dashed_fit) = band_and_fit_paths(&dashed);

        // The band's fill is one path in either scene; what is left is the edge.
        let edge_runs = dashed_band - solid_band;
        let ratio = edge_runs as f64 / dashed_fit as f64;
        assert!(
            (1.8..=2.6).contains(&ratio),
            "the two band edges inked {edge_runs} runs against the fitted line's \
             {dashed_fit} — {ratio:.2}× where one shared rhythm over two bowed \
             copies of the same span has to land near 2×. Near 1× is a coarser \
             period, near 4× a finer one, and a handful is a solid outline: any of \
             them is a second texture in a mark that is supposed to speak once"
        );

        let solid_inks: Vec<u32> = ink_census(&solid).into_keys().collect();
        let dashed_inks: Vec<u32> = ink_census(&dashed).into_keys().collect();
        assert_eq!(
            solid_inks, dashed_inks,
            "the treated mark draws in inks the untreated one does not. Only the \
             texture is allowed to move: a new word here is a fade, a third alpha, \
             or a hue this vocabulary has not earned"
        );
    }

    /// Whether a scene encodes at least one FILL, as against strokes only.
    ///
    /// vello packs "fill or stroke" into the top bit of its style stream's
    /// `flags_and_miter_limit` — `Style::FLAGS_STYLE_BIT`, 0 for a fill and 1
    /// for a stroke. The constant is public but sits on a type vello does not
    /// re-export, so it is written out here rather than imported. The test
    /// below proves the bit is being read the right way round before it trusts
    /// the answer.
    fn draws_a_fill(scene: &Scene) -> bool {
        const STYLE_BIT: u32 = 0x8000_0000;
        scene
            .encoding()
            .styles
            .iter()
            .any(|s| s.flags_and_miter_limit & STYLE_BIT == 0)
    }

    /// The same fit with `n` overridden.
    ///
    /// `n` is the one knob that turns the band off — below 3 the variance
    /// estimate has no degrees of freedom — without moving the fitted line,
    /// whose pixels are slope and intercept alone.
    fn with_n(batch: &RecordBatch, n: f64) -> RecordBatch {
        let idx = batch.schema().index_of("n").expect("n column");
        let mut cols = batch.columns().to_vec();
        cols[idx] = Arc::new(Float64Array::from(vec![n]));
        RecordBatch::try_new(batch.schema(), cols).expect("rebuilt batch")
    }

    /// **A band that wears the caveat still states its interval.** The dash is
    /// a texture on the boundary, not the loss of what the boundary encloses.
    ///
    /// Neither test above holds this. Both count band paths, and an
    /// implementation that dropped the fill and drew only the two dashed edges
    /// would satisfy them both — while being the "refuse to draw" answer that
    /// was rejected for the whole mark, applied to half of it. A reader would
    /// lose the interval on a gesture they read as a camera move, which is a
    /// worse state than the unqualified band this work started from.
    ///
    /// The `n = 2` scene is here to prove the measure discriminates: it
    /// suppresses the band and leaves a stroked line, so a `draws_a_fill` that
    /// answered yes to everything — the bit read the wrong way round — fails
    /// there instead of passing silently below.
    #[test]
    fn the_band_that_wears_the_caveat_still_states_its_interval() {
        let (batch, cm, scales) = anscombe_fit();
        let renderer = RegressionRenderer::default();

        let mut bandless = Scene::new();
        renderer.render_beyond_frame(&mut bandless, &with_n(&batch, 2.0), &cm, &scales, None);
        assert!(
            !draws_a_fill(&bandless),
            "a fit drawn with its band suppressed encodes nothing but strokes, yet \
             the fill probe says otherwise — it is not reading vello's style bit, so \
             the assertion below would pass on anything"
        );

        let mut dashed = Scene::new();
        renderer.render_beyond_frame(&mut dashed, &batch, &cm, &scales, None);
        assert!(
            draws_a_fill(&dashed),
            "the band that could not rescope has stopped being filled. Dashing its \
             edge is meant to qualify the interval, not withdraw it — a hollow \
             outline says the fit no longer states one at all"
        );

        // …and the fill is still the band's own, at the band's own alpha: one
        // sub-alpha path over and above the two dashed edges.
        let mut solid = Scene::new();
        renderer.render(&mut solid, &batch, &cm, &scales, None);
        let (solid_band, _) = band_and_fit_paths(&solid);
        assert_eq!(
            solid_band, 1,
            "the untreated band is supposed to be exactly one filled path at its own \
             alpha, got {solid_band}"
        );
    }

    /// Anscombe I as a one-row regression batch plus the scales the fit is
    /// sampled over — the fixture two tests share.
    fn anscombe_fit() -> (RecordBatch, ChannelMap, ScaleSet) {
        let xs = [10.0, 8.0, 13.0, 9.0, 11.0, 14.0, 6.0, 4.0, 12.0, 7.0, 5.0];
        let ys = [
            8.04, 6.95, 7.58, 8.81, 8.33, 9.96, 7.24, 4.26, 10.84, 4.82, 5.68,
        ];
        let n = xs.len() as f64;
        let x_bar = xs.iter().sum::<f64>() / n;
        let mean_y = ys.iter().sum::<f64>() / n;
        let sxx: f64 = xs.iter().map(|x| (x - x_bar).powi(2)).sum();
        let syy: f64 = ys.iter().map(|y| (y - mean_y).powi(2)).sum();
        let sxy: f64 = xs
            .iter()
            .zip(ys.iter())
            .map(|(x, y)| (x - x_bar) * (y - mean_y))
            .sum();
        let slope = sxy / sxx;
        let intercept = mean_y - slope * x_bar;

        let schema = Arc::new(Schema::new(vec![
            Field::new("slope", DataType::Float64, false),
            Field::new("intercept", DataType::Float64, false),
            Field::new("n", DataType::Float64, false),
            Field::new("x_bar", DataType::Float64, false),
            Field::new("sxx", DataType::Float64, false),
            Field::new("sxy", DataType::Float64, false),
            Field::new("syy", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![slope])),
                Arc::new(Float64Array::from(vec![intercept])),
                Arc::new(Float64Array::from(vec![n])),
                Arc::new(Float64Array::from(vec![x_bar])),
                Arc::new(Float64Array::from(vec![sxx])),
                Arc::new(Float64Array::from(vec![sxy])),
                Arc::new(Float64Array::from(vec![syy])),
            ],
        )
        .unwrap();

        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 20.0,
                range_start: 40.0,
                range_end: 600.0,
            },
        );
        scales.insert(
            Channel::Y,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 12.0,
                range_start: 450.0,
                range_end: 20.0,
            },
        );
        (batch, ChannelMap::new(), scales)
    }

    /// The pixel path the fit is stroked along, in the same 32 samples the
    /// renderer takes.
    fn sampled_fit_path(scales: &ScaleSet) -> Vec<(f64, f64)> {
        let x = scales.get(Channel::X).expect("x scale");
        let y = scales.get(Channel::Y).expect("y scale");
        let (lo, hi) = (x.domain_min().unwrap(), x.domain_max().unwrap());
        (0..32)
            .map(|i| {
                let t = f64::from(i) / 31.0;
                let xv = lo + (hi - lo) * t;
                (x.map_f64(xv), y.map_f64(0.5 * xv + 3.0))
            })
            .collect()
    }

    #[test]
    fn default_renderers_finds_density_and_regression() {
        let registry = default_renderers();
        assert!(find_renderer(&registry, MarkKind::Dot).is_some());
        assert!(find_renderer(&registry, MarkKind::BarX).is_some());
        assert!(find_renderer(&registry, MarkKind::Line).is_some());
        assert!(find_renderer(&registry, MarkKind::Density).is_some());
        assert!(find_renderer(&registry, MarkKind::DensityX).is_some());
        assert!(find_renderer(&registry, MarkKind::DensityY).is_some());
        assert!(find_renderer(&registry, MarkKind::RegressionX).is_some());
        assert!(find_renderer(&registry, MarkKind::RegressionY).is_some());
        // Heatmap is registered as of the density-marks instalment.
        assert!(find_renderer(&registry, MarkKind::Heatmap).is_some());
        // Hexbin is registered as of the hexbin follow-up.
        assert!(find_renderer(&registry, MarkKind::Hexbin).is_some());
        // Geo is registered as of the geo mark.
        assert!(find_renderer(&registry, MarkKind::Geo).is_some());
        // Unimplemented kinds should return None (no silent fallback). Voronoi is
        // the always-unimplemented census stand-in (geo's former role).
        assert!(find_renderer(&registry, MarkKind::Voronoi).is_none());
    }

    #[test]
    fn bar_default_render_interpolated_produces_content() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
            ],
        )
        .unwrap();

        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));

        let prev_positions = vec![(100.0, 100.0), (200.0, 200.0)];

        let mut scene = Scene::new();
        let renderer = BarRenderer { axis: BarAxis::Y };
        // Default impl should forward to render()
        renderer.render_interpolated(&mut scene, &batch, &cm, &scales, &prev_positions, 0.5, None);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "bar default render_interpolated should forward to render"
        );
    }

    // -----------------------------------------------------------------------
    // RectRenderer
    // -----------------------------------------------------------------------

    /// rectY: three x-binned bars from a zero y baseline. Proves (a) one fill per
    /// row and (b) the load-bearing mechanism — augment_scales synthesizes a
    /// single shared Channel::X Linear scale spanning [min(x1), max(x2)], which
    /// infer_scales never builds (there is no bare x column).
    #[test]
    fn recty_one_fill_per_row_and_synthesizes_shared_x_scale() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x1", DataType::Float64, false),
            Field::new("x2", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0])),
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X1, "x1".to_string());
        cm.insert(Channel::X2, "x2".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let (xr, yr) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, xr, yr);
        assert!(
            scales.get(Channel::X).is_none(),
            "no bare Channel::X before augment (only X1/X2 from the columns)"
        );

        let renderer = RectRenderer { kind: RectKind::Y };
        renderer.augment_scales(&mut scales, &batch, &cm, xr, yr);
        match scales.get(Channel::X) {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                ..
            }) => {
                assert_eq!(*domain_min, 0.0, "shared X domain min = min(x1)");
                assert_eq!(*domain_max, 3.0, "shared X domain max = max(x2)");
            }
            other => panic!("expected synthesized Linear X scale, got {other:?}"),
        }
        assert_eq!(
            renderer.zero_baseline_channel(),
            Some(Channel::Y),
            "rectY baselines on the y value axis"
        );

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(count_scene_paths(&scene), 3, "one fill per x-bin");
    }

    /// rectX: y-binned horizontal bars from a zero x baseline. Mirror of rectY —
    /// augment synthesizes the shared Channel::Y scale, baseline is on X.
    #[test]
    fn rectx_synthesizes_shared_y_scale_and_baselines_on_x() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("y1", DataType::Float64, false),
            Field::new("y2", DataType::Float64, false),
            Field::new("x", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 2.0])),
                Arc::new(Float64Array::from(vec![2.0, 5.0])),
                Arc::new(Float64Array::from(vec![10.0, 30.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::Y1, "y1".to_string());
        cm.insert(Channel::Y2, "y2".to_string());
        cm.insert(Channel::X, "x".to_string());

        let (xr, yr) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, xr, yr);
        let renderer = RectRenderer { kind: RectKind::X };
        renderer.augment_scales(&mut scales, &batch, &cm, xr, yr);
        match scales.get(Channel::Y) {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                ..
            }) => {
                assert_eq!(*domain_min, 0.0);
                assert_eq!(*domain_max, 5.0);
            }
            other => panic!("expected synthesized Linear Y scale, got {other:?}"),
        }
        assert_eq!(renderer.zero_baseline_channel(), Some(Channel::X));

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(count_scene_paths(&scene), 2, "one fill per y-bin");
    }

    /// Bare `rect`: both axes ranged (x1/x2 × y1/y2). augment synthesizes BOTH
    /// shared scales; no zero baseline.
    #[test]
    fn rect_xy_both_axes_ranged() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x1", DataType::Float64, false),
            Field::new("x2", DataType::Float64, false),
            Field::new("y1", DataType::Float64, false),
            Field::new("y2", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 2.0])),
                Arc::new(Float64Array::from(vec![1.0, 4.0])),
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
                Arc::new(Float64Array::from(vec![3.0, 6.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X1, "x1".to_string());
        cm.insert(Channel::X2, "x2".to_string());
        cm.insert(Channel::Y1, "y1".to_string());
        cm.insert(Channel::Y2, "y2".to_string());

        let (xr, yr) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, xr, yr);
        let renderer = RectRenderer { kind: RectKind::Xy };
        renderer.augment_scales(&mut scales, &batch, &cm, xr, yr);
        assert!(scales.get(Channel::X).is_some(), "shared X synthesized");
        assert!(scales.get(Channel::Y).is_some(), "shared Y synthesized");
        assert_eq!(
            renderer.zero_baseline_channel(),
            None,
            "bare rect has no baseline"
        );

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(count_scene_paths(&scene), 2);
    }

    /// A zero-width bin (x1 == x2) and a null endpoint are skipped, never
    /// panicking — only the one valid bin draws.
    #[test]
    fn rect_skips_degenerate_and_null_rows() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x1", DataType::Float64, true),
            Field::new("x2", DataType::Float64, true),
            Field::new("y", DataType::Float64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![Some(0.0), Some(1.0), Some(2.0)])),
                // row 1: x1==x2 (zero width); row 2: null upper edge.
                Arc::new(Float64Array::from(vec![Some(1.0), Some(1.0), None])),
                Arc::new(Float64Array::from(vec![Some(10.0), Some(20.0), Some(15.0)])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X1, "x1".to_string());
        cm.insert(Channel::X2, "x2".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let (xr, yr) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, xr, yr);
        let renderer = RectRenderer { kind: RectKind::Y };
        renderer.augment_scales(&mut scales, &batch, &cm, xr, yr);

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            1,
            "only the single well-formed bin draws (zero-width + null skipped)"
        );
    }

    /// A categorical fill column colours each rect — still one fill per row.
    #[test]
    fn rect_categorical_fill() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x1", DataType::Float64, false),
            Field::new("x2", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("g", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 1.0])),
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
                Arc::new(StringArray::from(vec!["a", "b"])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X1, "x1".to_string());
        cm.insert(Channel::X2, "x2".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "g".to_string());

        let (xr, yr) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, xr, yr);
        let renderer = RectRenderer { kind: RectKind::Y };
        renderer.augment_scales(&mut scales, &batch, &cm, xr, yr);

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            2,
            "one filled rect per row, coloured by g"
        );
    }

    /// A genuine (non-null) NaN in both x-edges of a row is dropped, not handed
    /// to Vello as malformed geometry; the finite rows still draw. `columns_extent`
    /// ignores NaN, so the synthesized scale stays finite.
    #[test]
    fn rect_skips_non_finite_edges() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x1", DataType::Float64, false),
            Field::new("x2", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, f64::NAN, 2.0])),
                Arc::new(Float64Array::from(vec![1.0, f64::NAN, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X1, "x1".to_string());
        cm.insert(Channel::X2, "x2".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let (xr, yr) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, xr, yr);
        let renderer = RectRenderer { kind: RectKind::Y };
        renderer.augment_scales(&mut scales, &batch, &cm, xr, yr);
        match scales.get(Channel::X) {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                ..
            }) => {
                assert!(domain_min.is_finite() && domain_max.is_finite());
                assert_eq!(*domain_min, 0.0);
                assert_eq!(*domain_max, 3.0);
            }
            other => panic!("expected finite Linear X scale, got {other:?}"),
        }

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        assert_eq!(
            count_scene_paths(&scene),
            2,
            "the NaN-edged row is dropped; the two finite bins draw"
        );
    }

    // -----------------------------------------------------------------------
    // design phase 4 PR B — NULL ink: a genuinely-NULL fill value renders the
    // reserved warm-gray ChartInk::LIGHT.null, never a scheme colour and never the default
    // mark colour (the NULL-reads-as-high bug, booked as the
    // NULL-numeric-fill chore).
    // -----------------------------------------------------------------------

    /// The old Tableau10 blue the bug used to paint NULL cells with — pinned
    /// here so the regression assertions can prove it is gone.
    const OLD_STEELBLUE: [f32; 4] = [0.306, 0.475, 0.655, 1.0];

    #[test]
    fn dsb_null_numeric_fill_renders_null_ink_on_cell() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Utf8, false),
            Field::new("y", DataType::Utf8, false),
            Field::new("v", DataType::Float64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(StringArray::from(vec!["u", "u"])),
                Arc::new(Float64Array::from(vec![Some(10.0), None])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "v".to_string());

        let (xr, yr) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, xr, yr);
        let renderer = CellRenderer::default();
        renderer.augment_scales(&mut scales, &batch, &cm, xr, yr);
        let ramp = scales.get(Channel::Fill).expect("fill ramp built");
        let ramp_at_10 = ramp.map_continuous(10.0);

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn,
            std::collections::HashSet::from([
                packed(ramp_at_10),
                packed(ChartInk::LIGHT.null.components),
            ]),
            "the valued cell samples the ramp; the NULL cell renders NULL ink"
        );
        assert!(
            !drawn.contains(&packed(OLD_STEELBLUE)),
            "the NULL cell no longer paints the old steelblue default"
        );
        assert!(
            !drawn.contains(&packed(ChartInk::LIGHT.mark_default.components)),
            "the NULL cell does not read as the default mark colour either"
        );
    }

    #[test]
    fn dsb_null_categorical_fill_renders_null_ink_on_dot() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("cat", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(Float64Array::from(vec![1.0, 2.0])),
                Arc::new(StringArray::from(vec![Some("a"), None])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "cat".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        let slot1 = scales
            .get(Channel::Fill)
            .and_then(|s| s.map_colour("a"))
            .expect("categorical fill scale built");

        let mut scene = Scene::new();
        DotRenderer.render(&mut scene, &batch, &cm, &scales, None);
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn,
            std::collections::HashSet::from([
                packed(slot1),
                packed(ChartInk::LIGHT.null.components)
            ]),
            "the categorised dot takes its palette slot; the NULL-category dot renders NULL ink"
        );
    }

    #[test]
    fn dsb_null_fill_renders_null_ink_on_hexbin() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("m", DataType::Float64, true),
            Field::new(HEX_DX_COL, DataType::Float64, false),
            Field::new(HEX_DY_COL, DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![0.0, 10.0])),
                Arc::new(Float64Array::from(vec![0.0, 10.0])),
                Arc::new(Float64Array::from(vec![Some(5.0), None])),
                Arc::new(Float64Array::from(vec![1.0, 1.0])),
                Arc::new(Float64Array::from(vec![1.0, 1.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        cm.insert(Channel::Fill, "m".to_string());

        let (xr, yr) = ((40.0, 600.0), (450.0, 20.0));
        let mut scales = infer_scales(&batch, &cm, xr, yr);
        let renderer = HexbinRenderer::default();
        renderer.augment_scales(&mut scales, &batch, &cm, xr, yr);

        let mut scene = Scene::new();
        renderer.render(&mut scene, &batch, &cm, &scales, None);
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert!(
            drawn.contains(&packed(ChartInk::LIGHT.null.components)),
            "the NULL-metric hex renders NULL ink"
        );
        assert!(
            !drawn.contains(&packed(ChartInk::LIGHT.mark_default.components))
                && !drawn.contains(&packed(OLD_STEELBLUE)),
            "the NULL-metric hex renders neither the default mark colour nor old steelblue"
        );
    }

    /// Negative control: a mark with NO fill channel keeps the default mark
    /// colour — NULL ink is reserved for a bound fill whose VALUE is NULL.
    #[test]
    fn dsb_no_fill_channel_keeps_default_colour() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0])),
                Arc::new(Float64Array::from(vec![2.0])),
            ],
        )
        .unwrap();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());
        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (450.0, 20.0));
        let mut scene = Scene::new();
        DotRenderer.render(&mut scene, &batch, &cm, &scales, None);
        let drawn: std::collections::HashSet<u32> =
            scene.encoding().draw_data.iter().copied().collect();
        assert_eq!(
            drawn,
            std::collections::HashSet::from([packed(ChartInk::LIGHT.mark_default.components)]),
            "no fill channel → the (Harbour slot 1) default mark colour, not NULL ink"
        );
    }
}
