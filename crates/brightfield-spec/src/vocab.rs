//! Registry-backed vocabulary for the Mosaic 0.24.x spec (Option Z).
//!
//! Every mark, interactor, input, and component name a Mosaic spec may use
//! is enumerated here. Each variant carries an implementation status —
//! `Implemented | Planned | Unimplemented` — so the preflight `SupportReport`
//! can walk a parsed AST and report which specs exercise
//! vocabulary brightfield does not yet render without having to second-guess.
//!
//! A name that is not present in the registry at all is a hard
//! `ParseError::UnknownName`; a name that is present but not marked
//! `Implemented` is a `ParseWarning::Unimplemented` + AST stub.

use std::fmt;

/// Implementation status of a vocabulary entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplStatus {
    /// brightfield renders this fully.
    Implemented,
    /// brightfield intends to render this; the card is in flight or queued.
    Planned,
    /// brightfield does not yet render this. Parsing stubs the node.
    Unimplemented,
}

impl fmt::Display for ImplStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Implemented => "implemented",
            Self::Planned => "planned",
            Self::Unimplemented => "unimplemented",
        })
    }
}

/// Helper macro: declare an exhaustive enum of vocabulary names keyed by
/// their wire representation, with a pair of `name()` / `from_wire()` helpers
/// and a `status()` method returning `ImplStatus`.
macro_rules! vocab_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident => ($wire:literal, $status:ident)
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name {
            $(
                $(#[$vmeta])*
                $variant,
            )+
        }

        impl $name {
            /// Canonical wire representation (as it appears in YAML/JSON).
            #[must_use]
            pub fn wire_name(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire, )+
                }
            }

            /// Look up by wire representation. Returns `None` for names not in
            /// the registry — the parser promotes that to
            /// `ParseError::UnknownName`.
            #[must_use]
            pub fn from_wire(wire: &str) -> Option<Self> {
                match wire {
                    $( $wire => Some(Self::$variant), )+
                    _ => None,
                }
            }

            /// Implementation status of this entry.
            #[must_use]
            pub fn status(self) -> ImplStatus {
                match self {
                    $( Self::$variant => ImplStatus::$status, )+
                }
            }

            /// All variants, in declaration order.
            #[must_use]
            pub fn all() -> &'static [Self] {
                &[ $( Self::$variant, )+ ]
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.wire_name())
            }
        }
    };
}

vocab_enum! {
    /// Known mark kinds from Mosaic 0.24.x. Staying true to the spec, every
    /// variant is declared Unimplemented until a downstream card wires up a
    /// renderer for it.
    pub enum MarkKind {
        // Area / bar / column
        AreaY => ("areaY", Implemented),
        AreaX => ("areaX", Implemented),
        BarY => ("barY", Implemented),
        BarX => ("barX", Implemented),
        // Cells
        // cell v1 wired end-to-end (density marks, 2026-07-06):
        // pre-aggregated categorical x × categorical y with a numeric fill
        // through a Sequential ramp (Utf8 fill keeps the Colour path).
        // cellX/cellY and the self-aggregating fill: count form stay
        // Unimplemented (deferred with hexbin).
        Cell => ("cell", Implemented),
        CellX => ("cellX", Unimplemented),
        CellY => ("cellY", Unimplemented),
        // Dots / circles
        Dot => ("dot", Implemented),
        DotX => ("dotX", Unimplemented),
        DotY => ("dotY", Unimplemented),
        Circle => ("circle", Unimplemented),
        // Lines
        Line => ("line", Implemented),
        LineX => ("lineX", Unimplemented),
        LineY => ("lineY", Unimplemented),
        // Rectangles
        // rect/rectX/rectY wired end-to-end (2026-07-03): RectRenderer
        // draws rectangles from explicit x1/x2 × y1/y2 extents (bare `rect`), or a
        // ranged edge + zero-baselined value (rectX/rectY numeric-edged bars).
        Rect => ("rect", Implemented),
        RectX => ("rectX", Implemented),
        RectY => ("rectY", Implemented),
        // Rules / ticks
        Rule => ("rule", Unimplemented),
        RuleX => ("ruleX", Implemented),
        RuleY => ("ruleY", Implemented),
        TickX => ("tickX", Unimplemented),
        TickY => ("tickY", Unimplemented),
        // Text
        Text => ("text", Implemented),
        TextX => ("textX", Unimplemented),
        TextY => ("textY", Unimplemented),
        // Links / vectors / arrows
        Link => ("link", Unimplemented),
        Vector => ("vector", Unimplemented),
        VectorX => ("vectorX", Unimplemented),
        VectorY => ("vectorY", Unimplemented),
        Arrow => ("arrow", Unimplemented),
        // Density / heatmap / contour / raster
        Density => ("density", Implemented),
        DensityX => ("densityX", Implemented),
        DensityY => ("densityY", Implemented),
        DenseLine => ("denseLine", Unimplemented),
        // heatmap/contour wired end-to-end (density marks,
        // 2026-07-06): both ride the 2D density lowerer's binned grid —
        // heatmap ramps the KDE-smoothed field per cell, contour traces
        // marching-squares iso-lines over it (thresholds = iso-level count,
        // shielded from the SQL bin count at lowerer registration).
        Heatmap => ("heatmap", Implemented),
        Contour => ("contour", Implemented),
        // Binned 2D count heatmap — filled cells coloured (by alpha) per bin
        // count, reusing the 2D density binning. (mark breadth)
        Raster => ("raster", Implemented),
        // Hex — hexbin (pixel-space cube-round binning, self-aggregating fill)
        // and hexgrid (decorative dataless mesh) wired end-to-end in the
        // hexbin follow-up.
        Hexbin => ("hexbin", Implemented),
        Hexgrid => ("hexgrid", Implemented),
        // Waffle
        WaffleX => ("waffleX", Unimplemented),
        WaffleY => ("waffleY", Unimplemented),
        // Regression / error
        RegressionY => ("regressionY", Implemented),
        RegressionX => ("regressionX", Implemented),
        ErrorbarX => ("errorbarX", Unimplemented),
        ErrorbarY => ("errorbarY", Unimplemented),
        // Frame / axis / grid
        Frame => ("frame", Unimplemented),
        AxisX => ("axisX", Unimplemented),
        AxisY => ("axisY", Unimplemented),
        AxisFx => ("axisFx", Unimplemented),
        AxisFy => ("axisFy", Unimplemented),
        GridX => ("gridX", Unimplemented),
        GridY => ("gridY", Unimplemented),
        GridFx => ("gridFx", Unimplemented),
        GridFy => ("gridFy", Unimplemented),
        // Geo — projected GeoJSON basemap / choropleth (last mark).
        // GeoLowerer (near-clone of SimpleLowerer + ST_AsGeoJSON on a spatial
        // geometry column) feeds a render-side GeoRenderer that projects each
        // vertex client-side (equirectangular / US-tuned Albers) and draws one
        // BezPath per feature. Sphere/graticule stay Unimplemented (deferred
        // globe companions).
        Geo => ("geo", Implemented),
        Sphere => ("sphere", Unimplemented),
        Graticule => ("graticule", Unimplemented),
        // Voronoi / delaunay / hull. Voronoi is the always-unimplemented census
        // stand-in (geo's former role) — genuinely far off, no lowerer/renderer.
        Voronoi => ("voronoi", Unimplemented),
        VoronoiMesh => ("voronoiMesh", Unimplemented),
        DelaunayMesh => ("delaunayMesh", Unimplemented),
        DelaunayLink => ("delaunayLink", Unimplemented),
        Hull => ("hull", Unimplemented),
        // Image
        Image => ("image", Unimplemented),
    }
}

impl MarkKind {
    /// Whether this mark kind computes a **positional** `bin` + `count` pair —
    /// `x: {bin: col}` with `y: {count:}` and its transpose.
    ///
    /// The one list the lift consults, so a spec cannot parse as binned and
    /// then lower as `SELECT *`: the parser lifts the pair only for a kind
    /// named here, and `brightfield-sql` registers `RectLowerer` for both.
    ///
    /// **Plain `rect` is deliberately absent.** `RectLowerer` is *registered*
    /// for it too — every kind needs a lowerer, and it delegates to
    /// `SimpleLowerer` when there is no positional bin — but it must not lift:
    /// the renderer's `RectKind::Xy` needs **both** `x1`/`x2` and `y1`/`y2`,
    /// while a bin+count binds one interval pair only, so the mark returns
    /// before drawing. Listing `Rect` here silenced its uncomputed-transform
    /// diagnostic and put nothing on the page in its place.
    ///
    /// So a kind belongs here only once something downstream **draws** what it
    /// produces. Quieting the diagnostic is the easy half.
    #[must_use]
    pub fn bins_positionally(self) -> bool {
        matches!(self, Self::RectX | Self::RectY)
    }

    /// The `(value, band)` positional channel pair this mark aggregates over,
    /// or `None` for a kind that has no band axis.
    ///
    /// A `barX` draws horizontal bars: the length is on `x` and the category
    /// runs down `y`, so `x: {sum: gold}` with `y: nationality` is one bar per
    /// nationality whose length is that nationality's total. `barY` is the
    /// transpose. The orientation comes from the KIND rather than from which
    /// channel happens to carry the map, so `mark: barX, x: nationality,
    /// y: {sum: gold}` — an aggregate on the band axis — is refused rather
    /// than silently drawn sideways.
    ///
    /// The same contract as [`Self::bins_positionally`]: this is the one list
    /// the lift consults, and `brightfield-sql` registers `BarLowerer` for
    /// exactly these kinds, so a spec cannot parse as aggregating and then
    /// lower as `SELECT *`. **A kind belongs here only once something
    /// downstream draws what it produces** — quieting the
    /// uncomputed-transform diagnostic is the easy half, and on its own it
    /// trades a true report for a blank frame.
    #[must_use]
    pub fn band_aggregate_axes(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::BarX => Some(("x", "y")),
            Self::BarY => Some(("y", "x")),
            _ => None,
        }
    }
}

vocab_enum! {
    /// Known interactor kinds used as a plot item's `select:` discriminator.
    ///
    /// Note: the same wire key `select:` is also used inside a `params:` entry
    /// to declare a Selection's resolution (`crossfilter`, `intersect`,
    /// `single`, `union`). Those names are modelled by
    /// [`SelectionResolution`], not here.
    pub enum InteractorKind {
        IntervalX => ("intervalX", Implemented),
        IntervalY => ("intervalY", Implemented),
        IntervalXY => ("intervalXY", Implemented),
        Interval => ("interval", Unimplemented),
        // Demoted to Unimplemented (harden, 2026-07-02): parsed but unwired —
        // `find_nearest` has no production caller and hover resolves `nearest:
        // None`. See 2026-07-02-interactor-status-demotion.md.
        Nearest => ("nearest", Unimplemented),
        NearestX => ("nearestX", Unimplemented),
        NearestY => ("nearestY", Unimplemented),
        // toggleX/toggleY wired end-to-end (2026-07-03): each becomes
        // a single-channel point selection (BrushKind::PointX/PointY) that drives
        // an equality predicate through propagate_selection. `toggle` (both axes)
        // stays Unimplemented until its value-pair producer + click gesture land.
        Toggle => ("toggle", Unimplemented),
        ToggleX => ("toggleX", Implemented),
        ToggleY => ("toggleY", Implemented),
        Highlight => ("highlight", Implemented),
        Region => ("region", Unimplemented),
        // Demoted to Unimplemented (harden, 2026-07-02): parsed but unwired —
        // apply_pan/apply_zoom/ChartState::set_navigation have no production
        // caller (no scroll/wheel handler; NavigationState is always None).
        Pan => ("pan", Unimplemented),
        PanX => ("panX", Unimplemented),
        PanY => ("panY", Unimplemented),
        PanZoom => ("panZoom", Unimplemented),
        PanZoomX => ("panZoomX", Unimplemented),
        PanZoomY => ("panZoomY", Unimplemented),
    }
}

vocab_enum! {
    /// Known input widget kinds (the `input:` discriminator).
    pub enum InputKind {
        // Wired end-to-end (2026-07-17): a hosted MenuElement drives
        // its param via commit_menu → propagate_param → re-render, with radio
        // and checkbox as `style:` presentations of menu (options-bag key — NO
        // new vocabulary; `input: radio`/`input: checkbox` stay UnknownName).
        Menu => ("menu", Implemented),
        Search => ("search", Unimplemented),
        // Wired end-to-end (2026-07-03): a hosted SliderElement drives
        // its param via commit_slider → propagate_param → re-render. Search/
        // Table remain Unimplemented (no widget yet).
        Slider => ("slider", Implemented),
        Table => ("table", Unimplemented),
    }
}

vocab_enum! {
    /// Known composition-level component kinds other than [`MarkKind`] /
    /// [`InteractorKind`] / [`InputKind`]. These are the layout and
    /// legend-as-component forms.
    pub enum ComponentKind {
        // The five layout forms all render end-to-end, and have since the
        // multi-view composite landed. `compute_layout` gives each a rect;
        // `placed_plots` — the one call BOTH the live window (pipeline's
        // `compose`) and the headless composite take — walks that tree, so a
        // plot's position on the page IS the layout implementation. Leaving
        // these Unimplemented made preflight declare every spec in the corpus
        // unrenderable, including the ones the product ships as its own
        // examples, which is the opposite of the honesty the report exists for.
        //
        // Evidence, per form: `examples/dashboard.yaml` (hconcat of two plots)
        // and `examples/param-slider.yaml` (vconcat) compose in
        // `examples_exercise.rs`; `examples/layout-spacer.yaml` places an
        // `hspace: 64` between two plots and `hspace_offsets_subsequent_plot`
        // pins the 64px offset through `placed_plots`;
        // `vspace_offsets_subsequent_plot` pins the vspace twin. A spacer
        // renders no ink by definition — reserving the gap is the whole of its
        // implementation, and the gap is reserved.
        Plot => ("plot", Implemented),
        HConcat => ("hconcat", Implemented),
        VConcat => ("vconcat", Implemented),
        HSpace => ("hspace", Implemented),
        VSpace => ("vspace", Implemented),
        // Standalone `legend:` nodes render end-to-end: resolved to
        // their `for:` plot's colour scale, drawn into the headless composite
        // AND hosted in the window as a display-only LegendElement at the same
        // layout rect.
        Legend => ("legend", Implemented),
    }
}

vocab_enum! {
    /// Selection-resolution kinds as they appear under a `params.<name>:
    /// { select: <resolution> }` declaration. All four are implemented at the
    /// SQL-emit layer — `compile_selection` (brightfield-sql) gives each a
    /// distinct predicate combination (crossfilter self-excludes then ANDs,
    /// intersect ANDs, union ORs, single keeps the most recent) and they are
    /// runtime-tested.
    pub enum SelectionResolution {
        Crossfilter => ("crossfilter", Implemented),
        Intersect => ("intersect", Implemented),
        Single => ("single", Implemented),
        Union => ("union", Implemented),
    }
}

vocab_enum! {
    /// Known legend channels. Keyed to the `legend:` discriminator value.
    pub enum LegendChannel {
        // A standalone `legend: color` renders its `for:` plot's colour scale as
        // swatches at the legend's layout rect (multi-view inc 6, headless
        // composite). Opacity/symbol legends have no renderer yet.
        Color => ("color", Implemented),
        Opacity => ("opacity", Unimplemented),
        Symbol => ("symbol", Unimplemented),
    }
}

/// Every mark option key some lowerer or renderer actually reads.
///
/// A Mosaic mark carries an open bag of option keys, and brightfield's
/// pipeline reads a small, nameable subset of it. Anything outside this list
/// is parsed, held in the AST, serialised back out faithfully — and then
/// **silently ignored** at render time. That silence is the defect this list
/// exists to end: a spec that says `curve: monotone-x` and gets straight
/// segments has been lied to, and only the reader could tell.
///
/// The list is hand-maintained against the consumers, ONE ENTRY PER READER,
/// because the readers live in crates this one cannot see (a spec crate that
/// depended on the renderer would be the dependency arrow pointing the wrong
/// way). Adding a consumer means adding its key here, and
/// `every_consumed_mark_option_key_is_named_once` keeps the list from growing
/// duplicates.
///
/// **The two ways this list goes wrong are not symmetric, and the dangerous
/// one is omission.** [`mark_option_is_consumed`] returns `false` for anything
/// not named here, and the parser warns on exactly that — so a key that IS
/// read but is missing from the list produces a diagnostic telling an author
/// their working instruction has no effect. That is a **false** diagnostic,
/// and it is the one failure an honesty channel cannot survive: a reader who
/// catches this channel lying once stops believing the true ones. The other
/// error — a key listed here whose reader was since removed — costs only a
/// missing diagnostic, which is where this list started.
///
/// So: **when in doubt about a key, go and check.** `rg` the key across the
/// lowerers and the renderer, and name the reader in the section below. Do
/// not leave it off "to be safe" — off is the unsafe side.
///
/// Where each is read:
///
/// - `x` `y` `x1` `y1` `x2` `y2` `fill` `stroke` `size` `text` — the visual
///   encoding channels. `brightfield_render::channel::ChannelMap::from_mark`
///   maps each to a column, a numeric literal, or an aggregate output column;
///   `brightfield_sql`'s lowerer additionally projects the positional six
///   when they are bound to a `$param`.
/// - `filterBy` — read by `brightfield_spec::analysis` when a mark declares
///   the filter at mark level rather than inside its `data:` block.
/// - `type` — the density lowerer's kernel/estimator discriminator.
/// - `thresholds` `bins` `binWidth` `bandwidth` — the density / hexbin
///   binning knobs, read by the lowerer and threaded back into the renderer
///   on a colour-scheme rebuild.
/// - `geometry` — the geo mark's geometry column, resolved in
///   `brightfield_spec::layout`.
/// - `label` — the number a bar prints on itself, read by
///   `brightfield_render::channel::ChannelMap::from_mark` into a
///   `brightfield_render::channel::LabelForm` and drawn by `BarRenderer`.
///   Brightfield's own key, not Mosaic's: upstream has no in-bar label on a
///   bar mark, and the count plots that show one draw it with a template
///   renderer outside the spec language. The word is Mosaic's for the same
///   idea on an input (`input: menu, label: Sport`), which is why it is
///   already in the parser's lift surface.
/// - `sort` — the band channel's order and row cap, read by
///   `brightfield-sql`'s `BarLowerer` for the kinds
///   [`MarkKind::band_aggregate_axes`] names. The `sort:` shapes that lowerer
///   does not compute are reported by
///   [`ParseWarning::UnconsumedSort`](crate::parse::ParseWarning::UnconsumedSort)
///   rather than by this list, for the same reason `x` is listed here while
///   the transforms sitting on it are reported separately.
/// - `aspectRatio` — read by `brightfield_render::channel::ChannelMap::from_mark`
///   into `ChannelMap::equal_aspect`, and honoured by
///   `brightfield_render::mark::DotRenderer::augment_scales`, which widens the
///   narrower of a `dot` mark's two positional domains until both axes share
///   one px-per-unit. Real Observable Plot's `aspectRatio` is a plot-level
///   option; this build reads it per mark instead, because a mark option
///   reaches `ChannelMap::from_mark` with no further wiring, while a new
///   plot-level attribute would need threading through each
///   renderer-construction call site. The value `1` is the one read; any
///   other value is left unbound, the same silence an unrecognised `label:`
///   value gets.
pub const CONSUMED_MARK_OPTION_KEYS: &[&str] = &[
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "fill",
    "stroke",
    "size",
    "text",
    "filterBy",
    "type",
    "thresholds",
    "bins",
    "binWidth",
    "bandwidth",
    "geometry",
    "sort",
    "label",
    "aspectRatio",
];

/// Whether a mark option key reaches a lowerer or a renderer.
///
/// `false` means the key is carried through the AST intact and then dropped
/// on the floor — see [`CONSUMED_MARK_OPTION_KEYS`].
#[must_use]
pub fn mark_option_is_consumed(key: &str) -> bool {
    CONSUMED_MARK_OPTION_KEYS.contains(&key)
}

/// The CSS Color Module Level 4 named colours **and their sRGB values**, plus
/// the three keywords a spec may write in a colour slot (`none`,
/// `transparent`, `currentColor`). Sorted by name, so [`is_colour_literal`] and
/// [`css_colour_keyword_rgb`] can binary-search it.
///
/// A fixed, standardised vocabulary — it does not grow with brightfield.
///
/// `None` marks the three entries that name **no** sRGB triple:
///
/// - `none` and `transparent` paint nothing. They are a colour-slot value, not
///   a colour, so the byte triple would be a fiction; the caller decides what
///   "paint nothing" means for the channel it is resolving.
/// - `currentColor` resolves against an inherited text colour, which comes out
///   of a CSS cascade. There is no cascade here and no inherited ink to read,
///   so this table has nothing honest to return for it — see
///   [`css_colour_keyword_rgb`].
///
/// Adding the values did not change the name set: it is byte-identical to the
/// one this table carried before.
const CSS_COLOUR_KEYWORDS: &[(&str, Option<[u8; 3]>)] = &[
    ("aliceblue", Some([0xf0, 0xf8, 0xff])),
    ("antiquewhite", Some([0xfa, 0xeb, 0xd7])),
    ("aqua", Some([0x00, 0xff, 0xff])),
    ("aquamarine", Some([0x7f, 0xff, 0xd4])),
    ("azure", Some([0xf0, 0xff, 0xff])),
    ("beige", Some([0xf5, 0xf5, 0xdc])),
    ("bisque", Some([0xff, 0xe4, 0xc4])),
    ("black", Some([0x00, 0x00, 0x00])),
    ("blanchedalmond", Some([0xff, 0xeb, 0xcd])),
    ("blue", Some([0x00, 0x00, 0xff])),
    ("blueviolet", Some([0x8a, 0x2b, 0xe2])),
    ("brown", Some([0xa5, 0x2a, 0x2a])),
    ("burlywood", Some([0xde, 0xb8, 0x87])),
    ("cadetblue", Some([0x5f, 0x9e, 0xa0])),
    ("chartreuse", Some([0x7f, 0xff, 0x00])),
    ("chocolate", Some([0xd2, 0x69, 0x1e])),
    ("coral", Some([0xff, 0x7f, 0x50])),
    ("cornflowerblue", Some([0x64, 0x95, 0xed])),
    ("cornsilk", Some([0xff, 0xf8, 0xdc])),
    ("crimson", Some([0xdc, 0x14, 0x3c])),
    ("currentcolor", None),
    ("cyan", Some([0x00, 0xff, 0xff])),
    ("darkblue", Some([0x00, 0x00, 0x8b])),
    ("darkcyan", Some([0x00, 0x8b, 0x8b])),
    ("darkgoldenrod", Some([0xb8, 0x86, 0x0b])),
    ("darkgray", Some([0xa9, 0xa9, 0xa9])),
    ("darkgreen", Some([0x00, 0x64, 0x00])),
    ("darkgrey", Some([0xa9, 0xa9, 0xa9])),
    ("darkkhaki", Some([0xbd, 0xb7, 0x6b])),
    ("darkmagenta", Some([0x8b, 0x00, 0x8b])),
    ("darkolivegreen", Some([0x55, 0x6b, 0x2f])),
    ("darkorange", Some([0xff, 0x8c, 0x00])),
    ("darkorchid", Some([0x99, 0x32, 0xcc])),
    ("darkred", Some([0x8b, 0x00, 0x00])),
    ("darksalmon", Some([0xe9, 0x96, 0x7a])),
    ("darkseagreen", Some([0x8f, 0xbc, 0x8f])),
    ("darkslateblue", Some([0x48, 0x3d, 0x8b])),
    ("darkslategray", Some([0x2f, 0x4f, 0x4f])),
    ("darkslategrey", Some([0x2f, 0x4f, 0x4f])),
    ("darkturquoise", Some([0x00, 0xce, 0xd1])),
    ("darkviolet", Some([0x94, 0x00, 0xd3])),
    ("deeppink", Some([0xff, 0x14, 0x93])),
    ("deepskyblue", Some([0x00, 0xbf, 0xff])),
    ("dimgray", Some([0x69, 0x69, 0x69])),
    ("dimgrey", Some([0x69, 0x69, 0x69])),
    ("dodgerblue", Some([0x1e, 0x90, 0xff])),
    ("firebrick", Some([0xb2, 0x22, 0x22])),
    ("floralwhite", Some([0xff, 0xfa, 0xf0])),
    ("forestgreen", Some([0x22, 0x8b, 0x22])),
    ("fuchsia", Some([0xff, 0x00, 0xff])),
    ("gainsboro", Some([0xdc, 0xdc, 0xdc])),
    ("ghostwhite", Some([0xf8, 0xf8, 0xff])),
    ("gold", Some([0xff, 0xd7, 0x00])),
    ("goldenrod", Some([0xda, 0xa5, 0x20])),
    ("gray", Some([0x80, 0x80, 0x80])),
    ("green", Some([0x00, 0x80, 0x00])),
    ("greenyellow", Some([0xad, 0xff, 0x2f])),
    ("grey", Some([0x80, 0x80, 0x80])),
    ("honeydew", Some([0xf0, 0xff, 0xf0])),
    ("hotpink", Some([0xff, 0x69, 0xb4])),
    ("indianred", Some([0xcd, 0x5c, 0x5c])),
    ("indigo", Some([0x4b, 0x00, 0x82])),
    ("ivory", Some([0xff, 0xff, 0xf0])),
    ("khaki", Some([0xf0, 0xe6, 0x8c])),
    ("lavender", Some([0xe6, 0xe6, 0xfa])),
    ("lavenderblush", Some([0xff, 0xf0, 0xf5])),
    ("lawngreen", Some([0x7c, 0xfc, 0x00])),
    ("lemonchiffon", Some([0xff, 0xfa, 0xcd])),
    ("lightblue", Some([0xad, 0xd8, 0xe6])),
    ("lightcoral", Some([0xf0, 0x80, 0x80])),
    ("lightcyan", Some([0xe0, 0xff, 0xff])),
    ("lightgoldenrodyellow", Some([0xfa, 0xfa, 0xd2])),
    ("lightgray", Some([0xd3, 0xd3, 0xd3])),
    ("lightgreen", Some([0x90, 0xee, 0x90])),
    ("lightgrey", Some([0xd3, 0xd3, 0xd3])),
    ("lightpink", Some([0xff, 0xb6, 0xc1])),
    ("lightsalmon", Some([0xff, 0xa0, 0x7a])),
    ("lightseagreen", Some([0x20, 0xb2, 0xaa])),
    ("lightskyblue", Some([0x87, 0xce, 0xfa])),
    ("lightslategray", Some([0x77, 0x88, 0x99])),
    ("lightslategrey", Some([0x77, 0x88, 0x99])),
    ("lightsteelblue", Some([0xb0, 0xc4, 0xde])),
    ("lightyellow", Some([0xff, 0xff, 0xe0])),
    ("lime", Some([0x00, 0xff, 0x00])),
    ("limegreen", Some([0x32, 0xcd, 0x32])),
    ("linen", Some([0xfa, 0xf0, 0xe6])),
    ("magenta", Some([0xff, 0x00, 0xff])),
    ("maroon", Some([0x80, 0x00, 0x00])),
    ("mediumaquamarine", Some([0x66, 0xcd, 0xaa])),
    ("mediumblue", Some([0x00, 0x00, 0xcd])),
    ("mediumorchid", Some([0xba, 0x55, 0xd3])),
    ("mediumpurple", Some([0x93, 0x70, 0xdb])),
    ("mediumseagreen", Some([0x3c, 0xb3, 0x71])),
    ("mediumslateblue", Some([0x7b, 0x68, 0xee])),
    ("mediumspringgreen", Some([0x00, 0xfa, 0x9a])),
    ("mediumturquoise", Some([0x48, 0xd1, 0xcc])),
    ("mediumvioletred", Some([0xc7, 0x15, 0x85])),
    ("midnightblue", Some([0x19, 0x19, 0x70])),
    ("mintcream", Some([0xf5, 0xff, 0xfa])),
    ("mistyrose", Some([0xff, 0xe4, 0xe1])),
    ("moccasin", Some([0xff, 0xe4, 0xb5])),
    ("navajowhite", Some([0xff, 0xde, 0xad])),
    ("navy", Some([0x00, 0x00, 0x80])),
    ("none", None),
    ("oldlace", Some([0xfd, 0xf5, 0xe6])),
    ("olive", Some([0x80, 0x80, 0x00])),
    ("olivedrab", Some([0x6b, 0x8e, 0x23])),
    ("orange", Some([0xff, 0xa5, 0x00])),
    ("orangered", Some([0xff, 0x45, 0x00])),
    ("orchid", Some([0xda, 0x70, 0xd6])),
    ("palegoldenrod", Some([0xee, 0xe8, 0xaa])),
    ("palegreen", Some([0x98, 0xfb, 0x98])),
    ("paleturquoise", Some([0xaf, 0xee, 0xee])),
    ("palevioletred", Some([0xdb, 0x70, 0x93])),
    ("papayawhip", Some([0xff, 0xef, 0xd5])),
    ("peachpuff", Some([0xff, 0xda, 0xb9])),
    ("peru", Some([0xcd, 0x85, 0x3f])),
    ("pink", Some([0xff, 0xc0, 0xcb])),
    ("plum", Some([0xdd, 0xa0, 0xdd])),
    ("powderblue", Some([0xb0, 0xe0, 0xe6])),
    ("purple", Some([0x80, 0x00, 0x80])),
    ("rebeccapurple", Some([0x66, 0x33, 0x99])),
    ("red", Some([0xff, 0x00, 0x00])),
    ("rosybrown", Some([0xbc, 0x8f, 0x8f])),
    ("royalblue", Some([0x41, 0x69, 0xe1])),
    ("saddlebrown", Some([0x8b, 0x45, 0x13])),
    ("salmon", Some([0xfa, 0x80, 0x72])),
    ("sandybrown", Some([0xf4, 0xa4, 0x60])),
    ("seagreen", Some([0x2e, 0x8b, 0x57])),
    ("seashell", Some([0xff, 0xf5, 0xee])),
    ("sienna", Some([0xa0, 0x52, 0x2d])),
    ("silver", Some([0xc0, 0xc0, 0xc0])),
    ("skyblue", Some([0x87, 0xce, 0xeb])),
    ("slateblue", Some([0x6a, 0x5a, 0xcd])),
    ("slategray", Some([0x70, 0x80, 0x90])),
    ("slategrey", Some([0x70, 0x80, 0x90])),
    ("snow", Some([0xff, 0xfa, 0xfa])),
    ("springgreen", Some([0x00, 0xff, 0x7f])),
    ("steelblue", Some([0x46, 0x82, 0xb4])),
    ("tan", Some([0xd2, 0xb4, 0x8c])),
    ("teal", Some([0x00, 0x80, 0x80])),
    ("thistle", Some([0xd8, 0xbf, 0xd8])),
    ("tomato", Some([0xff, 0x63, 0x47])),
    ("transparent", None),
    ("turquoise", Some([0x40, 0xe0, 0xd0])),
    ("violet", Some([0xee, 0x82, 0xee])),
    ("wheat", Some([0xf5, 0xde, 0xb3])),
    ("white", Some([0xff, 0xff, 0xff])),
    ("whitesmoke", Some([0xf5, 0xf5, 0xf5])),
    ("yellow", Some([0xff, 0xff, 0x00])),
    ("yellowgreen", Some([0x9a, 0xcd, 0x32])),
];

/// Whether a colour-channel string is a **constant colour** rather than a
/// column name — `fill: steelblue` versus `fill: version`.
///
/// A colour channel is overloaded in this spec language — one spec writes both
/// `fill: steelblue` and `fill: weather` and means different things by them —
/// so the string itself is the only discriminator. No upstream source is
/// vendored here to port a keyword table from, so `CSS_COLOUR_KEYWORDS` is
/// the CSS Level 4 list.
///
/// It decides two things. A rect binding a colour CONSTANT carries no groups, so
/// its `bin` + `count` is a plain histogram and can be lifted (see
/// `parse::binned_histogram` for why a grouped one may not be); and
/// `brightfield_render::channel::ChannelMap::from_mark` binds such a string as
/// the mark's constant ink instead of as a column name. Unrecognised ⇒ treated
/// as a column, which is the safe direction: the spec keeps its
/// uncomputed-transform diagnostic instead of drawing a lie.
///
/// **Recognising a literal is not the same as being able to paint it.** This
/// answers "is it a colour, or a column"; [`css_colour_keyword_rgb`] answers
/// "what colour", and returns `None` for keywords this build cannot resolve.
#[must_use]
pub fn is_colour_literal(value: &str) -> bool {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    // `rgb(…)` / `rgba(…)` / `hsl(…)` / `hwb(…)` / `lab(…)` / `color(…)` …
    if v.ends_with(')') && v.contains('(') {
        return true;
    }
    let lower = v.to_ascii_lowercase();
    CSS_COLOUR_KEYWORDS
        .binary_search_by_key(&lower.as_str(), |(name, _)| *name)
        .is_ok()
}

/// What a colour-slot **keyword** paints, as straight-alpha sRGB bytes.
///
/// - A named colour → its sRGB triple, opaque.
/// - `none` / `transparent` → `[0, 0, 0, 0]`, i.e. paint nothing. CSS treats
///   both as the fully transparent black, and so does Observable Plot's colour
///   slot; a mark that asks for no ink gets no ink.
/// - Anything else, **including `currentColor`** → `None`. There is no CSS
///   cascade in this renderer and therefore no inherited text colour to be
///   "current", so any value returned for it would be invented. The caller
///   keeps whatever default it already had, which is the honest fallthrough.
///
/// Case-insensitive (`SteelBlue` is `steelblue`) and whitespace-trimmed, so it
/// accepts exactly the strings [`is_colour_literal`] accepts as keywords.
/// **Hex and functional notation are not handled here** — hex is parsed by
/// `brightfield_render::mark::parse_css_hex`, and `rgb(…)`/`hsl(…)` are
/// unresolved in this build.
#[must_use]
pub fn css_colour_keyword_rgb(value: &str) -> Option<[u8; 4]> {
    let lower = value.trim().to_ascii_lowercase();
    match CSS_COLOUR_KEYWORDS.binary_search_by_key(&lower.as_str(), |(name, _)| *name) {
        Ok(i) => match CSS_COLOUR_KEYWORDS[i] {
            (_, Some([r, g, b])) => Some([r, g, b, 0xff]),
            // `none` and `transparent` paint nothing; `currentColor` has no
            // value here at all and must NOT be confused with them.
            ("none" | "transparent", None) => Some([0, 0, 0, 0]),
            (_, None) => None,
        },
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`is_colour_literal`] binary-searches the keyword table, so an
    /// out-of-order entry would make some colours invisible — and the failure
    /// would look like a stacking bug in a chart, three crates away.
    #[test]
    fn the_colour_keyword_table_is_sorted_and_unique() {
        for pair in CSS_COLOUR_KEYWORDS.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "CSS_COLOUR_KEYWORDS must be sorted and duplicate-free, but \
                 {:?} precedes {:?}",
                pair[0].0,
                pair[1].0
            );
        }
    }

    /// The keyword table is the CSS list and nothing else: exactly three
    /// entries carry no triple, and they are the three colour-slot keywords
    /// that are not colours. A named colour silently landing a `None` here
    /// would make that ONE colour unpaintable while every other keyword worked
    /// — the kind of gap `is_colour_literal` cannot see, because it only asks
    /// whether the name is in the table.
    #[test]
    fn only_the_three_non_colour_keywords_carry_no_value() {
        let valueless: Vec<&str> = CSS_COLOUR_KEYWORDS
            .iter()
            .filter(|(_, rgb)| rgb.is_none())
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(valueless, vec!["currentcolor", "none", "transparent"]);
        assert_eq!(
            CSS_COLOUR_KEYWORDS.len(),
            148 + 3,
            "the CSS Level 4 named-colour list is 148 entries; the other three \
             are `none`, `transparent` and `currentColor`"
        );
    }

    /// The values, spot-checked against the CSS specification — including the
    /// two the corpus actually writes. A transposed or mistyped byte here is
    /// invisible to every other test in this crate, because nothing else in
    /// `brightfield-spec` looks at a colour VALUE.
    #[test]
    fn keywords_resolve_to_their_css_values() {
        for (name, want) in [
            ("steelblue", [0x46, 0x82, 0xb4, 0xff]),
            ("SteelBlue", [0x46, 0x82, 0xb4, 0xff]),
            ("  steelblue  ", [0x46, 0x82, 0xb4, 0xff]),
            ("crimson", [0xdc, 0x14, 0x3c, 0xff]),
            ("gold", [0xff, 0xd7, 0x00, 0xff]),
            ("rebeccapurple", [0x66, 0x33, 0x99, 0xff]),
            ("black", [0x00, 0x00, 0x00, 0xff]),
            ("white", [0xff, 0xff, 0xff, 0xff]),
            // Paints nothing — a colour-slot value, not a colour.
            ("none", [0, 0, 0, 0]),
            ("transparent", [0, 0, 0, 0]),
        ] {
            assert_eq!(
                css_colour_keyword_rgb(name),
                Some(want),
                "`{name}` must resolve to its CSS value"
            );
        }

        // `currentColor` is recognised as a literal and has NO value here: it
        // reads an inherited text colour out of a cascade this renderer does
        // not have. Returning `[0,0,0,0]` for it would silently blank the mark;
        // returning black would invent a colour the spec never named.
        assert!(is_colour_literal("currentColor"));
        assert_eq!(css_colour_keyword_rgb("currentColor"), None);

        // Not keywords. Hex is the hex parser's job and functional notation is
        // unresolved; both must fall through rather than land on a neighbour.
        for other in ["#4682b4", "rgb(70, 130, 180)", "version", "steel blue", ""] {
            assert_eq!(
                css_colour_keyword_rgb(other),
                None,
                "`{other}` is not a colour keyword"
            );
        }
    }

    /// The distinction the binned-rect lift turns on: a colour CONSTANT is not
    /// a grouping, a bare word that is not a colour is a column name.
    #[test]
    fn colour_constants_are_told_from_column_names() {
        for constant in [
            "steelblue",
            "black",
            "SteelBlue",
            "#ccc",
            "#4682b4",
            "#4682b4ff",
            "rgb(70, 130, 180)",
            "currentColor",
            "transparent",
            "none",
        ] {
            assert!(is_colour_literal(constant), "{constant} is a colour");
        }
        for column in ["version", "weather", "delay", "count", "#zzz", "#12345"] {
            assert!(
                !is_colour_literal(column),
                "{column} is not a colour, so a fill bound to it may carry groups"
            );
        }
    }

    /// The list is the two oriented rect kinds, and **`rect` is deliberately
    /// absent** — pinning that is the point of the second assertion. It was
    /// listed once, and drew nothing while its uncomputed-transform diagnostic
    /// was silenced; see [`MarkKind::bins_positionally`] for why.
    #[test]
    fn only_the_oriented_rect_kinds_bin_positionally() {
        let binning: Vec<&str> = MarkKind::all()
            .iter()
            .filter(|k| k.bins_positionally())
            .map(|k| k.wire_name())
            .collect();
        assert_eq!(binning, vec!["rectX", "rectY"]);
        assert!(
            !MarkKind::Rect.bins_positionally(),
            "plain `rect` needs BOTH interval pairs; binning it removes the \
             diagnostic and draws nothing in its place"
        );
    }

    /// The consumed-key list is a registry, and a registry with a duplicate
    /// in it is a registry someone edited without reading.
    #[test]
    fn every_consumed_mark_option_key_is_named_once() {
        let mut seen: Vec<&str> = Vec::new();
        for key in CONSUMED_MARK_OPTION_KEYS {
            assert!(
                !seen.contains(key),
                "{key} is listed twice in CONSUMED_MARK_OPTION_KEYS"
            );
            seen.push(key);
        }
        // The channels the renderer maps are all present — the one subset
        // whose absence would silence a diagnostic on every spec at once.
        for channel in [
            "x", "y", "x1", "y1", "x2", "y2", "fill", "stroke", "size", "text",
        ] {
            assert!(
                mark_option_is_consumed(channel),
                "{channel} is a rendered channel and must be listed as consumed"
            );
        }
        // `sort` joined the list with `BarLowerer`; `limit` did not, and that
        // is not an oversight. A `limit:` brightfield reads is the one nested
        // inside a `sort:` map, never a mark-level key of its own, so listing
        // it here would silence a top-level `limit:` nothing reads.
        assert!(
            mark_option_is_consumed("sort"),
            "`sort` has a reader and must be listed, or its authors are told a \
             working instruction has no effect"
        );
        // And the keys the corpus proved unconsumed stay unconsumed.
        for ignored in ["limit", "curve", "fillOpacity", "r"] {
            assert!(
                !mark_option_is_consumed(ignored),
                "{ignored} has no reader; listing it as consumed would hide a real diagnostic"
            );
        }
    }

    /// verification: every variant of every Kind enum exposes an
    /// `ImplStatus` via a `status()` method.
    #[test]
    fn every_kind_has_status() {
        for k in MarkKind::all() {
            let _ = k.status();
        }
        for k in InteractorKind::all() {
            let _ = k.status();
        }
        for k in InputKind::all() {
            let _ = k.status();
        }
        for k in ComponentKind::all() {
            let _ = k.status();
        }
        for k in SelectionResolution::all() {
            let _ = k.status();
        }
        for k in LegendChannel::all() {
            let _ = k.status();
        }
    }

    /// (ifb) — REVISED (harden, 2026-07-02). `Highlight` stays
    /// `Implemented` (the renderer's `HighlightState` dim/emphasis is wired), but
    /// `Nearest`/`NearestX`/`NearestY` are demoted to `Unimplemented`: parsed but
    /// unwired (`find_nearest` has no production caller; hover resolves
    /// `nearest: None`). See the demotion memo.
    #[test]
    fn feedback_variant_statuses_after_demotion() {
        assert_eq!(
            InteractorKind::Highlight.status(),
            ImplStatus::Implemented,
            "Highlight stays Implemented — its renderer HighlightState is wired"
        );
        for variant in [
            InteractorKind::Nearest,
            InteractorKind::NearestX,
            InteractorKind::NearestY,
        ] {
            assert_eq!(
                variant.status(),
                ImplStatus::Unimplemented,
                "{variant:?} is parsed but unwired — demoted until a hover handler consumes it"
            );
        }
    }

    /// Input-widget status (recorded 2026-07-03; updated 2026-07-17).
    /// `InputKind::Slider` is Implemented (a hosted SliderElement drives its
    /// param through commit_slider → propagate_param → re-render) AND
    /// `InputKind::Menu` is Implemented (a hosted MenuElement
    /// drives its param through commit_menu → propagate_param → re-render,
    /// radio/checkbox riding as `style:` presentations). Search/Table stay
    /// Unimplemented — and keep warning honestly at parse.
    #[test]
    fn input_kind_slider_implemented_when_wired() {
        assert_eq!(
            InputKind::Slider.status(),
            ImplStatus::Implemented,
            "Slider is wired end-to-end — re-promoted"
        );
        assert_eq!(
            InputKind::Menu.status(),
            ImplStatus::Implemented,
            "Menu is wired end-to-end — promoted"
        );
        let implemented: Vec<InputKind> = InputKind::all()
            .iter()
            .copied()
            .filter(|k| k.status() == ImplStatus::Implemented)
            .collect();
        assert_eq!(
            implemented,
            vec![InputKind::Menu, InputKind::Slider],
            "exactly Menu + Slider are implemented; Search/Table remain Unimplemented"
        );
    }

    /// cfs point-selection (2026-07-03). `toggleX`/`toggleY` are
    /// Implemented: each maps to a single-channel point selection
    /// (BrushKind::PointX/PointY) that drives an equality predicate through
    /// propagate_selection. `toggle` (both axes) stays Unimplemented until its
    /// value-pair producer + click gesture land.
    #[test]
    fn toggle_x_y_implemented_toggle_deferred() {
        assert_eq!(InteractorKind::ToggleX.status(), ImplStatus::Implemented);
        assert_eq!(InteractorKind::ToggleY.status(), ImplStatus::Implemented);
        assert_eq!(
            InteractorKind::Toggle.status(),
            ImplStatus::Unimplemented,
            "toggle (both axes) stays deferred until its value-pair producer lands"
        );
    }

    /// The six Pan/PanZoom variants stay `Unimplemented`, and the reason has
    /// been restated because the old one cited functions that no longer have a
    /// caller.
    ///
    /// It used to say `apply_pan` / `apply_zoom` / `ChartState::set_navigation`
    /// existed unit-tested and unwired. Those live in modules the shipped
    /// binary cannot reach; navigation is BUILT now, with its own gesture
    /// arithmetic, its own settle rule, a persistent extent on the session and
    /// pointer plus keyboard bindings.
    ///
    /// What these six variants are is a way for a SPEC to declare that a plot
    /// is navigable and along which axes. Nothing reads them: navigation is
    /// offered on any plot with a continuous positional scale, and the axis
    /// lock is a view-level toggle the reader cycles rather than something the
    /// spec pins. So the vocabulary entries remain genuinely unconsumed, which
    /// is what `Unimplemented` means here, and promoting them would publish a
    /// spec capability that changes nothing about what the app does.
    ///
    /// Nothing about conformance turns on this either way: `panZoom` appears in
    /// exactly one of the 54 vendored upstream specs, and that spec also uses
    /// `mark: frame`, which stays unimplemented — so these six unblock no
    /// spec.
    #[test]
    fn pan_variants_unimplemented_until_wired() {
        for variant in [
            InteractorKind::Pan,
            InteractorKind::PanX,
            InteractorKind::PanY,
            InteractorKind::PanZoom,
            InteractorKind::PanZoomX,
            InteractorKind::PanZoomY,
        ] {
            assert_eq!(
                variant.status(),
                ImplStatus::Unimplemented,
                "{variant:?} is parsed, and nothing consumes it: navigation is a view \
                 gesture rather than a spec-declared capability"
            );
        }
    }

    #[test]
    fn wire_lookup_round_trips() {
        for k in MarkKind::all() {
            assert_eq!(MarkKind::from_wire(k.wire_name()), Some(*k));
        }
        assert!(MarkKind::from_wire("fooBar").is_none());
    }

    /// Sequential colour scale. `LegendChannel::Color`
    /// stays Implemented — it now covers continuous (gradient-bar) legends as
    /// well as categorical (swatch) legends.
    #[test]
    fn legend_color_channel_stays_implemented() {
        assert_eq!(LegendChannel::Color.status(), ImplStatus::Implemented);
    }

    /// RE-PINNED by the hexbin follow-up, then by geo.
    /// Heatmap, Contour, Cell (density marks) plus Hexbin and Hexgrid are
    /// Implemented: hexbin is pixel-space cube-round binning with a
    /// self-aggregating fill, hexgrid the decorative dataless mesh. The
    /// still-deferred marks stay Unimplemented: cellX/cellY, denseLine, and
    /// Voronoi (the always-unimplemented swap stand-in — geo was promoted).
    #[test]
    fn density_mark_promotions_and_non_promotions() {
        for promoted in [
            MarkKind::Heatmap,
            MarkKind::Contour,
            MarkKind::Cell,
            MarkKind::Hexbin,
            MarkKind::Hexgrid,
        ] {
            assert_eq!(
                promoted.status(),
                ImplStatus::Implemented,
                "{promoted:?} is wired end-to-end — promoted"
            );
        }
        for staged_out in [
            MarkKind::CellX,
            MarkKind::CellY,
            MarkKind::DenseLine,
            MarkKind::Voronoi,
        ] {
            assert_eq!(
                staged_out.status(),
                ImplStatus::Unimplemented,
                "{staged_out:?} stays deferred — it must NOT ride this promotion"
            );
        }
    }

    /// Geo mark. `MarkKind::Geo` is promoted to
    /// Implemented — it parses with no `ParseWarning::Unimplemented` and renders
    /// end-to-end. `Voronoi` inherits geo's former role as the always-
    /// unimplemented census stand-in (genuinely far off; no lowerer/renderer).
    #[test]
    fn geo_implemented_voronoi_is_new_standin() {
        assert_eq!(
            MarkKind::Geo.status(),
            ImplStatus::Implemented,
            "geo renders end-to-end — promoted"
        );
        assert_eq!(
            MarkKind::Voronoi.status(),
            ImplStatus::Unimplemented,
            "Voronoi is the new always-unimplemented census stand-in"
        );
    }

    /// Every composition-level component renders.
    ///
    /// `Legend` was promoted when standalone legends started rendering in the
    /// headless composite AND as hosted window elements at their layout rects.
    /// The five layout forms carry the status their shipped layout
    /// implementation warrants: each is positioned by `compute_layout` and
    /// consumed through `placed_plots`, the call the live window and the
    /// headless composite share. Pinned as a whole list so a new component
    /// kind cannot be added at `Implemented` without someone saying so here.
    #[test]
    fn every_component_kind_is_implemented() {
        assert_eq!(
            ComponentKind::Legend.status(),
            ImplStatus::Implemented,
            "legend is hosted in the window"
        );
        let implemented: Vec<ComponentKind> = ComponentKind::all()
            .iter()
            .copied()
            .filter(|k| k.status() == ImplStatus::Implemented)
            .collect();
        assert_eq!(
            implemented,
            vec![
                ComponentKind::Plot,
                ComponentKind::HConcat,
                ComponentKind::VConcat,
                ComponentKind::HSpace,
                ComponentKind::VSpace,
                ComponentKind::Legend,
            ],
            "all six composition components render; a Legend whose CHANNEL is \
             unimplemented is caught by preflight's channel arm, not here"
        );
    }
}
