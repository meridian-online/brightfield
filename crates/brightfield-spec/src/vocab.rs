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
    /// The one registry both halves read: the parser lifts the pair only for a
    /// kind named here, and `RectLowerer` is registered only for the same
    /// kinds, so a spec cannot parse as binned and then lower as `SELECT *`.
    /// Mosaic's own list is the `EXTENT` set in `vgplot/plot/transforms/bin.js`
    /// (`rectY-x`, `rectX-y`, `rect-x`, `rect-y`, `ruleY-x`, `ruleX-y`);
    /// the rules are left off because brightfield's rule lowerer does not
    /// aggregate, and a binned rule would parse quiet and still draw nothing.
    #[must_use]
    pub fn bins_positionally(self) -> bool {
        matches!(self, Self::Rect | Self::RectX | Self::RectY)
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
];

/// Whether a mark option key reaches a lowerer or a renderer.
///
/// `false` means the key is carried through the AST intact and then dropped
/// on the floor — see [`CONSUMED_MARK_OPTION_KEYS`].
#[must_use]
pub fn mark_option_is_consumed(key: &str) -> bool {
    CONSUMED_MARK_OPTION_KEYS.contains(&key)
}

/// The CSS Color Module Level 4 named colours, plus the three keywords a Plot
/// spec may write in a colour slot (`none`, `transparent`, `currentColor`).
/// Sorted, so [`is_colour_literal`] can binary-search it.
///
/// A fixed, standardised vocabulary — it does not grow with brightfield.
const CSS_COLOUR_KEYWORDS: &[&str] = &[
    "aliceblue",
    "antiquewhite",
    "aqua",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "black",
    "blanchedalmond",
    "blue",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "currentcolor",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "fuchsia",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "gray",
    "green",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "lime",
    "limegreen",
    "linen",
    "magenta",
    "maroon",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "navy",
    "none",
    "oldlace",
    "olive",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "purple",
    "rebeccapurple",
    "red",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "silver",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "teal",
    "thistle",
    "tomato",
    "transparent",
    "turquoise",
    "violet",
    "wheat",
    "white",
    "whitesmoke",
    "yellow",
    "yellowgreen",
];

/// Whether a colour-channel string is a **constant colour** rather than a
/// column name — `fill: steelblue` versus `fill: version`.
///
/// This is Observable Plot's rule, not one invented here: Plot's
/// `maybeColorChannel` treats a string that `isColor()` accepts as a constant
/// and everything else as a field name, which is why the same spec can write
/// `fill: steelblue` and `fill: weather` and mean different things. Plot is not
/// vendored in this tree, so the keyword table is the CSS Level 4 list rather
/// than a port.
///
/// It decides one thing here: a rect binding a colour CONSTANT carries no
/// groups, so its `bin` + `count` is a plain histogram and can be lifted; a
/// rect binding a COLUMN is one Mosaic stacks, and lifting it would merge the
/// groups into a single bar that looks right and under-reports every group but
/// one. Unrecognised ⇒ treated as a column, which is the safe direction: the
/// spec keeps its uncomputed-transform diagnostic instead of drawing a lie.
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
    CSS_COLOUR_KEYWORDS.binary_search(&lower.as_str()).is_ok()
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
                pair[0] < pair[1],
                "CSS_COLOUR_KEYWORDS must be sorted and duplicate-free, but \
                 {:?} precedes {:?}",
                pair[0],
                pair[1]
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

    /// The registry that keeps the parser and the lowerer from disagreeing:
    /// the kinds that lift a positional bin are exactly the kinds `RectLowerer`
    /// is registered for.
    #[test]
    fn only_the_rect_family_bins_positionally() {
        let binning: Vec<&str> = MarkKind::all()
            .iter()
            .filter(|k| k.bins_positionally())
            .map(|k| k.wire_name())
            .collect();
        assert_eq!(binning, vec!["rect", "rectX", "rectY"]);
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
        // And the keys the corpus proved unconsumed stay unconsumed.
        for ignored in ["sort", "limit", "curve", "fillOpacity", "r"] {
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
