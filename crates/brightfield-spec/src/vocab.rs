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
        AreaY => ("areaY", Unimplemented),
        AreaX => ("areaX", Unimplemented),
        BarY => ("barY", Unimplemented),
        BarX => ("barX", Unimplemented),
        // Cells
        Cell => ("cell", Unimplemented),
        CellX => ("cellX", Unimplemented),
        CellY => ("cellY", Unimplemented),
        // Dots / circles
        Dot => ("dot", Unimplemented),
        DotX => ("dotX", Unimplemented),
        DotY => ("dotY", Unimplemented),
        Circle => ("circle", Unimplemented),
        // Lines
        Line => ("line", Unimplemented),
        LineX => ("lineX", Unimplemented),
        LineY => ("lineY", Unimplemented),
        // Rectangles
        Rect => ("rect", Unimplemented),
        RectX => ("rectX", Unimplemented),
        RectY => ("rectY", Unimplemented),
        // Rules / ticks
        Rule => ("rule", Unimplemented),
        RuleX => ("ruleX", Unimplemented),
        RuleY => ("ruleY", Unimplemented),
        TickX => ("tickX", Unimplemented),
        TickY => ("tickY", Unimplemented),
        // Text
        Text => ("text", Unimplemented),
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
        Raster => ("raster", Unimplemented),
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
        IntervalX => ("intervalX", Unimplemented),
        IntervalY => ("intervalY", Unimplemented),
        IntervalXY => ("intervalXY", Unimplemented),
        Interval => ("interval", Unimplemented),
        Nearest => ("nearest", Implemented),
        NearestX => ("nearestX", Implemented),
        NearestY => ("nearestY", Implemented),
        Toggle => ("toggle", Unimplemented),
        ToggleX => ("toggleX", Unimplemented),
        ToggleY => ("toggleY", Unimplemented),
        Highlight => ("highlight", Implemented),
        Region => ("region", Unimplemented),
        Pan => ("pan", Implemented),
        PanX => ("panX", Implemented),
        PanY => ("panY", Implemented),
        PanZoom => ("panZoom", Implemented),
        PanZoomX => ("panZoomX", Implemented),
        PanZoomY => ("panZoomY", Implemented),
    }
}

vocab_enum! {
    /// Known input widget kinds (the `input:` discriminator).
    pub enum InputKind {
        Menu => ("menu", Unimplemented),
        Search => ("search", Unimplemented),
        Slider => ("slider", Unimplemented),
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
        Color => ("color", Unimplemented),
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

    /// ac-09 (ifb): Nearest, NearestX, NearestY, Highlight are Implemented.
    #[test]
    fn ifb_ac09_feedback_variants_implemented() {
        let variants = [
            InteractorKind::Nearest,
            InteractorKind::NearestX,
            InteractorKind::NearestY,
            InteractorKind::Highlight,
        ];
        for variant in &variants {
            assert_eq!(
                variant.status(),
                ImplStatus::Implemented,
                "{:?} should be Implemented",
                variant
            );
        }
    }

    /// ac-11 (nav): All six Pan/PanZoom interactor variants are Implemented.
    #[test]
    fn nav_ac11_pan_variants_implemented() {
        let pan_variants = [
            InteractorKind::Pan,
            InteractorKind::PanX,
            InteractorKind::PanY,
            InteractorKind::PanZoom,
            InteractorKind::PanZoomX,
            InteractorKind::PanZoomY,
        ];
        for variant in &pan_variants {
            assert_eq!(
                variant.status(),
                ImplStatus::Implemented,
                "{:?} should be Implemented",
                variant
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
}
