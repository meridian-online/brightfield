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
        Plot => ("plot", Unimplemented),
        HConcat => ("hconcat", Unimplemented),
        VConcat => ("vconcat", Unimplemented),
        HSpace => ("hspace", Unimplemented),
        VSpace => ("vspace", Unimplemented),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// verification: every variant of every Kind enum exposes an
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

    /// 2026-07-03; UPDATED 2026-07-17.
    /// `InputKind::Slider` is Implemented (a hosted SliderElement drives its
    /// param through commit_slider → propagate_param → re-render) AND
    /// `InputKind::Menu` is Implemented (a hosted MenuElement
    /// drives its param through commit_menu → propagate_param → re-render,
    /// radio/checkbox riding as `style:` presentations). Search/Table stay
    /// Unimplemented — and keep warning honestly at parse.
    #[test]
    fn slw_ac08_input_kind_slider_implemented_when_wired() {
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

    /// (nav) — REVERSED (harden, 2026-07-02). The six Pan/PanZoom variants
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

    /// Sequential colour scale. `LegendChannel::Color`
    /// stays Implemented — it now covers continuous (gradient-bar) legends as
    /// well as categorical (swatch) legends.
    #[test]
    fn scs_ac09_legend_color_channel_stays_implemented() {
        assert_eq!(LegendChannel::Color.status(), ImplStatus::Implemented);
    }

    /// RE-PINNED by the hexbin follow-up, then by geo.
    /// Heatmap, Contour, Cell (density marks) plus Hexbin and Hexgrid are
    /// Implemented: hexbin is pixel-space cube-round binning with a
    /// self-aggregating fill, hexgrid the decorative dataless mesh. The
    /// still-deferred marks stay Unimplemented: cellX/cellY, denseLine, and
    /// Voronoi (the always-unimplemented swap stand-in — geo was promoted).
    #[test]
    fn dmk_ac05_density_mark_promotions_and_non_promotions() {
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
    fn geo_ac01_geo_implemented_voronoi_is_new_standin() {
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

    /// Framed window. `ComponentKind::Legend` is
    /// promoted to Implemented: standalone legends render in the headless
    /// composite AND as hosted window elements at their layout rects. The
    /// other layout components stay Unimplemented (DEV-0001 scaffolding).
    #[test]
    fn fww_ac05_component_legend_implemented_when_hosted() {
        assert_eq!(
            ComponentKind::Legend.status(),
            ImplStatus::Implemented,
            "legend is hosted in the window — promoted"
        );
        let implemented: Vec<ComponentKind> = ComponentKind::all()
            .iter()
            .copied()
            .filter(|k| k.status() == ImplStatus::Implemented)
            .collect();
        assert_eq!(
            implemented,
            vec![ComponentKind::Legend],
            "only Legend is implemented; the layout components remain Unimplemented"
        );
    }
}
