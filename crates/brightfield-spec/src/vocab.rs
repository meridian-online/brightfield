//! Registry-backed vocabulary for the Mosaic 0.24.x spec (Option Z).
//!
//! Every mark, interactor, input, and component name a Mosaic spec may use
//! is enumerated here. Each variant carries an implementation status —
//! `Implemented | Planned | Unimplemented` — so the preflight `SupportReport`
//! (card 0002) can walk a parsed AST and report which specs exercise
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
        Cell => ("cell", Unimplemented),
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
        // rect/rectX/rectY wired end-to-end (card 0008, 2026-07-03): RectRenderer
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
        Heatmap => ("heatmap", Unimplemented),
        Contour => ("contour", Unimplemented),
        // Binned 2D count heatmap — filled cells coloured (by alpha) per bin
        // count, reusing the 2D density binning. (card 0008 mark breadth)
        Raster => ("raster", Implemented),
        // Hex
        Hexbin => ("hexbin", Unimplemented),
        Hexgrid => ("hexgrid", Unimplemented),
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
        // Geo
        Geo => ("geo", Unimplemented),
        Sphere => ("sphere", Unimplemented),
        Graticule => ("graticule", Unimplemented),
        // Voronoi / delaunay / hull
        Voronoi => ("voronoi", Unimplemented),
        VoronoiMesh => ("voronoiMesh", Unimplemented),
        DelaunayMesh => ("delaunayMesh", Unimplemented),
        DelaunayLink => ("delaunayLink", Unimplemented),
        Hull => ("hull", Unimplemented),
        // Image
        Image => ("image", Unimplemented),
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
        // toggleX/toggleY wired end-to-end (card 0006, 2026-07-03): each becomes
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
        Menu => ("menu", Unimplemented),
        Search => ("search", Unimplemented),
        // Wired end-to-end (card 0005, 2026-07-03): a hosted SliderElement drives
        // its param via commit_slider → propagate_param → re-render. Menu/Search/
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
        Plot => ("plot", Unimplemented),
        HConcat => ("hconcat", Unimplemented),
        VConcat => ("vconcat", Unimplemented),
        HSpace => ("hspace", Unimplemented),
        VSpace => ("vspace", Unimplemented),
        Legend => ("legend", Unimplemented),
    }
}

vocab_enum! {
    /// Selection-resolution kinds as they appear under a `params.<name>:
    /// { select: <resolution> }` declaration.
    pub enum SelectionResolution {
        Crossfilter => ("crossfilter", Unimplemented),
        Intersect => ("intersect", Unimplemented),
        Single => ("single", Unimplemented),
        Union => ("union", Unimplemented),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ac-03 verification: every variant of every Kind enum exposes an
    /// `ImplStatus` via a `status()` method.
    #[test]
    fn dfspec_ac03_every_kind_has_status() {
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

    /// ac-09 (ifb) — REVISED (harden, 2026-07-02). `Highlight` stays
    /// `Implemented` (the renderer's `HighlightState` dim/emphasis is wired), but
    /// `Nearest`/`NearestX`/`NearestY` are demoted to `Unimplemented`: parsed but
    /// unwired (`find_nearest` has no production caller; hover resolves
    /// `nearest: None`). Reverses ifb ac-09/ac-10 — see the demotion memo.
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

    /// slw ac-08 (card 0005, 2026-07-03). `InputKind::Slider` is Implemented: a
    /// hosted SliderElement drives its param through commit_slider →
    /// propagate_param → re-render (reversing the 2026-07-02 harden demotion for
    /// slider only). The other input kinds — Menu/Search/Table — stay Unimplemented.
    #[test]
    fn slw_ac08_input_kind_slider_implemented_when_wired() {
        assert_eq!(
            InputKind::Slider.status(),
            ImplStatus::Implemented,
            "Slider is wired end-to-end (card 0005) — re-promoted"
        );
        let implemented: Vec<InputKind> = InputKind::all()
            .iter()
            .copied()
            .filter(|k| k.status() == ImplStatus::Implemented)
            .collect();
        assert_eq!(
            implemented,
            vec![InputKind::Slider],
            "only Slider is implemented; Menu/Search/Table remain Unimplemented"
        );
    }

    /// cfs point-selection (card 0006, 2026-07-03). `toggleX`/`toggleY` are
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

    /// ac-11 (nav) — REVERSED (harden, 2026-07-02). The six Pan/PanZoom variants
    /// are demoted to `Unimplemented`: `apply_pan`/`apply_zoom`/
    /// `ChartState::set_navigation` exist and are unit-tested, but no production
    /// caller wires them (no scroll/wheel handler; navigation is always None).
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
                "{variant:?} is parsed but unwired — demoted until navigation is consumed"
            );
        }
    }

    #[test]
    fn dfspec_ac03_wire_lookup_round_trips() {
        for k in MarkKind::all() {
            assert_eq!(MarkKind::from_wire(k.wire_name()), Some(*k));
        }
        assert!(MarkKind::from_wire("fooBar").is_none());
    }

    /// scs_ac09 (card 0008, sequential colour scale). `LegendChannel::Color`
    /// stays Implemented — it now covers continuous (gradient-bar) legends as
    /// well as categorical (swatch) legends.
    #[test]
    fn scs_ac09_legend_color_channel_stays_implemented() {
        assert_eq!(LegendChannel::Color.status(), ImplStatus::Implemented);
    }
}
