//! Scale types and inference — mapping data domains to pixel ranges.
//!
//! Supports linear (numeric), band (categorical), and time (timestamp) scales.
//! Scale inference examines Arrow column types to determine the appropriate
//! scale type for each encoding channel.

use std::collections::HashMap;

use arrow::array::{Array, Float64Array, StringArray, TimestampMicrosecondArray};
use arrow::datatypes::{DataType, TimeUnit};
use arrow::record_batch::RecordBatch;
use brightfield_spec::layout::FixedDomains;

use crate::channel::{Channel, ChannelMap};

/// Put a colour scale's category list into the order that fixes each category's
/// palette slot: ascending by the category's own text.
///
/// A palette slot is a category's INDEX in this list, so whatever produces the
/// list decides the colours. Left to first appearance, that producer is the row
/// order of a scan — which DuckDB does not promise to repeat, and which a
/// sample changes outright. Ordering by the value instead makes the slot a
/// function of the category set alone, so two renders of the same spec agree,
/// and a render over a subset of the rows agrees with the render over all of
/// them as long as the subset's set is the same.
///
/// **Colour only, and positional band scales deliberately not.** A band scale's
/// order is where the bars are, and the query that produced the rows may have
/// ordered them on purpose — re-ordering it alphabetically would answer a
/// determinism problem by discarding an author's `ORDER BY`. A colour scale
/// carries no such instruction: what a reader needs from it is that the same
/// category takes the same slot every time, not that a particular one leads.
///
/// Ordering here rather than in SQL keeps one comparator for both producers.
/// The categories a render infers come out of an Arrow batch and the ones a
/// restoration supplies come out of a query, and a DuckDB collation ordering
/// the second could disagree with a Rust comparator ordering the first on
/// exactly the inputs where it matters.
pub fn order_categories(categories: &mut [String]) {
    categories.sort_unstable();
}

/// A single scale mapping a data domain to a pixel range.
#[derive(Debug, Clone)]
pub enum Scale {
    /// Numeric linear scale: maps [min, max] -> [range_start, range_end].
    Linear {
        domain_min: f64,
        domain_max: f64,
        range_start: f64,
        range_end: f64,
    },
    /// Categorical band scale: maps discrete categories to equal-width bands.
    Band {
        categories: Vec<String>,
        range_start: f64,
        range_end: f64,
        padding: f64,
    },
    /// Time scale: maps [min_us, max_us] microsecond timestamps to pixel range.
    Time {
        domain_min_us: i64,
        domain_max_us: i64,
        range_start: f64,
        range_end: f64,
    },
    /// Colour scale: maps categories to colours from a palette.
    Colour {
        categories: Vec<String>,
        palette: Vec<[f32; 4]>,
    },
    /// Sequential colour scale: maps a numeric magnitude to an interpolated
    /// colour ramp. `stops` are evenly-spaced RGBA control points (low → high);
    /// [`Scale::map_continuous`] normalises a value into `[0, 1]` and
    /// piecewise-lerps between the bracketing pair.
    Sequential {
        domain_min: f64,
        domain_max: f64,
        stops: Vec<[f32; 4]>,
    },
}

impl Scale {
    /// Map a numeric value to a pixel position (linear and time scales).
    pub fn map_f64(&self, value: f64) -> f64 {
        match self {
            Self::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            } => {
                if (domain_max - domain_min).abs() < f64::EPSILON {
                    return (*range_start + *range_end) / 2.0;
                }
                let t = (value - domain_min) / (domain_max - domain_min);
                range_start + t * (range_end - range_start)
            }
            Self::Time {
                domain_min_us,
                domain_max_us,
                range_start,
                range_end,
            } => {
                let span = (*domain_max_us - *domain_min_us) as f64;
                if span.abs() < f64::EPSILON {
                    return (*range_start + *range_end) / 2.0;
                }
                let t = (value - *domain_min_us as f64) / span;
                range_start + t * (range_end - range_start)
            }
            _ => value, // Band/Colour scales don't use f64 mapping.
        }
    }

    /// Map a numeric value to an interpolated ramp colour (Sequential scales).
    ///
    /// Clamps `value` into the domain, normalises to `t ∈ [0, 1]`, and lerps
    /// per-channel between the two `stops` bracketing `t·(n-1)`. Endpoints return
    /// the first/last stop exactly; a degenerate (`domain_min == domain_max`)
    /// domain returns the top stop (mirroring how `map_f64` collapses a zero-span
    /// linear domain). Returns opaque black for a non-Sequential scale — callers
    /// only invoke this on the Fill Sequential scale.
    pub fn map_continuous(&self, value: f64) -> [f32; 4] {
        let Self::Sequential {
            domain_min,
            domain_max,
            stops,
        } = self
        else {
            return [0.0, 0.0, 0.0, 1.0];
        };
        let Some(&top) = stops.last() else {
            return [0.0, 0.0, 0.0, 1.0];
        };
        let span = domain_max - domain_min;
        if span.abs() < f64::EPSILON {
            return top;
        }
        let t = ((value - domain_min) / span).clamp(0.0, 1.0);
        let n = stops.len();
        if n == 1 {
            return stops[0];
        }
        let scaled = t * (n - 1) as f64;
        let i = (scaled.floor() as usize).min(n - 2);
        let frac = (scaled - i as f64) as f32;
        let a = stops[i];
        let b = stops[i + 1];
        [
            a[0] + (b[0] - a[0]) * frac,
            a[1] + (b[1] - a[1]) * frac,
            a[2] + (b[2] - a[2]) * frac,
            a[3] + (b[3] - a[3]) * frac,
        ]
    }

    /// Map a pixel position back to a data value (inverse of `map_f64`).
    ///
    /// Returns `Some` for continuous scales (Linear, Time), `None` for discrete
    /// scales (Band, Colour) where continuous inversion is undefined.
    /// For Time scales, the returned f64 represents microsecond timestamp.
    pub fn inverse_f64(&self, pixel: f64) -> Option<f64> {
        match self {
            Self::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            } => {
                let range_span = range_end - range_start;
                if range_span.abs() < f64::EPSILON {
                    return Some((*domain_min + *domain_max) / 2.0);
                }
                let t = (pixel - range_start) / range_span;
                Some(domain_min + t * (domain_max - domain_min))
            }
            Self::Time {
                domain_min_us,
                domain_max_us,
                range_start,
                range_end,
            } => {
                let range_span = range_end - range_start;
                if range_span.abs() < f64::EPSILON {
                    return Some((*domain_min_us + *domain_max_us) as f64 / 2.0);
                }
                let t = (pixel - range_start) / range_span;
                let domain_span = (*domain_max_us - *domain_min_us) as f64;
                Some(*domain_min_us as f64 + t * domain_span)
            }
            Self::Band { .. } | Self::Colour { .. } | Self::Sequential { .. } => None,
        }
    }

    /// Map a category to a band centre position.
    pub fn map_category(&self, category: &str) -> Option<f64> {
        match self {
            Self::Band {
                categories,
                range_start,
                range_end,
                padding,
            } => {
                let idx = categories.iter().position(|c| c == category)?;
                let n = categories.len() as f64;
                let total_range = range_end - range_start;
                let band_width = total_range / n;
                let padded_start = range_start + band_width * *padding / 2.0;
                Some(padded_start + band_width * idx as f64 + band_width * (1.0 - *padding) / 2.0)
            }
            _ => None,
        }
    }

    /// Get the band width for bar rendering.
    pub fn band_width(&self) -> Option<f64> {
        match self {
            Self::Band {
                categories,
                range_start,
                range_end,
                padding,
            } => {
                let n = categories.len() as f64;
                if n == 0.0 {
                    return None;
                }
                let total_range = range_end - range_start;
                let band_width = total_range / n;
                Some(band_width * (1.0 - padding))
            }
            _ => None,
        }
    }

    /// Look up the colour for a category.
    pub fn map_colour(&self, category: &str) -> Option<[f32; 4]> {
        match self {
            Self::Colour {
                categories,
                palette,
            } => {
                let idx = categories.iter().position(|c| c == category)?;
                Some(palette[idx % palette.len()])
            }
            _ => None,
        }
    }

    /// Domain min for linear/time/sequential scales. A Sequential's extent feeds
    /// the gradient-legend min tick label.
    pub fn domain_min(&self) -> Option<f64> {
        match self {
            Self::Linear { domain_min, .. } => Some(*domain_min),
            Self::Time { domain_min_us, .. } => Some(*domain_min_us as f64),
            Self::Sequential { domain_min, .. } => Some(*domain_min),
            _ => None,
        }
    }

    /// Domain max for linear/time/sequential scales. A Sequential's extent feeds
    /// the gradient-legend max tick label.
    pub fn domain_max(&self) -> Option<f64> {
        match self {
            Self::Linear { domain_max, .. } => Some(*domain_max),
            Self::Time { domain_max_us, .. } => Some(*domain_max_us as f64),
            Self::Sequential { domain_max, .. } => Some(*domain_max),
            _ => None,
        }
    }

    /// Range start.
    pub fn range_start(&self) -> f64 {
        match self {
            Self::Linear { range_start, .. }
            | Self::Band { range_start, .. }
            | Self::Time { range_start, .. } => *range_start,
            // Colour ramps carry no positional pixel range.
            Self::Colour { .. } | Self::Sequential { .. } => 0.0,
        }
    }

    /// Range end.
    pub fn range_end(&self) -> f64 {
        match self {
            Self::Linear { range_end, .. }
            | Self::Band { range_end, .. }
            | Self::Time { range_end, .. } => *range_end,
            // Colour ramps carry no positional pixel range.
            Self::Colour { .. } | Self::Sequential { .. } => 0.0,
        }
    }
}

/// Optional override of data-inferred scale domains per axis.
///
/// When `Some`, the chart renders only the specified data range on that axis.
/// When `None`, the full data-inferred domain is used.
/// Used by pan/zoom navigation — the interaction layer mutates this struct,
/// the render and engine layers consume it read-only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewExtent {
    /// Overridden x-axis domain: `Some((min, max))` or `None` for full extent.
    pub x: Option<(f64, f64)>,
    /// Overridden y-axis domain: `Some((min, max))` or `None` for full extent.
    pub y: Option<(f64, f64)>,
}

/// Collection of inferred scales for a chart, keyed by channel.
#[derive(Debug, Clone, Default)]
pub struct ScaleSet {
    scales: HashMap<Channel, Scale>,
}

impl ScaleSet {
    /// Create an empty scale set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a scale for a channel.
    pub fn insert(&mut self, channel: Channel, scale: Scale) {
        self.scales.insert(channel, scale);
    }

    /// Get the scale for a channel.
    pub fn get(&self, channel: Channel) -> Option<&Scale> {
        self.scales.get(&channel)
    }
}

/// Anchor a freshly-inferred scale set to a launch reference, widen-only
/// (launch-anchored scales). Each rebuild infers `fresh` from the current
/// batches and folds it against the immutable `launch` set so the frame of
/// reference holds still while only the data moves — but a gesture that
/// REWRITES the query (a slider changing a `$param`, not a subset filter) can
/// surface rows outside the launch domain, which a hard pin would clip into
/// invisibility, so the anchor widens rather than pins.
///
/// Per channel:
/// - present in BOTH, continuous (Linear / Time / Sequential): the launch
///   domain UNIONed with the fresh domain (`min` of mins, `max` of maxes), on
///   the launch range/stops. A subset batch (`fresh ⊆ launch`) yields exactly
///   `launch`, so an ordinary filter gesture stays pixel-identical to a hard
///   pin; a query-rewrite gesture widens to keep the new rows on-plot.
/// - present in BOTH, categorical (Band / Colour) or a scale-kind mismatch:
///   `launch` wins (category positions and colours never churn under a gesture;
///   a param-introduced new category renders as the existing missing-category
///   behaviour — v1 scope).
/// - only in `launch`: `launch`. Only in `fresh`: `fresh` — the case of a mark
///   whose batch was empty at launch (so it contributed no scales) and arrived
///   later, e.g. a raster whose `colorScheme` Fill ramp would otherwise never
///   appear.
#[must_use]
pub fn anchor_scales(launch: &ScaleSet, fresh: ScaleSet) -> ScaleSet {
    let mut anchored = ScaleSet::new();
    for &ch in Channel::all() {
        let scale = match (launch.get(ch), fresh.get(ch)) {
            (Some(l), Some(f)) => anchor_scale(l, f),
            (Some(l), None) => l.clone(),
            (None, Some(f)) => f.clone(),
            (None, None) => continue,
        };
        anchored.insert(ch, scale);
    }
    anchored
}

/// Fold one channel's `fresh` scale into its `launch` scale per the widen-only
/// rule (see [`anchor_scales`]). Continuous scales widen the launch domain to
/// include fresh; categorical scales and any kind mismatch keep launch.
fn anchor_scale(launch: &Scale, fresh: &Scale) -> Scale {
    match (launch, fresh) {
        (
            Scale::Linear {
                domain_min: lmin,
                domain_max: lmax,
                range_start,
                range_end,
            },
            Scale::Linear {
                domain_min: fmin,
                domain_max: fmax,
                ..
            },
        ) => Scale::Linear {
            domain_min: lmin.min(*fmin),
            domain_max: lmax.max(*fmax),
            range_start: *range_start,
            range_end: *range_end,
        },
        (
            Scale::Time {
                domain_min_us: lmin,
                domain_max_us: lmax,
                range_start,
                range_end,
            },
            Scale::Time {
                domain_min_us: fmin,
                domain_max_us: fmax,
                ..
            },
        ) => Scale::Time {
            domain_min_us: (*lmin).min(*fmin),
            domain_max_us: (*lmax).max(*fmax),
            range_start: *range_start,
            range_end: *range_end,
        },
        (
            Scale::Sequential {
                domain_min: lmin,
                domain_max: lmax,
                stops,
            },
            Scale::Sequential {
                domain_min: fmin,
                domain_max: fmax,
                ..
            },
        ) => Scale::Sequential {
            domain_min: lmin.min(*fmin),
            domain_max: lmax.max(*fmax),
            stops: stops.clone(),
        },
        // Categorical (Band / Colour) or a scale-kind mismatch: launch wins.
        (l, _) => l.clone(),
    }
}

/// One positional axis's pinned domain — the frame of reference a
/// `xDomain: Fixed` / `yDomain: Fixed` plot keeps while filters move the rows
/// underneath it.
///
/// A domain and nothing else. The pixel RANGE is deliberately not carried: a
/// range belongs to the layout the scale was inferred against, and a window
/// resize gives the same plot a new one. Applying a pin re-domains the scale
/// the current layout produced, so a pinned plot resizes like any other.
#[derive(Debug, Clone, PartialEq)]
pub enum PinnedDomain {
    /// A continuous numeric extent, from a [`Scale::Linear`].
    Linear(f64, f64),
    /// A microsecond-timestamp extent, from a [`Scale::Time`].
    Time(i64, i64),
    /// A categorical ORDER, from a [`Scale::Band`] — the list whose index
    /// assigns each category its slot along the axis. Pinning it is what keeps
    /// a filtered-away category's slot open instead of closing the gap and
    /// sliding every later category one place.
    Band(Vec<String>),
}

impl PinnedDomain {
    /// The pin `scale` hands back, or `None` for a scale carrying no positional
    /// domain to pin.
    ///
    /// [`Scale::Colour`] and [`Scale::Sequential`] answer `None` here because
    /// they are the COLOUR channels' scales; a positional axis never resolves
    /// to one, and the explicit `colorDomain` instruction is a separate
    /// mechanism ([`ColourOverride`]).
    #[must_use]
    pub fn of(scale: &Scale) -> Option<Self> {
        match scale {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => Some(Self::Linear(*domain_min, *domain_max)),
            Scale::Time {
                domain_min_us,
                domain_max_us,
                ..
            } => Some(Self::Time(*domain_min_us, *domain_max_us)),
            Scale::Band { categories, .. } => Some(Self::Band(categories.clone())),
            Scale::Colour { .. } | Scale::Sequential { .. } => None,
        }
    }

    /// `scale` re-domained onto this pin, keeping its own pixel range (and, for
    /// a band, its own padding).
    ///
    /// `None` when the pin and the scale are different kinds — a column that
    /// came back numeric at launch and categorical after a gesture has changed
    /// what the axis IS, and re-domaining across that would place rows by a
    /// rule neither render used. The freshly-inferred scale stands instead.
    #[must_use]
    fn applied_to(&self, scale: &Scale) -> Option<Scale> {
        match (self, scale) {
            (
                Self::Linear(lo, hi),
                Scale::Linear {
                    range_start,
                    range_end,
                    ..
                },
            ) => Some(Scale::Linear {
                domain_min: *lo,
                domain_max: *hi,
                range_start: *range_start,
                range_end: *range_end,
            }),
            (
                Self::Time(lo, hi),
                Scale::Time {
                    range_start,
                    range_end,
                    ..
                },
            ) => Some(Scale::Time {
                domain_min_us: *lo,
                domain_max_us: *hi,
                range_start: *range_start,
                range_end: *range_end,
            }),
            (
                Self::Band(categories),
                Scale::Band {
                    range_start,
                    range_end,
                    padding,
                    ..
                },
            ) => Some(Scale::Band {
                categories: categories.clone(),
                range_start: *range_start,
                range_end: *range_end,
                padding: *padding,
            }),
            _ => None,
        }
    }
}

/// One plot's pinned positional domains, captured from the scales its FIRST
/// composition drew against and re-applied to every later one.
///
/// **The capture moment is Mosaic's, trap included.** Mosaic's `Fixed` fixes a
/// domain after the first render, on whatever data the marks then hold, so a
/// plot whose first render is already filtered pins the filtered domain.
/// brightfield does the same rather than resolving an unfiltered extent the
/// author never asked for: `Fixed` is a portability instruction, and a spec
/// that renders differently here than in Mosaic has failed at the thing the
/// instruction exists for. `deviations.yaml` DEV-0005 records the scope that
/// is NOT read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PinnedDomains {
    /// The x axis's pin, once captured.
    pub x: Option<PinnedDomain>,
    /// The y axis's pin, once captured.
    pub y: Option<PinnedDomain>,
}

impl PinnedDomains {
    /// Whether nothing is pinned — the state of every plot whose spec asks for
    /// no pin, and the state in which [`apply_pinned_domains`] is a no-op.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.x.is_none() && self.y.is_none()
    }

    /// Capture, from `scales`, each axis `request` asks to pin and this does not
    /// hold yet.
    ///
    /// Idempotent after the first capture: an axis already pinned is left
    /// alone, which is what makes this safe to call on every composition and is
    /// where "the FIRST render" is enforced. An axis whose scale is absent (a
    /// mark whose batch was empty) captures nothing and is offered the same
    /// chance on the next composition.
    pub fn capture(&mut self, scales: &ScaleSet, request: FixedDomains) {
        for (wanted, slot, channel) in [
            (request.x, &mut self.x, Channel::X),
            (request.y, &mut self.y, Channel::Y),
        ] {
            if !wanted || slot.is_some() {
                continue;
            }
            *slot = scales.get(channel).and_then(PinnedDomain::of);
        }
    }
}

/// Re-domain `scales`' positional channels onto `pins`, in place.
///
/// A channel with no pin, or whose freshly-inferred scale is a different kind
/// from its pin, is left exactly as inference produced it — so an empty
/// [`PinnedDomains`] leaves the whole set untouched.
pub fn apply_pinned_domains(scales: &mut ScaleSet, pins: &PinnedDomains) {
    for (pin, channel) in [(&pins.x, Channel::X), (&pins.y, Channel::Y)] {
        let Some(pin) = pin else { continue };
        let Some(scale) = scales.get(channel) else {
            continue;
        };
        if let Some(pinned) = pin.applied_to(scale) {
            scales.insert(channel, pinned);
        }
    }
}

/// A plot-level explicit colour-scale override — Mosaic's `colorDomain` /
/// `colorRange` attributes, resolved once at app assembly (literal arrays, or
/// `$param` references into literal-value params — the weather.yaml shape) and
/// applied AFTER scale inference and every mark's `augment_scales`, so the
/// author's explicit domain/range wins over both column inference and the
/// density-family ramp builders.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColourOverride {
    /// Explicit categorical domain (the category ORDER, which fixes each
    /// category's palette slot) from a string-array `colorDomain`.
    pub categories: Option<Vec<String>>,
    /// Explicit continuous domain endpoints from a 2-numeric `colorDomain`.
    pub domain: Option<(f64, f64)>,
    /// Explicit colours from `colorRange` — the categorical palette, or the
    /// evenly-spaced sequential ramp stops (2 endpoints interpolate as a
    /// two-stop ramp; k stops as a k-stop ramp).
    pub range: Option<Vec<[f32; 4]>>,
}

impl ColourOverride {
    /// Whether the override carries nothing to apply.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.categories.is_none() && self.domain.is_none() && self.range.is_none()
    }
}

/// Apply an explicit `colorDomain`/`colorRange` override to the colour-bearing
/// channels (Fill / Stroke) of `set`, in place.
///
/// - A categorical [`Scale::Colour`] takes the override's category order and/or
///   palette (so `colorDomain: [a, b]` + `colorRange: [c1, c2]` pins `a → c1`,
///   `b → c2` regardless of data order).
/// - A continuous [`Scale::Sequential`] takes the override's `[lo, hi]` domain
///   and/or its colours as the evenly-spaced ramp stops.
/// - Positional scales and absent channels are untouched; an override facet
///   that does not fit the scale kind (e.g. `categories` against a Sequential)
///   is ignored.
pub fn apply_colour_override(set: &mut ScaleSet, ov: &ColourOverride) {
    for ch in [Channel::Fill, Channel::Stroke] {
        let Some(scale) = set.get(ch) else { continue };
        let new = match scale {
            Scale::Colour {
                categories,
                palette,
            } => {
                let categories = ov.categories.clone().unwrap_or_else(|| categories.clone());
                let palette = ov.range.clone().unwrap_or_else(|| palette.clone());
                if categories.is_empty() || palette.is_empty() {
                    continue;
                }
                Scale::Colour {
                    categories,
                    palette,
                }
            }
            Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            } => {
                let (domain_min, domain_max) = ov.domain.unwrap_or((*domain_min, *domain_max));
                let stops = match &ov.range {
                    // A ramp needs at least two stops to interpolate.
                    Some(r) if r.len() >= 2 => r.clone(),
                    _ => stops.clone(),
                };
                Scale::Sequential {
                    domain_min,
                    domain_max,
                    stops,
                }
            }
            _ => continue,
        };
        set.insert(ch, new);
    }
}

/// A built-in continuous colour scheme. Wire names are lowercase and
/// Mosaic-aligned, so a `colorScheme:` value stays portable across renderers.
///
/// The default is [`SequentialScheme::Viridis`] — a deliberate divergence from
/// Mosaic/Plot's `turbo` quantitative default. Viridis is perceptually uniform
/// and colourblind-safe; `turbo` stays available by name (see `deviations.yaml`
/// DEV-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SequentialScheme {
    /// Perceptually-uniform, colourblind-safe (matplotlib/ggplot default).
    #[default]
    Viridis,
    /// Single-hue light → dark sequential (ColorBrewer Blues) — the classic
    /// count map, light-anchored.
    Blues,
    /// Mosaic/Plot's declared quantitative default — a rainbow map, included for
    /// spec fidelity.
    Turbo,
    /// The Meridian design system's blue-240 ramp (steps 100..=700) — an
    /// OPT-IN Brightfield-local name; the default stays [`Self::Viridis`].
    /// Non-portable: `serialise_spec` expands it to explicit `colorRange`
    /// stops on export (see deviations.yaml DEV-0004).
    Meridian,
}

impl SequentialScheme {
    /// The lowercase, Mosaic-aligned wire name.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Viridis => "viridis",
            Self::Blues => "blues",
            Self::Turbo => "turbo",
            Self::Meridian => "meridian",
        }
    }

    /// The next scheme in the transient colour-cycle:
    /// Viridis → Blues → Turbo → Meridian → Viridis. The single source of
    /// truth for the cycle order.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Viridis => Self::Blues,
            Self::Blues => Self::Turbo,
            Self::Turbo => Self::Meridian,
            Self::Meridian => Self::Viridis,
        }
    }

    /// Parse a wire name (case-exact). `None` for an unrecognised scheme — the
    /// caller warns and falls back to the default.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "viridis" => Some(Self::Viridis),
            "blues" => Some(Self::Blues),
            "turbo" => Some(Self::Turbo),
            "meridian" => Some(Self::Meridian),
            _ => None,
        }
    }

    /// Evenly-spaced RGBA control points (low → high), interpolated by
    /// [`Scale::map_continuous`]. Nine hand-transcribed points per classic
    /// scheme (a full 256-entry LUT is a later refinement); meridian carries
    /// the design crate's thirteen published steps verbatim.
    #[must_use]
    pub fn stops(self) -> Vec<[f32; 4]> {
        match self {
            Self::Viridis => VIRIDIS_STOPS.to_vec(),
            Self::Blues => BLUES_STOPS.to_vec(),
            Self::Turbo => TURBO_STOPS.to_vec(),
            Self::Meridian => MERIDIAN_STOPS.to_vec(),
        }
    }
}

/// Viridis control points (matplotlib, 9-class), dark purple → bright yellow.
const VIRIDIS_STOPS: &[[f32; 4]] = &[
    [0.267, 0.004, 0.329, 1.0], // #440154
    [0.278, 0.176, 0.482, 1.0], // #472d7b
    [0.231, 0.322, 0.545, 1.0], // #3b528b
    [0.173, 0.447, 0.557, 1.0], // #2c728e
    [0.129, 0.569, 0.549, 1.0], // #21918c
    [0.157, 0.682, 0.502, 1.0], // #28ae80
    [0.369, 0.788, 0.384, 1.0], // #5ec962
    [0.678, 0.863, 0.188, 1.0], // #addc30
    [0.992, 0.906, 0.145, 1.0], // #fde725
];

/// Blues control points (ColorBrewer sequential, 9-class), near-white → navy.
const BLUES_STOPS: &[[f32; 4]] = &[
    [0.969, 0.984, 1.000, 1.0],  // #f7fbff
    [0.871, 0.922, 0.969, 1.0],  // #deebf7
    [0.776, 0.859, 0.937, 1.0],  // #c6dbef
    [0.620, 0.792, 0.882, 1.0],  // #9ecae1
    [0.420, 0.682, 0.839, 1.0],  // #6baed6
    [0.259, 0.573, 0.776, 1.0],  // #4292c6
    [0.129, 0.443, 0.710, 1.0],  // #2171b5
    [0.031, 0.3176, 0.612, 1.0], // #08519c
    [0.031, 0.188, 0.420, 1.0],  // #08306b
];

/// Turbo control points (Google turbo, 9-sample), purple → blue → green →
/// yellow → dark red.
const TURBO_STOPS: &[[f32; 4]] = &[
    [0.190, 0.072, 0.232, 1.0], // #30123b
    [0.246, 0.395, 0.832, 1.0], // #3f65d4
    [0.239, 0.657, 0.985, 1.0], // #3ea8fb
    [0.180, 0.902, 0.769, 1.0], // #2ee6c4
    [0.427, 0.988, 0.475, 1.0], // #6dfc79
    [0.760, 0.965, 0.235, 1.0], // #c2f63c
    [0.973, 0.798, 0.155, 1.0], // #f8cc28
    [0.960, 0.446, 0.104, 1.0], // #f5721a
    [0.480, 0.016, 0.011, 1.0], // #7a0403
];

/// The Meridian sequential ramp (blue-240, steps 100..=700) — the design
/// crate's thirteen published control points, converted once at compile time.
const MERIDIAN_STOPS: &[[f32; 4]] =
    &crate::ink::components(meridian_design::viz::SEQUENTIAL_MERIDIAN);

/// Default colour palette — the Meridian "Harbour" categorical order (blue,
/// gold, teal, red, violet, orange, plum, green), replacing Observable Plot's
/// categorical10. The ORDER is the colourblind-safety mechanism (chosen for
/// maximum adjacent CVD distance) and is therefore data, never cosmetic —
/// eight slots; `map_colour` cycles by index beyond them (see deviations.yaml
/// DEV-0004).
const CATEGORICAL_PALETTE: &[[f32; 4]] =
    &crate::ink::components(meridian_design::viz::CATEGORICAL_LIGHT);

/// Infer scales from a RecordBatch and ChannelMap.
///
/// For each channel in the map, examines the corresponding Arrow column type:
/// - Float64 / Int64 / Int32 / Int16 / numeric -> LinearScale
/// - Utf8 / string -> BandScale
/// - Timestamp -> TimeScale
///
/// `x_range` and `y_range` are the pixel ranges for x and y axes respectively.
/// Fold any literal channel values (e.g. `y: 0`) into the scale set so a
/// constant-positioned mark (like a baseline rule) is placed correctly. A
/// literal on a positional axis extends an existing Linear scale's domain to
/// include the value (so an off-range literal stays on-plot), or — when no
/// column gave that axis a scale — synthesises a Linear scale around the value.
/// Non-linear (Band/Time/Colour) scales are left unchanged.
fn extend_scales_with_literals<I: Iterator<Item = (Channel, f64)>>(
    set: &mut ScaleSet,
    literals: I,
    x_range: (f64, f64),
    y_range: (f64, f64),
) {
    for (channel, value) in literals {
        let (range_start, range_end) = match channel {
            Channel::X | Channel::X1 | Channel::X2 => x_range,
            Channel::Y | Channel::Y1 | Channel::Y2 => y_range,
            _ => continue, // literals only position on x/y axes
        };
        let new_scale = match set.get(channel) {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                ..
            }) => Some(Scale::Linear {
                domain_min: domain_min.min(value),
                domain_max: domain_max.max(value),
                range_start,
                range_end,
            }),
            Some(_) => None, // non-linear axis: can't merge a numeric literal
            None => {
                // No column scale on this axis — synthesise one spanning 0..value
                // (baseline-friendly), guarding against a zero span.
                let (lo, hi) = (value.min(0.0), value.max(0.0));
                let (lo, hi) = if (hi - lo).abs() < f64::EPSILON {
                    (value - 1.0, value + 1.0)
                } else {
                    (lo, hi)
                };
                Some(Scale::Linear {
                    domain_min: lo,
                    domain_max: hi,
                    range_start,
                    range_end,
                })
            }
        };
        if let Some(s) = new_scale {
            set.insert(channel, s);
        }
    }
}

/// Insert or widen a Linear scale on `channel` so its domain spans `[min, max]`
/// over the given pixel `range`.
///
/// Statistical marks build positional scales from emitted data extents rather
/// than an inferable column (e.g. regression's x/y come from `x_min`/`x_max`
/// aggregates — the executed batch has no raw x/y column). When a sibling mark
/// already established a Linear scale on the channel, the domain is unioned so
/// co-rendered marks share one axis; an existing non-Linear (Band/Time/Colour)
/// scale is left untouched.
pub fn merge_linear_scale(
    set: &mut ScaleSet,
    channel: Channel,
    min: f64,
    max: f64,
    range: (f64, f64),
) {
    let (domain_min, domain_max) = match set.get(channel) {
        Some(Scale::Linear {
            domain_min,
            domain_max,
            ..
        }) => (domain_min.min(min), domain_max.max(max)),
        Some(_) => return, // non-linear axis already established
        None => (min, max),
    };
    set.insert(
        channel,
        Scale::Linear {
            domain_min,
            domain_max,
            range_start: range.0,
            range_end: range.1,
        },
    );
}

pub fn infer_scales(
    batch: &RecordBatch,
    channel_map: &ChannelMap,
    x_range: (f64, f64),
    y_range: (f64, f64),
) -> ScaleSet {
    let mut set = ScaleSet::new();

    for (channel, col_name) in channel_map.iter() {
        let col_idx = match batch.schema().index_of(col_name) {
            Ok(idx) => idx,
            Err(_) => continue,
        };
        let col = batch.column(col_idx);
        let (range_start, range_end) = match channel {
            Channel::X | Channel::X1 | Channel::X2 => x_range,
            Channel::Y | Channel::Y1 | Channel::Y2 => y_range,
            _ => (0.0, 0.0),
        };

        let scale = infer_column_scale(col.as_ref(), range_start, range_end, *channel);
        if let Some(s) = scale {
            set.insert(*channel, s);
        }
    }

    extend_scales_with_literals(&mut set, channel_map.literals_iter(), x_range, y_range);
    set
}

/// Infer scales from multiple (RecordBatch, ChannelMap) pairs with unioned domains.
///
/// For each channel that appears in any channel map, collects domain values from
/// all batches and produces a single scale spanning the combined range:
/// - Linear: min(all_mins), max(all_maxes)
/// - Band: set union of categories, preserving insertion order
/// - Colour: set union of categories, then [`order_categories`]
/// - Time: min(all_mins), max(all_maxes)
///
/// The existing `infer_scales()` is unchanged.
pub fn infer_scales_multi(
    entries: &[(&RecordBatch, &ChannelMap)],
    x_range: (f64, f64),
    y_range: (f64, f64),
) -> ScaleSet {
    // Collect all channels across all entries.
    let mut all_channels: Vec<Channel> = Vec::new();
    for (_, cm) in entries {
        for (ch, _) in cm.iter() {
            if !all_channels.contains(ch) {
                all_channels.push(*ch);
            }
        }
    }

    let mut set = ScaleSet::new();

    for channel in &all_channels {
        let (range_start, range_end) = match channel {
            Channel::X | Channel::X1 | Channel::X2 => x_range,
            Channel::Y | Channel::Y1 | Channel::Y2 => y_range,
            _ => (0.0, 0.0),
        };

        // Collect per-batch scales for this channel and union them.
        let mut scales_for_channel: Vec<Scale> = Vec::new();
        for (batch, cm) in entries {
            if let Some(col_name) = cm.get(*channel) {
                let col_idx = match batch.schema().index_of(col_name) {
                    Ok(idx) => idx,
                    Err(_) => continue,
                };
                let col = batch.column(col_idx);
                if let Some(s) = infer_column_scale(col.as_ref(), range_start, range_end, *channel)
                {
                    scales_for_channel.push(s);
                }
            }
        }

        if let Some(merged) = union_scales(&scales_for_channel, range_start, range_end) {
            set.insert(*channel, merged);
        }
    }

    for (_, cm) in entries {
        extend_scales_with_literals(&mut set, cm.literals_iter(), x_range, y_range);
    }
    set
}

/// Union a list of scales of the same type into a single scale.
fn union_scales(scales: &[Scale], range_start: f64, range_end: f64) -> Option<Scale> {
    if scales.is_empty() {
        return None;
    }

    match &scales[0] {
        Scale::Linear { .. } => {
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for s in scales {
                if let Scale::Linear {
                    domain_min,
                    domain_max,
                    ..
                } = s
                {
                    if *domain_min < min {
                        min = *domain_min;
                    }
                    if *domain_max > max {
                        max = *domain_max;
                    }
                }
            }
            if min.is_infinite() {
                None
            } else {
                Some(Scale::Linear {
                    domain_min: min,
                    domain_max: max,
                    range_start,
                    range_end,
                })
            }
        }
        Scale::Band { padding, .. } => {
            let padding = *padding;
            let mut categories: Vec<String> = Vec::new();
            for s in scales {
                if let Scale::Band {
                    categories: cats, ..
                } = s
                {
                    for cat in cats {
                        if !categories.contains(cat) {
                            categories.push(cat.clone());
                        }
                    }
                }
            }
            Some(Scale::Band {
                categories,
                range_start,
                range_end,
                padding,
            })
        }
        Scale::Colour { palette, .. } => {
            let palette = palette.clone();
            let mut categories: Vec<String> = Vec::new();
            for s in scales {
                if let Scale::Colour {
                    categories: cats, ..
                } = s
                {
                    for cat in cats {
                        if !categories.contains(cat) {
                            categories.push(cat.clone());
                        }
                    }
                }
            }
            // The union of two ordered lists is not ordered, so the rule is
            // re-applied to the merged set rather than inherited from the parts.
            order_categories(&mut categories);
            Some(Scale::Colour {
                categories,
                palette,
            })
        }
        Scale::Time { .. } => {
            let mut min = i64::MAX;
            let mut max = i64::MIN;
            for s in scales {
                if let Scale::Time {
                    domain_min_us,
                    domain_max_us,
                    ..
                } = s
                {
                    if *domain_min_us < min {
                        min = *domain_min_us;
                    }
                    if *domain_max_us > max {
                        max = *domain_max_us;
                    }
                }
            }
            if min == i64::MAX {
                None
            } else {
                Some(Scale::Time {
                    domain_min_us: min,
                    domain_max_us: max,
                    range_start,
                    range_end,
                })
            }
        }
        Scale::Sequential { stops, .. } => {
            // Union the ramp extents by min-of-mins / max-of-maxes, keeping the
            // first scale's stops (co-rendered rasters share one scheme).
            let stops = stops.clone();
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for s in scales {
                if let Scale::Sequential {
                    domain_min,
                    domain_max,
                    ..
                } = s
                {
                    if *domain_min < min {
                        min = *domain_min;
                    }
                    if *domain_max > max {
                        max = *domain_max;
                    }
                }
            }
            if min.is_infinite() {
                None
            } else {
                Some(Scale::Sequential {
                    domain_min: min,
                    domain_max: max,
                    stops,
                })
            }
        }
    }
}

/// The kind of a positional axis, classified from its bound columns' Arrow
/// types WITHOUT building scales — a datatype peek mirroring the private
/// `infer_column_scale`'s arms. Used to decide the default axis inset
/// (axis-inset round) before ranges are fed to scale inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisClass {
    /// A linear or time scale (numeric / timestamp column). Gets the default
    /// inset on non-zero-baseline ends.
    Continuous,
    /// A band scale (`Utf8` column). Gets no default inset (band `padding`
    /// already owns categorical edge spacing); explicit insets still apply.
    Band,
}

/// Classify a positional axis (the `X`/`X1`/`X2` or `Y`/`Y1`/`Y2` family) as
/// continuous or band by peeking the bound columns' Arrow `DataType` across all
/// mark entries — no value scan, no scale built. `Continuous` wins over `Band`
/// if any mark binds a numeric/time column to the axis (a mixed axis is
/// continuous). Returns `None` when no mark binds a column to the axis: an
/// augment-only axis (regression/1-D density perpendicular) or an absent one —
/// no default inset is applied there (conservative: never floats a baseline we
/// can't see).
pub fn positional_axis_class(
    entries: &[(&RecordBatch, &ChannelMap)],
    axis: Channel,
) -> Option<AxisClass> {
    let family: &[Channel] = match axis {
        Channel::X => &[Channel::X, Channel::X1, Channel::X2],
        Channel::Y => &[Channel::Y, Channel::Y1, Channel::Y2],
        _ => return None,
    };
    let mut saw_continuous = false;
    let mut saw_band = false;
    for (batch, cm) in entries {
        for ch in family {
            let Some(col_name) = cm.get(*ch) else {
                continue;
            };
            let Ok(idx) = batch.schema().index_of(col_name) else {
                continue;
            };
            match batch.column(idx).data_type() {
                DataType::Utf8 => saw_band = true,
                DataType::Float64
                | DataType::Int64
                | DataType::Int32
                | DataType::Int16
                | DataType::Timestamp(TimeUnit::Microsecond, _) => saw_continuous = true,
                _ => {}
            }
        }
    }
    if saw_continuous {
        Some(AxisClass::Continuous)
    } else if saw_band {
        Some(AxisClass::Band)
    } else {
        None
    }
}

fn infer_column_scale(
    col: &dyn Array,
    range_start: f64,
    range_end: f64,
    channel: Channel,
) -> Option<Scale> {
    match col.data_type() {
        DataType::Float64 => {
            let arr = col.as_any().downcast_ref::<Float64Array>()?;
            let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min.is_infinite() {
                return None;
            }
            Some(Scale::Linear {
                domain_min: min,
                domain_max: max,
                range_start,
                range_end,
            })
        }
        DataType::Int64 => {
            let arr = col.as_any().downcast_ref::<arrow::array::Int64Array>()?;
            let (mut min, mut max) = (i64::MAX, i64::MIN);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min == i64::MAX {
                return None;
            }
            Some(Scale::Linear {
                domain_min: min as f64,
                domain_max: max as f64,
                range_start,
                range_end,
            })
        }
        DataType::Int32 => {
            let arr = col.as_any().downcast_ref::<arrow::array::Int32Array>()?;
            let (mut min, mut max) = (i32::MAX, i32::MIN);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min == i32::MAX {
                return None;
            }
            Some(Scale::Linear {
                domain_min: min as f64,
                domain_max: max as f64,
                range_start,
                range_end,
            })
        }
        DataType::Int16 => {
            let arr = col.as_any().downcast_ref::<arrow::array::Int16Array>()?;
            let (mut min, mut max) = (i16::MAX, i16::MIN);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min == i16::MAX {
                return None;
            }
            Some(Scale::Linear {
                domain_min: min as f64,
                domain_max: max as f64,
                range_start,
                range_end,
            })
        }
        DataType::Utf8 => {
            let arr = col.as_any().downcast_ref::<StringArray>().unwrap();
            let mut categories = Vec::new();
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i).to_string();
                    if !categories.contains(&v) {
                        categories.push(v);
                    }
                }
            }
            if matches!(channel, Channel::Fill | Channel::Stroke) {
                order_categories(&mut categories);
                Some(Scale::Colour {
                    palette: CATEGORICAL_PALETTE.to_vec(),
                    categories,
                })
            } else {
                Some(Scale::Band {
                    categories,
                    range_start,
                    range_end,
                    padding: 0.1,
                })
            }
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let arr = col.as_any().downcast_ref::<TimestampMicrosecondArray>()?;
            let (mut min, mut max) = (i64::MAX, i64::MIN);
            for i in 0..arr.len() {
                if !arr.is_null(i) {
                    let v = arr.value(i);
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
            }
            if min == i64::MAX {
                return None;
            }
            Some(Scale::Time {
                domain_min_us: min,
                domain_max_us: max,
                range_start,
                range_end,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, StringArray, TimestampMicrosecondArray};
    use arrow::datatypes::{Field, Schema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    fn make_numeric_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap()
    }

    fn make_categorical_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0])),
            ],
        )
        .unwrap()
    }

    fn make_time_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "ts",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new("value", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampMicrosecondArray::from(vec![
                    1_000_000, 2_000_000, 3_000_000, 4_000_000,
                ])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 15.0, 25.0])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn infer_linear_scales() {
        let batch = make_numeric_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (400.0, 20.0));

        let x = scales.get(Channel::X).expect("x scale should exist");
        match x {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => {
                assert!((domain_min - 1.0).abs() < f64::EPSILON);
                assert!((domain_max - 3.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Linear scale for x, got: {other:?}"),
        }

        let y = scales.get(Channel::Y).expect("y scale should exist");
        match y {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => {
                assert!((domain_min - 10.0).abs() < f64::EPSILON);
                assert!((domain_max - 30.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Linear scale for y, got: {other:?}"),
        }
    }

    #[test]
    fn infer_band_scale() {
        let batch = make_categorical_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "category".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (400.0, 20.0));

        let x = scales.get(Channel::X).expect("x scale should exist");
        match x {
            Scale::Band { categories, .. } => {
                assert_eq!(categories, &["a", "b", "c"]);
            }
            other => panic!("expected Band scale for x, got: {other:?}"),
        }
    }

    #[test]
    fn infer_time_scale() {
        let batch = make_time_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "ts".to_string());
        cm.insert(Channel::Y, "value".to_string());

        let scales = infer_scales(&batch, &cm, (40.0, 600.0), (400.0, 20.0));

        let x = scales.get(Channel::X).expect("x scale should exist");
        match x {
            Scale::Time {
                domain_min_us,
                domain_max_us,
                ..
            } => {
                assert_eq!(*domain_min_us, 1_000_000);
                assert_eq!(*domain_max_us, 4_000_000);
            }
            other => panic!("expected Time scale for x, got: {other:?}"),
        }
    }

    #[test]
    fn infer_colour_scale_for_fill_channel() {
        let batch = make_categorical_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::Fill, "category".to_string());

        let scales = infer_scales(&batch, &cm, (0.0, 0.0), (0.0, 0.0));

        let fill = scales.get(Channel::Fill).expect("fill scale should exist");
        match fill {
            Scale::Colour {
                categories,
                palette,
            } => {
                assert_eq!(categories, &["a", "b", "c"]);
                assert!(!palette.is_empty());
            }
            other => panic!("expected Colour scale for fill, got: {other:?}"),
        }
    }

    #[test]
    fn linear_scale_maps_correctly() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 0.0,
            range_end: 500.0,
        };
        assert!((scale.map_f64(0.0) - 0.0).abs() < f64::EPSILON);
        assert!((scale.map_f64(50.0) - 250.0).abs() < f64::EPSILON);
        assert!((scale.map_f64(100.0) - 500.0).abs() < f64::EPSILON);
    }

    // --- ViewExtent ---

    #[test]
    fn view_extent_with_both_axes() {
        let ve = ViewExtent {
            x: Some((10.0, 50.0)),
            y: Some((100.0, 200.0)),
        };
        assert_eq!(ve.x, Some((10.0, 50.0)));
        assert_eq!(ve.y, Some((100.0, 200.0)));
    }

    #[test]
    fn view_extent_with_none_axes() {
        let ve = ViewExtent::default();
        assert_eq!(ve.x, None);
        assert_eq!(ve.y, None);
    }

    #[test]
    fn view_extent_partial() {
        let ve = ViewExtent {
            x: Some((1.0, 2.0)),
            y: None,
        };
        assert!(ve.x.is_some());
        assert!(ve.y.is_none());
    }

    // --- Scale::inverse_f64 ---

    #[test]
    fn linear_inverse_at_endpoints() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 0.0,
            range_end: 500.0,
        };
        let inv_min = scale.inverse_f64(0.0).expect("linear should return Some");
        let inv_max = scale.inverse_f64(500.0).expect("linear should return Some");
        assert!((inv_min - 0.0).abs() < f64::EPSILON);
        assert!((inv_max - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn linear_inverse_at_midpoint() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 0.0,
            range_end: 500.0,
        };
        let inv_mid = scale.inverse_f64(250.0).expect("linear should return Some");
        assert!((inv_mid - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn linear_inverse_roundtrip() {
        let scale = Scale::Linear {
            domain_min: 10.0,
            domain_max: 90.0,
            range_start: 40.0,
            range_end: 600.0,
        };
        let value = 55.0;
        let pixel = scale.map_f64(value);
        let roundtrip = scale.inverse_f64(pixel).unwrap();
        assert!((roundtrip - value).abs() < 1e-10);
    }

    #[test]
    fn time_inverse() {
        let scale = Scale::Time {
            domain_min_us: 1_000_000,
            domain_max_us: 4_000_000,
            range_start: 0.0,
            range_end: 300.0,
        };
        let inv = scale.inverse_f64(100.0).expect("time should return Some");
        // At 1/3 of the range => 1/3 of domain span
        let expected = 1_000_000.0 + (3_000_000.0 / 3.0);
        assert!((inv - expected).abs() < 1.0);
    }

    #[test]
    fn band_inverse_returns_none() {
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string()],
            range_start: 0.0,
            range_end: 200.0,
            padding: 0.1,
        };
        assert!(scale.inverse_f64(100.0).is_none());
    }

    #[test]
    fn colour_inverse_returns_none() {
        let scale = Scale::Colour {
            categories: vec!["a".to_string()],
            palette: vec![[1.0, 0.0, 0.0, 1.0]],
        };
        assert!(scale.inverse_f64(50.0).is_none());
    }

    #[test]
    fn band_scale_maps_categories() {
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            range_start: 0.0,
            range_end: 300.0,
            padding: 0.1,
        };
        let a_pos = scale.map_category("a").expect("a should map");
        let b_pos = scale.map_category("b").expect("b should map");
        let c_pos = scale.map_category("c").expect("c should map");
        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
        assert!(scale.map_category("d").is_none());

        let bw = scale.band_width().expect("should have band width");
        assert!(bw > 0.0);
        assert!(bw < 100.0); // each band is 100px wide, with 10% padding -> 90px
    }

    // --- infer_scales_multi ---

    #[test]
    fn multi_unions_linear_domains() {
        // Batch 1: x in [1, 5], y in [10, 50]
        let schema1 = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch1 = RecordBatch::try_new(
            schema1,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 5.0])),
                Arc::new(Float64Array::from(vec![10.0, 50.0])),
            ],
        )
        .unwrap();

        // Batch 2: x in [3, 8], y in [5, 30]
        let schema2 = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        let batch2 = RecordBatch::try_new(
            schema2,
            vec![
                Arc::new(Float64Array::from(vec![3.0, 8.0])),
                Arc::new(Float64Array::from(vec![5.0, 30.0])),
            ],
        )
        .unwrap();

        let mut cm1 = ChannelMap::new();
        cm1.insert(Channel::X, "x".to_string());
        cm1.insert(Channel::Y, "y".to_string());
        let mut cm2 = ChannelMap::new();
        cm2.insert(Channel::X, "x".to_string());
        cm2.insert(Channel::Y, "y".to_string());

        let entries: Vec<(&RecordBatch, &ChannelMap)> = vec![(&batch1, &cm1), (&batch2, &cm2)];
        let scales = infer_scales_multi(&entries, (40.0, 600.0), (450.0, 20.0));

        let x = scales.get(Channel::X).expect("x scale should exist");
        match x {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => {
                assert!(
                    (domain_min - 1.0).abs() < f64::EPSILON,
                    "x min should be 1.0"
                );
                assert!(
                    (domain_max - 8.0).abs() < f64::EPSILON,
                    "x max should be 8.0"
                );
            }
            other => panic!("expected Linear scale for x, got: {other:?}"),
        }

        let y = scales.get(Channel::Y).expect("y scale should exist");
        match y {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => {
                assert!(
                    (domain_min - 5.0).abs() < f64::EPSILON,
                    "y min should be 5.0"
                );
                assert!(
                    (domain_max - 50.0).abs() < f64::EPSILON,
                    "y max should be 50.0"
                );
            }
            other => panic!("expected Linear scale for y, got: {other:?}"),
        }
    }

    #[test]
    fn multi_unions_categorical_fill() {
        let schema1 = Arc::new(Schema::new(vec![Field::new(
            "category",
            DataType::Utf8,
            false,
        )]));
        let batch1 = RecordBatch::try_new(
            schema1,
            vec![Arc::new(StringArray::from(vec!["red", "blue"]))],
        )
        .unwrap();

        let schema2 = Arc::new(Schema::new(vec![Field::new(
            "category",
            DataType::Utf8,
            false,
        )]));
        let batch2 = RecordBatch::try_new(
            schema2,
            vec![Arc::new(StringArray::from(vec!["blue", "green"]))],
        )
        .unwrap();

        let mut cm1 = ChannelMap::new();
        cm1.insert(Channel::Fill, "category".to_string());
        let mut cm2 = ChannelMap::new();
        cm2.insert(Channel::Fill, "category".to_string());

        let entries: Vec<(&RecordBatch, &ChannelMap)> = vec![(&batch1, &cm1), (&batch2, &cm2)];
        let scales = infer_scales_multi(&entries, (0.0, 0.0), (0.0, 0.0));

        let fill = scales.get(Channel::Fill).expect("fill scale should exist");
        match fill {
            Scale::Colour { categories, .. } => {
                // Union of {red, blue} and {blue, green} = {red, blue, green}
                assert_eq!(categories.len(), 3);
                assert!(categories.contains(&"red".to_string()));
                assert!(categories.contains(&"blue".to_string()));
                assert!(categories.contains(&"green".to_string()));
            }
            other => panic!("expected Colour scale for fill, got: {other:?}"),
        }
    }

    #[test]
    fn multi_single_entry_matches_infer_scales() {
        let batch = make_numeric_batch();
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let single = infer_scales(&batch, &cm, (40.0, 600.0), (400.0, 20.0));
        let multi = infer_scales_multi(&[(&batch, &cm)], (40.0, 600.0), (400.0, 20.0));

        // Both should produce identical domains
        let sx = single.get(Channel::X).unwrap();
        let mx = multi.get(Channel::X).unwrap();
        assert!((sx.domain_min().unwrap() - mx.domain_min().unwrap()).abs() < f64::EPSILON);
        assert!((sx.domain_max().unwrap() - mx.domain_max().unwrap()).abs() < f64::EPSILON);
    }

    #[test]
    fn merge_linear_scale_inserts_unions_and_skips_nonlinear() {
        let range = (0.0, 100.0);

        // Absent → insert.
        let mut set = ScaleSet::new();
        merge_linear_scale(&mut set, Channel::X, 2.0, 8.0, range);
        match set.get(Channel::X).expect("x scale inserted") {
            Scale::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            } => {
                assert_eq!((*domain_min, *domain_max), (2.0, 8.0));
                assert_eq!((*range_start, *range_end), (0.0, 100.0));
            }
            other => panic!("expected Linear, got {other:?}"),
        }

        // Present Linear → union (widen) on both ends.
        merge_linear_scale(&mut set, Channel::X, 1.0, 5.0, range);
        merge_linear_scale(&mut set, Channel::X, 4.0, 12.0, range);
        match set.get(Channel::X).unwrap() {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => assert_eq!((*domain_min, *domain_max), (1.0, 12.0)),
            other => panic!("expected Linear, got {other:?}"),
        }

        // Present non-Linear → left untouched (don't clobber a Band axis).
        let mut set2 = ScaleSet::new();
        set2.insert(
            Channel::X,
            Scale::Band {
                categories: vec!["a".to_string(), "b".to_string()],
                range_start: 0.0,
                range_end: 10.0,
                padding: 0.1,
            },
        );
        merge_linear_scale(&mut set2, Channel::X, 1.0, 9.0, range);
        assert!(
            matches!(set2.get(Channel::X).unwrap(), Scale::Band { .. }),
            "non-linear scale must not be overwritten by merge_linear_scale"
        );
    }

    // --- Scale::Sequential + map_continuous ---

    #[test]
    fn map_continuous_interpolates_and_clamps() {
        let black = [0.0, 0.0, 0.0, 1.0];
        let white = [1.0, 1.0, 1.0, 1.0];
        let scale = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 10.0,
            stops: vec![black, white],
        };

        // Endpoints return the first/last stop exactly.
        assert_eq!(scale.map_continuous(0.0), black);
        assert_eq!(scale.map_continuous(10.0), white);

        // Midpoint of a 2-stop ramp is the channel-wise average.
        let mid = scale.map_continuous(5.0);
        for (c, v) in mid.iter().take(3).enumerate() {
            assert!((v - 0.5).abs() < 1e-6, "channel {c} mid = {v}");
        }

        // Out-of-domain values clamp to the endpoints.
        assert_eq!(scale.map_continuous(-5.0), black);
        assert_eq!(scale.map_continuous(42.0), white);

        // A degenerate (min == max) domain returns the top stop.
        let degenerate = Scale::Sequential {
            domain_min: 3.0,
            domain_max: 3.0,
            stops: vec![black, white],
        };
        assert_eq!(degenerate.map_continuous(3.0), white);
    }

    #[test]
    fn map_continuous_locates_correct_bracket() {
        // Three stops over [0, 2]: red, green, blue. A value at t=0.75 sits in the
        // second segment (green → blue) three-quarters along.
        let scale = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 2.0,
            stops: vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ],
        };
        let c = scale.map_continuous(1.5); // t = 0.75 → seg 1, frac 0.5
        assert!((c[0] - 0.0).abs() < 1e-6);
        assert!((c[1] - 0.5).abs() < 1e-6, "green = {}", c[1]);
        assert!((c[2] - 0.5).abs() < 1e-6, "blue = {}", c[2]);
    }

    // --- SequentialScheme ---

    #[test]
    fn scheme_stops_and_wire_roundtrip() {
        for scheme in [
            SequentialScheme::Viridis,
            SequentialScheme::Blues,
            SequentialScheme::Turbo,
            SequentialScheme::Meridian,
        ] {
            let stops = scheme.stops();
            assert!(stops.len() >= 5, "{scheme:?} has >= 5 stops");
            for s in &stops {
                for &c in s {
                    assert!(
                        (0.0..=1.0).contains(&c),
                        "{scheme:?} component {c} in range"
                    );
                }
            }
            assert_eq!(
                SequentialScheme::from_wire(scheme.wire_name()),
                Some(scheme),
                "{scheme:?} round-trips through its wire name"
            );
        }
        // Unknown / wrong-case names yield None; the caller warns + defaults.
        assert_eq!(SequentialScheme::from_wire("magma"), None);
        assert_eq!(SequentialScheme::from_wire("Viridis"), None);
        // The default scheme is viridis.
        assert_eq!(SequentialScheme::default(), SequentialScheme::Viridis);
    }

    #[test]
    fn next_cycles_viridis_blues_turbo_meridian() {
        // The transient colour-cycle order (meridian added
        // by design phase 4 PR B), wrapping back to the start after four
        // presses.
        assert_eq!(SequentialScheme::Viridis.next(), SequentialScheme::Blues);
        assert_eq!(SequentialScheme::Blues.next(), SequentialScheme::Turbo);
        assert_eq!(SequentialScheme::Turbo.next(), SequentialScheme::Meridian);
        assert_eq!(SequentialScheme::Meridian.next(), SequentialScheme::Viridis);
        // Four cycles from any start return to it.
        for start in [
            SequentialScheme::Viridis,
            SequentialScheme::Blues,
            SequentialScheme::Turbo,
            SequentialScheme::Meridian,
        ] {
            assert_eq!(
                start.next().next().next().next(),
                start,
                "{start:?} cycles in 4"
            );
        }
    }

    // --- design phase 4 PR B: the meridian scheme + Harbour palette carry the
    //     design crate's published values verbatim ---

    #[test]
    fn dsb_meridian_stops_match_design_crate() {
        let stops = SequentialScheme::Meridian.stops();
        let src = meridian_design::viz::SEQUENTIAL_MERIDIAN;
        assert_eq!(stops.len(), src.len(), "all 13 published steps carried");
        for (i, (stop, token)) in stops.iter().zip(src.iter()).enumerate() {
            assert_eq!(
                *stop,
                [token.r, token.g, token.b, token.a],
                "meridian stop {i} equals the design token"
            );
        }
        // The DEFAULT stays viridis — meridian is opt-in by name only.
        assert_eq!(SequentialScheme::default(), SequentialScheme::Viridis);
    }

    #[test]
    fn dsb_categorical_palette_is_harbour() {
        let src = meridian_design::viz::CATEGORICAL_LIGHT;
        assert_eq!(CATEGORICAL_PALETTE.len(), src.len(), "8 Harbour slots");
        for (i, (slot, token)) in CATEGORICAL_PALETTE.iter().zip(src.iter()).enumerate() {
            assert_eq!(
                *slot,
                [token.r, token.g, token.b, token.a],
                "Harbour slot {i} equals the design token (order is load-bearing)"
            );
        }
        // A 9th category cycles back to slot 1 (index % len).
        let scale = Scale::Colour {
            categories: (0..9).map(|i| format!("c{i}")).collect(),
            palette: CATEGORICAL_PALETTE.to_vec(),
        };
        assert_eq!(
            scale.map_colour("c8"),
            Some(CATEGORICAL_PALETTE[0]),
            "9th category wraps to slot 1"
        );
    }

    #[test]
    fn dsb_spec_export_hex_agrees_with_design_crate() {
        // brightfield-spec carries the meridian ramp as CSS hex strings (the
        // serialise-time colorRange expansion) WITHOUT depending on the design
        // crate; this render-side test pins the two byte-equal so they can't
        // drift.
        let hex = brightfield_spec::parse::MERIDIAN_COLOR_RANGE_HEX;
        let src = meridian_design::viz::SEQUENTIAL_MERIDIAN;
        assert_eq!(hex.len(), src.len());
        for (i, (h, token)) in hex.iter().zip(src.iter()).enumerate() {
            assert_eq!(
                *h,
                token.hex(),
                "export hex stop {i} equals the design token"
            );
        }
    }

    // --- design phase 4 PR B: explicit colorDomain/colorRange overrides ---

    #[test]
    fn dsb_colour_override_pins_categorical_domain_and_range() {
        // Data-order inference gave b-then-a; the explicit override pins the
        // author's domain order and palette, so a → #111111, b → #222222
        // regardless of data arrival order.
        let mut set = ScaleSet::new();
        set.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["b".into(), "a".into()],
                palette: CATEGORICAL_PALETTE.to_vec(),
            },
        );
        let c1 = [0x11 as f32 / 255.0; 3];
        let c2 = [0x22 as f32 / 255.0; 3];
        let ov = ColourOverride {
            categories: Some(vec!["a".into(), "b".into()]),
            domain: None,
            range: Some(vec![[c1[0], c1[1], c1[2], 1.0], [c2[0], c2[1], c2[2], 1.0]]),
        };
        apply_colour_override(&mut set, &ov);
        let scale = set.get(Channel::Fill).expect("fill scale kept");
        assert_eq!(scale.map_colour("a"), Some([c1[0], c1[1], c1[2], 1.0]));
        assert_eq!(scale.map_colour("b"), Some([c2[0], c2[1], c2[2], 1.0]));
    }

    #[test]
    fn dsb_colour_override_two_stop_ramp_and_domain() {
        // A 2-endpoint colorRange becomes a two-stop ramp; a 2-numeric
        // colorDomain replaces the inferred endpoints.
        let mut set = ScaleSet::new();
        set.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 0.0,
                domain_max: 40.0,
                stops: SequentialScheme::Viridis.stops(),
            },
        );
        let lo = [0.0, 0.0, 0.0, 1.0];
        let hi = [1.0, 1.0, 1.0, 1.0];
        let ov = ColourOverride {
            categories: None,
            domain: Some((0.0, 100.0)),
            range: Some(vec![lo, hi]),
        };
        apply_colour_override(&mut set, &ov);
        let scale = set.get(Channel::Fill).expect("fill scale kept");
        match scale {
            Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            } => {
                assert_eq!(
                    (*domain_min, *domain_max),
                    (0.0, 100.0),
                    "explicit domain wins"
                );
                assert_eq!(stops.len(), 2, "two-stop ramp");
            }
            other => panic!("expected Sequential, got {other:?}"),
        }
        // The midpoint interpolates halfway between the endpoints.
        let mid = scale.map_continuous(50.0);
        for c in &mid[..3] {
            assert!((c - 0.5).abs() < 1e-6, "midpoint interpolates, got {mid:?}");
        }
        // A partial override leaves the untouched facets alone: a positional
        // scale is never touched.
        let mut pos = ScaleSet::new();
        pos.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 1.0,
                range_start: 0.0,
                range_end: 10.0,
            },
        );
        apply_colour_override(&mut pos, &ov);
        assert!(
            matches!(
                pos.get(Channel::X),
                Some(Scale::Linear { domain_min, domain_max, range_start, range_end })
                    if *domain_min == 0.0 && *domain_max == 1.0
                        && *range_start == 0.0 && *range_end == 10.0
            ),
            "positional scales untouched"
        );
    }

    // --- adding Sequential leaves every exhaustive match decided ---

    #[test]
    fn sequential_match_arms_decided() {
        let stops = SequentialScheme::Viridis.stops();
        let a = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 10.0,
            stops: stops.clone(),
        };
        let b = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 25.0,
            stops: stops.clone(),
        };

        // union_scales unions by min-of-mins / max-of-maxes.
        let unioned = union_scales(&[a.clone(), b], 0.0, 0.0).expect("union yields a scale");
        match unioned {
            Scale::Sequential {
                domain_min,
                domain_max,
                ..
            } => {
                assert!((domain_min - 0.0).abs() < f64::EPSILON);
                assert!((domain_max - 25.0).abs() < f64::EPSILON);
            }
            other => panic!("expected Sequential, got {other:?}"),
        }

        // compute_ticks returns no positional ticks; domain_max reads the extent.
        assert!(crate::axis::compute_ticks(&a, 5).is_empty());
        assert_eq!(a.domain_min(), Some(0.0));
        assert_eq!(a.domain_max(), Some(10.0));
        // A colour ramp has no positional pixel range and cannot invert.
        assert_eq!(a.range_start(), 0.0);
        assert_eq!(a.range_end(), 0.0);
        assert!(a.inverse_f64(5.0).is_none());
    }

    // --- F1: anchor_scales widen-only matrix ---

    fn linear(min: f64, max: f64) -> Scale {
        Scale::Linear {
            domain_min: min,
            domain_max: max,
            range_start: 0.0,
            range_end: 400.0,
        }
    }

    /// Launch-anchored, widen-only: the pure anchor fold. A SUBSET
    /// fresh domain yields exactly launch (an ordinary filter gesture is
    /// pixel-identical to a hard pin); a SUPERSET fresh domain widens the launch
    /// domain to include it (a query-rewrite gesture keeps new rows on-plot);
    /// a categorical channel keeps launch (no colour/position churn); a channel
    /// present only in fresh (a late-arriving mark) is adopted; only-in-launch
    /// is kept.
    #[test]
    fn anchor_scales_is_widen_only() {
        // Subset fresh → launch exactly (byte-for-byte the launch domain).
        let mut launch = ScaleSet::new();
        launch.insert(Channel::X, linear(0.0, 100.0));
        let mut subset = ScaleSet::new();
        subset.insert(Channel::X, linear(25.0, 75.0));
        let a = anchor_scales(&launch, subset);
        assert_eq!(a.get(Channel::X).unwrap().domain_min(), Some(0.0));
        assert_eq!(a.get(Channel::X).unwrap().domain_max(), Some(100.0));

        // Superset fresh (query rewrite surfaced rows below/above launch) →
        // widened union, so the new rows aren't clipped.
        let mut superset = ScaleSet::new();
        superset.insert(Channel::X, linear(-10.0, 130.0));
        let a = anchor_scales(&launch, superset);
        assert_eq!(a.get(Channel::X).unwrap().domain_min(), Some(-10.0));
        assert_eq!(a.get(Channel::X).unwrap().domain_max(), Some(130.0));

        // One-sided widening keeps the untouched bound at launch.
        let mut below = ScaleSet::new();
        below.insert(Channel::X, linear(-5.0, 40.0));
        let a = anchor_scales(&launch, below);
        assert_eq!(a.get(Channel::X).unwrap().domain_min(), Some(-5.0));
        assert_eq!(
            a.get(Channel::X).unwrap().domain_max(),
            Some(100.0),
            "upper bound stays launch"
        );

        // Sequential widens domain but keeps the LAUNCH ramp stops (colours
        // never re-anchor — the F3 regression class).
        let mut lseq = ScaleSet::new();
        lseq.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 0.0,
                domain_max: 100.0,
                stops: SequentialScheme::Blues.stops(),
            },
        );
        let mut fseq = ScaleSet::new();
        fseq.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 0.0,
                domain_max: 40.0,
                stops: SequentialScheme::Viridis.stops(),
            },
        );
        match anchor_scales(&lseq, fseq).get(Channel::Fill) {
            Some(Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            }) => {
                assert_eq!(
                    (*domain_min, *domain_max),
                    (0.0, 100.0),
                    "subset density → launch domain"
                );
                assert_eq!(
                    *stops,
                    SequentialScheme::Blues.stops(),
                    "launch stops, not fresh's"
                );
            }
            other => panic!("expected Sequential, got {other:?}"),
        }

        // Categorical: launch wins (category order/colours pinned) even when
        // fresh drops a category.
        let mut lcat = ScaleSet::new();
        lcat.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["a".into(), "b".into(), "c".into()],
                palette: CATEGORICAL_PALETTE.to_vec(),
            },
        );
        let mut fcat = ScaleSet::new();
        fcat.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["c".into()],
                palette: CATEGORICAL_PALETTE.to_vec(),
            },
        );
        match anchor_scales(&lcat, fcat).get(Channel::Fill) {
            Some(Scale::Colour { categories, .. }) => {
                assert_eq!(categories.len(), 3, "launch categories pinned");
                assert_eq!(categories[2], "c");
            }
            other => panic!("expected Colour, got {other:?}"),
        }

        // Channel only in fresh (a mark whose batch was empty at launch) is
        // adopted; channel only in launch is kept.
        let mut launch_y = ScaleSet::new();
        launch_y.insert(Channel::Y, linear(0.0, 10.0));
        let mut fresh_fill = ScaleSet::new();
        fresh_fill.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 0.0,
                domain_max: 9.0,
                stops: SequentialScheme::Blues.stops(),
            },
        );
        let a = anchor_scales(&launch_y, fresh_fill);
        assert!(a.get(Channel::Y).is_some(), "only-in-launch kept");
        assert!(
            a.get(Channel::Fill).is_some(),
            "only-in-fresh adopted (F2: late raster ramp)"
        );
    }

    // --- a launch-baked inset survives every anchored rebuild ---

    #[test]
    fn inset_survives_anchored_rebuild() {
        // Launch range carries a nonzero inset (45..615, not the un-inset
        // 40..620). anchor_scale copies the launch range verbatim, so the inset
        // rides through every widen-only fold.
        let inset_launch = |min, max| Scale::Linear {
            domain_min: min,
            domain_max: max,
            range_start: 45.0,
            range_end: 615.0,
        };
        let mut launch = ScaleSet::new();
        launch.insert(Channel::X, inset_launch(0.0, 100.0));

        // A widening gesture (superset domain) folds in; range stays launch.
        let mut fresh = ScaleSet::new();
        fresh.insert(Channel::X, linear(-10.0, 130.0)); // linear() ranges 0..400
        match anchor_scales(&launch, fresh).get(Channel::X) {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            }) => {
                assert_eq!((*domain_min, *domain_max), (-10.0, 130.0), "domain widened");
                assert_eq!(
                    (*range_start, *range_end),
                    (45.0, 615.0),
                    "launch range (inset baked in) preserved verbatim"
                );
            }
            other => panic!("expected Linear, got {other:?}"),
        }

        // A subset (filter) gesture is byte-identical to launch, inset included.
        let mut subset = ScaleSet::new();
        subset.insert(Channel::X, linear(25.0, 75.0));
        if let Some(Scale::Linear {
            range_start,
            range_end,
            ..
        }) = anchor_scales(&launch, subset).get(Channel::X)
        {
            assert_eq!(
                (*range_start, *range_end),
                (45.0, 615.0),
                "subset rebuild is inset-identical"
            );
        } else {
            panic!("expected Linear");
        }
    }

    // --- positional_axis_class datatype peek ---

    #[test]
    fn axi_positional_axis_class_peeks_datatypes() {
        let num = make_numeric_batch();
        let mut ncm = ChannelMap::new();
        ncm.insert(Channel::X, "x".into());
        ncm.insert(Channel::Y, "y".into());
        let np = [(&num, &ncm)];
        assert_eq!(
            positional_axis_class(&np, Channel::X),
            Some(AxisClass::Continuous)
        );
        assert_eq!(
            positional_axis_class(&np, Channel::Y),
            Some(AxisClass::Continuous)
        );

        let cat = make_categorical_batch();
        let mut ccm = ChannelMap::new();
        ccm.insert(Channel::X, "category".into());
        ccm.insert(Channel::Y, "value".into());
        let cp = [(&cat, &ccm)];
        assert_eq!(
            positional_axis_class(&cp, Channel::X),
            Some(AxisClass::Band)
        );
        assert_eq!(
            positional_axis_class(&cp, Channel::Y),
            Some(AxisClass::Continuous)
        );

        // No binding on an axis → None (augment-only / absent — no default).
        let mut only_x = ChannelMap::new();
        only_x.insert(Channel::X, "x".into());
        let op = [(&num, &only_x)];
        assert_eq!(positional_axis_class(&op, Channel::Y), None);

        // Mixed: one mark bands x, another binds x numeric → Continuous wins.
        let mixed = [(&cat, &ccm), (&num, &ncm)];
        assert_eq!(
            positional_axis_class(&mixed, Channel::X),
            Some(AxisClass::Continuous)
        );

        // Time (Timestamp) axis classifies Continuous — the AC's "linear/time".
        let tim = make_time_batch();
        let mut tcm = ChannelMap::new();
        tcm.insert(Channel::X, "ts".into());
        let tp = [(&tim, &tcm)];
        assert_eq!(
            positional_axis_class(&tp, Channel::X),
            Some(AxisClass::Continuous)
        );
    }

    // --- positional domain pinning (`Domain: Fixed`) ---

    /// The pin carries a DOMAIN and nothing else, so applying it onto a scale
    /// built for a different layout keeps that layout's pixel range. A window
    /// resize gives a plot a new range; a pinned plot has to resize with it.
    #[test]
    fn a_pin_re_domains_a_scale_without_touching_its_range() {
        let pin = PinnedDomain::of(&Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 40.0,
            range_end: 600.0,
        })
        .expect("a linear scale offers a pin");

        let mut set = ScaleSet::new();
        set.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 10.0,
                domain_max: 20.0,
                range_start: 40.0,
                range_end: 900.0,
            },
        );
        apply_pinned_domains(
            &mut set,
            &PinnedDomains {
                x: Some(pin),
                y: None,
            },
        );
        match set.get(Channel::X).expect("x scale kept") {
            Scale::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            } => {
                assert_eq!((*domain_min, *domain_max), (0.0, 100.0), "domain pinned");
                assert_eq!(
                    (*range_start, *range_end),
                    (40.0, 900.0),
                    "the range is the current layout's, not the pin's"
                );
            }
            other => panic!("expected a linear scale, got {other:?}"),
        }
    }

    /// A band pin carries the category ORDER, which is what assigns each
    /// category its slot. Pinning it is how a filtered-away category keeps its
    /// place instead of every later one sliding along.
    #[test]
    fn a_band_pin_restores_the_categories_a_filter_removed() {
        let pin = PinnedDomain::of(&Scale::Band {
            categories: vec!["a".into(), "b".into(), "c".into()],
            range_start: 0.0,
            range_end: 300.0,
            padding: 0.1,
        })
        .expect("a band scale offers a pin");

        let mut set = ScaleSet::new();
        set.insert(
            Channel::X,
            Scale::Band {
                categories: vec!["a".into()],
                range_start: 0.0,
                range_end: 300.0,
                padding: 0.2,
            },
        );
        apply_pinned_domains(
            &mut set,
            &PinnedDomains {
                x: Some(pin),
                y: None,
            },
        );
        match set.get(Channel::X).expect("x scale kept") {
            Scale::Band {
                categories,
                padding,
                ..
            } => {
                assert_eq!(categories, &["a", "b", "c"], "every slot is back, in order");
                assert!(
                    (*padding - 0.2).abs() < f64::EPSILON,
                    "padding belongs to the current scale, not the pin"
                );
            }
            other => panic!("expected a band scale, got {other:?}"),
        }
    }

    /// A colour channel offers no positional pin: `colorDomain` is a separate
    /// instruction with its own mechanism ([`ColourOverride`]).
    #[test]
    fn colour_scales_offer_no_positional_pin() {
        assert!(PinnedDomain::of(&Scale::Colour {
            categories: vec!["a".into()],
            palette: CATEGORICAL_PALETTE.to_vec(),
        })
        .is_none());
        assert!(PinnedDomain::of(&Scale::Sequential {
            domain_min: 0.0,
            domain_max: 1.0,
            stops: vec![[0.0; 4], [1.0; 4]],
        })
        .is_none());
    }

    /// A column that came back numeric at launch and categorical after a
    /// gesture has changed what the axis IS. Re-domaining across that would
    /// place rows by a rule neither render used, so the fresh scale stands.
    #[test]
    fn a_pin_of_the_wrong_kind_leaves_the_fresh_scale_alone() {
        let mut set = ScaleSet::new();
        set.insert(
            Channel::X,
            Scale::Band {
                categories: vec!["a".into(), "b".into()],
                range_start: 0.0,
                range_end: 300.0,
                padding: 0.1,
            },
        );
        apply_pinned_domains(
            &mut set,
            &PinnedDomains {
                x: Some(PinnedDomain::Linear(0.0, 100.0)),
                y: None,
            },
        );
        match set.get(Channel::X).expect("x scale kept") {
            Scale::Band { categories, .. } => {
                assert_eq!(categories, &["a", "b"], "the fresh band scale is untouched");
            }
            other => panic!("expected the band scale to survive, got {other:?}"),
        }
    }

    /// **Nothing pinned, nothing written.** The default path through
    /// `build_multi_mark_scene_with_domains` passes an empty set, so this is
    /// what makes a spec asking for no pin take the behaviour it always had.
    #[test]
    fn an_empty_pin_set_writes_nothing() {
        let mut set = ScaleSet::new();
        set.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 3.0,
                domain_max: 7.0,
                range_start: 0.0,
                range_end: 100.0,
            },
        );
        let empty = PinnedDomains::default();
        assert!(empty.is_empty());
        apply_pinned_domains(&mut set, &empty);
        match set.get(Channel::X).expect("x scale kept") {
            Scale::Linear {
                domain_min,
                domain_max,
                ..
            } => assert_eq!((*domain_min, *domain_max), (3.0, 7.0)),
            other => panic!("expected the scale untouched, got {other:?}"),
        }
    }

    /// **The first composition is the one that pins.** `capture` is called on
    /// every composition, so it is here that "first render" is enforced: an
    /// axis already holding a pin keeps it, whatever a later set of scales
    /// says.
    #[test]
    fn capture_takes_the_first_answer_and_keeps_it() {
        let both = FixedDomains { x: true, y: true };
        let scales_at = |lo: f64, hi: f64| {
            let mut s = ScaleSet::new();
            s.insert(
                Channel::X,
                Scale::Linear {
                    domain_min: lo,
                    domain_max: hi,
                    range_start: 0.0,
                    range_end: 100.0,
                },
            );
            s
        };

        let mut pins = PinnedDomains::default();
        pins.capture(&scales_at(0.0, 50.0), both);
        assert_eq!(pins.x, Some(PinnedDomain::Linear(0.0, 50.0)));
        pins.capture(&scales_at(2.0, 9.0), both);
        assert_eq!(
            pins.x,
            Some(PinnedDomain::Linear(0.0, 50.0)),
            "a later composition does not re-pin"
        );
        assert_eq!(pins.y, None, "no y scale existed to pin");
    }

    /// An axis the spec did not ask to pin is never captured, so it can never
    /// be applied.
    #[test]
    fn capture_ignores_an_axis_the_spec_did_not_pin() {
        let mut set = ScaleSet::new();
        set.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 1.0,
                range_start: 0.0,
                range_end: 10.0,
            },
        );
        set.insert(
            Channel::Y,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 1.0,
                range_start: 0.0,
                range_end: 10.0,
            },
        );
        let mut pins = PinnedDomains::default();
        pins.capture(&set, FixedDomains { x: false, y: true });
        assert_eq!(pins.x, None, "x was not requested");
        assert_eq!(pins.y, Some(PinnedDomain::Linear(0.0, 1.0)));
    }

    /// **A colour domain's order is the ordering rule's; a band domain's is the
    /// rows'.** One column, two channels, and the two answers differ — which is
    /// the whole of why `order_categories` is scoped to colour.
    ///
    /// A band scale's category order is where the marks ARE, and the query that
    /// produced these rows may have ordered them on purpose. Sorting it would
    /// answer a determinism problem by discarding an author's `ORDER BY`.
    #[test]
    fn colour_inference_orders_categories_and_band_inference_does_not() {
        let col = StringArray::from(vec!["zulu", "alpha", "zulu", "mike"]);

        let fill = infer_column_scale(&col, 0.0, 0.0, Channel::Fill)
            .expect("a string column gives the fill channel a colour scale");
        match &fill {
            Scale::Colour { categories, .. } => assert_eq!(
                categories,
                &["alpha".to_string(), "mike".to_string(), "zulu".to_string()],
                "a palette slot must be a function of the category set, not of arrival order"
            ),
            other => panic!("expected a colour scale, got {other:?}"),
        }

        let x = infer_column_scale(&col, 0.0, 100.0, Channel::X)
            .expect("a string column gives a positional channel a band scale");
        match &x {
            Scale::Band { categories, .. } => assert_eq!(
                categories,
                &["zulu".to_string(), "alpha".to_string(), "mike".to_string()],
                "the bands must stay in the order the rows arrived in"
            ),
            other => panic!("expected a band scale, got {other:?}"),
        }
    }

    /// The union of two ordered lists is not ordered, so the merge re-applies
    /// the rule rather than inheriting it from the parts.
    #[test]
    fn unioning_two_colour_scales_re_orders_the_merged_set() {
        let left = infer_column_scale(
            &StringArray::from(vec!["alpha", "zulu"]),
            0.0,
            0.0,
            Channel::Fill,
        )
        .expect("colour scale");
        let right = infer_column_scale(&StringArray::from(vec!["mike"]), 0.0, 0.0, Channel::Fill)
            .expect("colour scale");

        match union_scales(&[left, right], 0.0, 0.0).expect("a union of colour scales") {
            Scale::Colour { categories, .. } => assert_eq!(
                categories,
                vec!["alpha".to_string(), "mike".to_string(), "zulu".to_string()],
                "appending the second list to the first would leave `mike` after `zulu`"
            ),
            other => panic!("expected a colour scale, got {other:?}"),
        }
    }
}
