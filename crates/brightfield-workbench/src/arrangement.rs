//! The window's arrangement: which regions it lays out, what fills each one,
//! and how much of its own axis each one takes.
//!
//! # Why this exists
//!
//! The extents a window lays itself out with were literals in the shell's draw
//! path, and the same numbers were literals a second time in the arithmetic
//! that sizes the window before any frame exists. Two spellings of one measure
//! is the drift [`crate::registry`] was written to end for panes; this ends it
//! for the regions those panes sit in. The rule is the one
//! `protocol_registry()` already follows in the shell: one declaration, read by
//! whatever needs it, with no second copy held elsewhere.
//!
//! `the_drawn_regions_match_the_declared_arrangement` in the shell's
//! `tests/arrangement.rs` is what makes that true rather than aspirational: it
//! lays a real frame out, reads back the rect each region was drawn at, and
//! compares it with the extent declared here. A draw path holding its own
//! number reddens it.
//!
//! # The shape of an arrangement
//!
//! A [`Region`] is a name, an [`Edge`], an [`Extent`] and an [`Occupant`]. An
//! [`Arrangement`] is an ordered list of them: the order is the order the
//! shell adds the panels in, which is what decides whether a rail spans past a
//! band or stops short of it.
//!
//! Three extents, and the difference between them is a decision rather than a
//! detail:
//!
//! - [`Extent::Band`] is fixed and not resizable. A band carries one row of
//!   chrome; letting the user drag it would be offering a choice with no
//!   content behind it.
//! - [`Extent::Rail`] carries a default and a floor, and is resizable between
//!   them. The floor is the width past which the rail's content stops being
//!   readable rather than merely tight.
//! - [`Extent::Remainder`] is the canvas: whatever the bands and rails leave.
//!
//! [`Extent::Overlay`] is the fourth, and it floats rather than taking room
//! from a sibling — see its own docs, and
//! [`crate::chrome::status_rail_overlay`], which draws the status band that
//! way. `the_overlay_region_takes_no_space_and_the_rest_do` holds the split.
//!
//! # Collapsing is a second question, so it is a second field
//!
//! [`Collapse`] says whether the user may take a region's axis back, and how
//! much of that axis stays behind when they do. It sits beside [`Extent`] on
//! [`Region`] rather than becoming a fifth case of it, and the two questions
//! really are independent: a rail that collapses still opens at its default,
//! still refuses to narrow past its floor, and is still resizable, so nothing
//! [`Extent`] already answers changes when a region gains a collapsed state.
//!
//! A fifth variant would change the answer for the four readers that match on
//! [`Extent`] today — `band_extent`, `rail_default` and `rail_min` in the
//! shell's draw path, and [`audit_arrangement`] here. The first three panic on
//! an extent they do not recognise, so a collapsible rail spelled as its own
//! variant would stop being a rail to the draw path, to the arithmetic that
//! sizes the window and to the audit at the same moment, and each of them
//! would need a second spelling of "rail". Two spellings of one measure is the
//! drift this module exists to end.
//!
//! What stays behind is declared per region rather than taken from one
//! constant, because the axis it is measured on differs. The ledger is a
//! bottom rail, so [`chrome::rail_selector_height`] of *height* leaves its
//! selector strip whole, with its pane names still drawn and still live. The
//! same number of *width* on a side rail leaves a stub with room for the
//! control that reopens it and none for the names.
//! `the_collapsed_ledger_keeps_its_selector_strip_and_reopens_from_it` and
//! `the_navigator_rail_collapses_from_its_strip_and_reopens` are the two ends
//! of that, drawn.
//!
//! # More than one arrangement, at the cost of a declaration
//!
//! [`spine_left`] is the arrangement the window ships with. A second one is a
//! second `static` here and a second name, and deliberately not a second draw
//! path: the shell's draw path reads regions by id, so an arrangement that
//! declares the same ids in different places is drawn by the code that is
//! already there.
//!
//! It is not, on its own, a second arrangement on screen. Both of the shell's
//! readers — its draw path and the arithmetic that sizes the window before a
//! frame exists — call [`default_arrangement`], which takes no argument and
//! answers [`spine_left`]. Reaching a second one is that function growing a
//! parameter and those two call sites being given something to pass it; until
//! then a `static` added here is declared and unread.

use crate::chrome;
use crate::item::ItemId;

// ---------------------------------------------------------------------------
// The band measures
// ---------------------------------------------------------------------------

/// The title band's height in logical points: the app's identity, the way back
/// to the front door, and the subject.
///
/// The window-bar rung — a grid row with the band's own padding above and
/// below — which is the same arithmetic
/// [`chrome::status_rail_height`] uses for the rail's band. Declared rather
/// than measured for the reason the module docs give: the window is sized
/// before a frame exists, so a content-sized band has no height to read.
pub const TITLE_BAND_HEIGHT: f32 =
    meridian_design::spacing::ROW_GRID + 2.0 * meridian_design::spacing::SPACE_2;

/// The locator band's height in logical points: the breadcrumb, and what the
/// document on disk is doing.
///
/// The dense rung with the same padding, not the grid rung: this band carries
/// a line of text and no pointer target, so the rung that exists to clear the
/// pointer-target floor buys nothing here. The frame this arrangement was
/// drawn from asked for 30. The design system's ladder does not carry 30;
/// 28 is the nearest rung below it, and picking a rung is the rule the ladder
/// exists to impose.
pub const LOCATOR_BAND_HEIGHT: f32 =
    meridian_design::spacing::ROW_DENSE + 2.0 * meridian_design::spacing::SPACE_2;

/// The hint band's height in logical points: the key grammar of the surface
/// that has one.
///
/// The same rung as [`TITLE_BAND_HEIGHT`], and deliberately not the shorter
/// band the frame drew. `Panel::exact_size` clamps the rect a band *reports*
/// while its content goes on occupying what it wanted, so a band pinned
/// shorter than its content hands its sibling less than the window arithmetic
/// subtracted. The hint row was measured at 29 logical points when the shell's
/// `BAR_HEIGHT` was written, which is above the 26 the frame asked for and
/// below this.
pub const HINT_BAND_HEIGHT: f32 = TITLE_BAND_HEIGHT;

// ---------------------------------------------------------------------------
// The rail measures
// ---------------------------------------------------------------------------

/// The navigator rail's default width, outer, in logical points.
pub const NAVIGATOR_RAIL_WIDTH: f32 = 240.0;
/// The navigator rail's floor: the width past which `Panel::resizable` refuses
/// to narrow it.
///
/// The spine draws a two-line operator row — a kicker over a name — beside a
/// run-state dot and its word. Below this the name and the state word collide.
pub const NAVIGATOR_RAIL_MIN_WIDTH: f32 = 160.0;

/// The inspector rail's default width, outer, in logical points.
pub const INSPECTOR_RAIL_WIDTH: f32 = 280.0;
/// The inspector rail's floor, outer, in logical points.
pub const INSPECTOR_RAIL_MIN_WIDTH: f32 = 200.0;

/// The ledger rail's default height, outer, in logical points.
pub const LEDGER_RAIL_HEIGHT: f32 = 180.0;
/// The ledger rail's floor, outer, in logical points.
pub const LEDGER_RAIL_MIN_HEIGHT: f32 = 120.0;

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// A region's name — the id the shell records its drawn rect under, and the id
/// a test asks for that rect by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(&'static str);

impl RegionId {
    /// A region id.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The id's string form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for RegionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// The title band — app identity, the way home, and the subject.
pub const TITLE_BAND: RegionId = RegionId::new("title-band");
/// The locator band — the breadcrumb.
pub const LOCATOR_BAND: RegionId = RegionId::new("locator-band");
/// The hint band — the key grammar, on a surface that has one.
pub const HINT_BAND: RegionId = RegionId::new("hint-band");
/// The status band — what the window is doing.
pub const STATUS_BAND: RegionId = RegionId::new("status-band");
/// The navigator rail — the protocol spine.
pub const NAVIGATOR_RAIL: RegionId = RegionId::new("navigator-rail");
/// The inspector rail — what the selection is.
pub const INSPECTOR_RAIL: RegionId = RegionId::new("inspector-rail");
/// The ledger rail — the record under the work.
pub const LEDGER_RAIL: RegionId = RegionId::new("ledger-rail");
/// The canvas — the remainder.
pub const CANVAS: RegionId = RegionId::new("canvas");

/// Which edge of the window a region takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Above everything below it.
    Top,
    /// Below everything above it.
    Bottom,
    /// The left-hand side.
    Left,
    /// The right-hand side.
    Right,
    /// No edge: the region is what is left.
    Centre,
}

/// How much of its own axis a region takes.
///
/// The axis is the one its [`Edge`] runs across: height for a top or bottom
/// region, width for a left or right one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Extent {
    /// Exactly this many logical points, not resizable.
    Band(f32),
    /// This many by default, resizable, and refusing to go below `min`.
    Rail {
        /// What it opens at.
        default: f32,
        /// What it refuses to narrow past.
        min: f32,
    },
    /// Whatever the bands and rails leave.
    Remainder,
    /// A floating layer over the window's edge: this tall when it has
    /// something to say, and absent — taking no space from any sibling —
    /// when it does not.
    ///
    /// Declared rather than omitted because a region that sometimes draws is
    /// still a region of the arrangement, and a reader asking "what is at the
    /// bottom edge" should find the answer here rather than in the one file
    /// that happens to paint it.
    Overlay(f32),
}

impl Extent {
    /// What this region opens at, in logical points, or `None` for
    /// [`Extent::Remainder`], which has no extent of its own.
    #[must_use]
    pub const fn default_extent(self) -> Option<f32> {
        match self {
            Extent::Band(size) | Extent::Overlay(size) | Extent::Rail { default: size, .. } => {
                Some(size)
            }
            Extent::Remainder => None,
        }
    }

    /// The extent this region refuses to go below, or `None` where the
    /// question does not arise.
    #[must_use]
    pub const fn min_extent(self) -> Option<f32> {
        match self {
            Extent::Band(size) | Extent::Overlay(size) => Some(size),
            Extent::Rail { min, .. } => Some(min),
            Extent::Remainder => None,
        }
    }

    /// Whether the user may drag this region's edge.
    #[must_use]
    pub const fn is_resizable(self) -> bool {
        matches!(self, Extent::Rail { .. })
    }

    /// Whether this region takes space from its siblings.
    ///
    /// `false` for [`Extent::Overlay`], which floats.
    #[must_use]
    pub const fn takes_space(self) -> bool {
        !matches!(self, Extent::Overlay(_))
    }
}

/// Whether the user may take a region's axis back, and what stays on screen
/// when they do.
///
/// A different question from [`Extent`], which is why it is a different field
/// on [`Region`] — the module docs argue that at length, and
/// `the_shipped_arrangement_lets_a_rail_collapse_and_a_band_not` is where the
/// shipped answer is pinned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collapse {
    /// The region is always at its [`Extent`]. Nothing draws a control that
    /// would take its axis away.
    Fixed,
    /// The user may collapse the region to this many logical points of its own
    /// axis: what the window goes on drawing there, and what the arithmetic
    /// that sizes the window subtracts while it is collapsed.
    To(f32),
}

impl Collapse {
    /// Whether the user may collapse this region.
    #[must_use]
    pub const fn is_collapsible(self) -> bool {
        matches!(self, Self::To(_))
    }

    /// The extent the region takes while it is collapsed, or `None` for a
    /// region that does not collapse.
    #[must_use]
    pub const fn extent(self) -> Option<f32> {
        match self {
            Self::To(size) => Some(size),
            Self::Fixed => None,
        }
    }
}

/// What fills a region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Occupant {
    /// The window's own chrome, said in words rather than named by an item:
    /// the bands carry no pane, and inventing an [`ItemId`] for a row of text
    /// would make the item vocabulary claim something it does not hold.
    Chrome(&'static str),
    /// Panes, in the order the region's own selector strip offers them.
    Panes(&'static [ItemId]),
    /// The canvas, whose occupancy is two questions rather than one: which
    /// projection of a step is showing, and what stands there when the subject
    /// is the protocol rather than one of its steps.
    Canvas {
        /// The step's projections, behind the canvas's one toggle. Two, and
        /// the toggle is built from this slice, so a third here would be a
        /// third on screen — which is what
        /// `the_canvas_toggle_offers_two_projections_and_no_more` refuses.
        projections: &'static [Projection],
        /// The asset graph, which stands at canvas size while there is no
        /// chart for the canvas to hold — the canvas's empty state rather
        /// than a second thing it can be showing. A window holding both
        /// documents draws the projection: the protocol is readable, without
        /// a click, in two of its three regions — the inspector rail opens on
        /// the chart's own controls there — and the chart has only this one.
        graph: ItemId,
    },
}

/// One reading of a step's output, and the word the canvas toggle offers it
/// under.
///
/// The word is declared here rather than taken from the pane's own
/// [`Subject`](crate::Subject) title, and that is a departure from the rule
/// the rest of the chrome follows. The reason is that the two are different
/// claims: a pane's title names the document it is showing — the chart pane's
/// is the dashboard's own name, which changes with the file — while these two
/// words name the *reading*, and stay put while the subject under them
/// changes. `the_canvas_toggle_offers_two_projections_and_no_more` counts
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Projection {
    /// The pane that draws this reading.
    pub item: ItemId,
    /// The word the toggle offers it under.
    pub label: &'static str,
}

/// One region of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    /// Its name.
    pub id: RegionId,
    /// Which edge it takes.
    pub edge: Edge,
    /// How much of its axis it takes.
    pub extent: Extent,
    /// What fills it.
    pub occupant: Occupant,
    /// Whether the user may take its axis back, and what stays when they do.
    pub collapse: Collapse,
}

impl Region {
    /// What this region opens at — [`Extent::default_extent`].
    #[must_use]
    pub const fn default_extent(&self) -> Option<f32> {
        self.extent.default_extent()
    }

    /// What this region refuses to go below — [`Extent::min_extent`].
    #[must_use]
    pub const fn min_extent(&self) -> Option<f32> {
        self.extent.min_extent()
    }

    /// What this region takes while collapsed — [`Collapse::extent`].
    #[must_use]
    pub const fn collapsed_extent(&self) -> Option<f32> {
        self.collapse.extent()
    }
}

/// One window's arrangement: its regions, in the order they are added.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrangement {
    /// What this arrangement is called, for a reader and for a report.
    pub name: &'static str,
    /// Its regions, in the order the shell adds the panels.
    ///
    /// Order is content, not presentation: a bottom band added before a left
    /// rail spans the full width and the rail stops above it, and the two
    /// added the other way round give the opposite window.
    pub regions: &'static [Region],
}

impl Arrangement {
    /// One region, by name.
    #[must_use]
    pub fn region(&self, id: RegionId) -> Option<&'static Region> {
        self.regions.iter().find(|r| r.id == id)
    }

    /// One region, by name, or a panic naming the arrangement that lacks it.
    ///
    /// The shell's draw path calls this: a region the arrangement does not
    /// declare is a structural mistake, and a `None` swallowed there would
    /// surface as a band that silently stopped being drawn.
    ///
    /// # Panics
    ///
    /// If this arrangement declares no region called `id`.
    #[must_use]
    pub fn expect_region(&self, id: RegionId) -> &'static Region {
        self.region(id)
            .unwrap_or_else(|| panic!("{}: no region called {id}", self.name))
    }
}

// ---------------------------------------------------------------------------
// The arrangements
// ---------------------------------------------------------------------------

/// The navigator rail's panes.
static NAVIGATOR_PANES: &[ItemId] = &[ItemId::new("protocol-outline")];

/// The inspector rail's panes — the protocol's operator inspector and the
/// chart's, behind the rail's own selector.
static INSPECTOR_PANES: &[ItemId] = &[
    ItemId::new("protocol-inspector"),
    ItemId::new("chart-controls"),
];

/// The ledger rail's panes — the step list and the document that declares it.
static LEDGER_PANES: &[ItemId] = &[ItemId::new("protocol-steps"), ItemId::new("spec-editor")];

/// The canvas's two projections, in the order the toggle offers them: the
/// tabular reading of a step's output, then the drawn one.
static PROJECTIONS: &[Projection] = &[
    Projection {
        item: ItemId::new("chart-data-grid"),
        label: "Grid",
    },
    Projection {
        item: ItemId::new("chart-canvas"),
        label: "Chart",
    },
];

/// Arrangement A — spine left.
///
/// The protocol is the navigator rail, as an ordered spine of operators with
/// their run state; the canvas belongs to the step's projection at full size;
/// the inspector is the right rail; the ledger is the bottom rail.
///
/// All three rails open, and each declares a [`Collapse`] the user can take
/// them down to — the ledger onto its own selector strip, the two side rails
/// onto a stub of the same measure. The bands and the canvas do not collapse:
/// `the_shipped_arrangement_lets_a_rail_collapse_and_a_band_not` counts which
/// is which off this list.
static SPINE_LEFT: Arrangement = Arrangement {
    name: "spine left",
    regions: &[
        Region {
            id: TITLE_BAND,
            edge: Edge::Top,
            extent: Extent::Band(TITLE_BAND_HEIGHT),
            occupant: Occupant::Chrome("app identity, the way home, and the subject"),
            collapse: Collapse::Fixed,
        },
        Region {
            id: LOCATOR_BAND,
            edge: Edge::Top,
            extent: Extent::Band(LOCATOR_BAND_HEIGHT),
            occupant: Occupant::Chrome("the breadcrumb"),
            collapse: Collapse::Fixed,
        },
        Region {
            id: STATUS_BAND,
            edge: Edge::Bottom,
            extent: Extent::Overlay(chrome::status_rail_height()),
            occupant: Occupant::Chrome("what the window is doing"),
            collapse: Collapse::Fixed,
        },
        Region {
            id: HINT_BAND,
            edge: Edge::Bottom,
            extent: Extent::Band(HINT_BAND_HEIGHT),
            occupant: Occupant::Chrome("the key grammar of the surface that has one"),
            collapse: Collapse::Fixed,
        },
        Region {
            id: LEDGER_RAIL,
            edge: Edge::Bottom,
            extent: Extent::Rail {
                default: LEDGER_RAIL_HEIGHT,
                min: LEDGER_RAIL_MIN_HEIGHT,
            },
            occupant: Occupant::Panes(LEDGER_PANES),
            collapse: Collapse::To(chrome::rail_selector_height()),
        },
        Region {
            id: NAVIGATOR_RAIL,
            edge: Edge::Left,
            extent: Extent::Rail {
                default: NAVIGATOR_RAIL_WIDTH,
                min: NAVIGATOR_RAIL_MIN_WIDTH,
            },
            occupant: Occupant::Panes(NAVIGATOR_PANES),
            collapse: Collapse::To(chrome::rail_selector_height()),
        },
        Region {
            id: INSPECTOR_RAIL,
            edge: Edge::Right,
            extent: Extent::Rail {
                default: INSPECTOR_RAIL_WIDTH,
                min: INSPECTOR_RAIL_MIN_WIDTH,
            },
            occupant: Occupant::Panes(INSPECTOR_PANES),
            collapse: Collapse::To(chrome::rail_selector_height()),
        },
        Region {
            id: CANVAS,
            edge: Edge::Centre,
            extent: Extent::Remainder,
            occupant: Occupant::Canvas {
                projections: PROJECTIONS,
                graph: ItemId::new("protocol-canvas"),
            },
            collapse: Collapse::Fixed,
        },
    ],
};

/// Arrangement A — spine left.
#[must_use]
pub const fn spine_left() -> &'static Arrangement {
    &SPINE_LEFT
}

/// The arrangement the window opens in.
#[must_use]
pub const fn default_arrangement() -> &'static Arrangement {
    spine_left()
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// Check an arrangement against the contract, and against the item vocabulary
/// the process has published.
///
/// Called from a contract test after the shell's registries have published
/// their ids, for the reason [`crate::audit`] is: the answers this checks are
/// answers no type can hold. Each finding is one line.
///
/// What it refuses:
///
/// - a region id declared twice, which would make [`Arrangement::region`]
///   answer with whichever came first;
/// - an occupant naming an [`ItemId`] the process does not know, which is how
///   a renamed pane would leave a region with an empty body;
/// - a rail whose floor is above what it opens at, which `Panel::min_size`
///   would silently resolve by ignoring the default;
/// - a non-positive band or rail extent;
/// - a collapsed extent that is not below what its region opens at, which
///   would leave the collapse gesture drawn and giving no room back;
/// - a collapsed extent on a region that is not a rail, since the control that
///   collapses one is drawn in a rail's own strip and nothing would read the
///   declaration;
/// - a count of [`Extent::Remainder`] regions other than one.
///
/// # Errors
///
/// The findings, one per line, or `Ok(())` when there are none.
pub fn audit_arrangement(arrangement: &Arrangement) -> Result<(), String> {
    let mut findings: Vec<String> = Vec::new();
    let known = ItemId::known();
    let mut seen: Vec<RegionId> = Vec::new();
    let mut remainders = 0_usize;

    for region in arrangement.regions {
        let id = region.id;
        if seen.contains(&id) {
            findings.push(format!("{id}: declared twice"));
        }
        seen.push(id);

        match region.extent {
            Extent::Band(size) | Extent::Overlay(size) => {
                if !size.is_finite() || size <= 0.0 {
                    findings.push(format!("{id}: band extent {size} is not positive"));
                }
            }
            Extent::Rail { default, min } => {
                if !min.is_finite() || min <= 0.0 {
                    findings.push(format!("{id}: rail floor {min} is not positive"));
                }
                if min > default {
                    findings.push(format!(
                        "{id}: rail floor {min} is above its default {default}"
                    ));
                }
            }
            Extent::Remainder => remainders += 1,
        }

        if let Some(collapsed) = region.collapse.extent() {
            if !collapsed.is_finite() || collapsed <= 0.0 {
                findings.push(format!(
                    "{id}: collapsed extent {collapsed} is not positive"
                ));
            }
            if let Extent::Rail { default, .. } = region.extent {
                if collapsed >= default {
                    findings.push(format!(
                        "{id}: collapsed extent {collapsed} is not below the {default} it opens \
                         at, so collapsing it gives no room back"
                    ));
                }
            } else {
                findings.push(format!(
                    "{id}: declares a collapsed extent and is not a rail; the control that \
                     collapses a region is drawn in a rail's own strip"
                ));
            }
        }

        let occupants: Vec<ItemId> = match region.occupant {
            Occupant::Chrome(_) => Vec::new(),
            Occupant::Panes(panes) => panes.to_vec(),
            Occupant::Canvas { projections, graph } => {
                let mut ids: Vec<ItemId> = projections.iter().map(|p| p.item).collect();
                ids.push(graph);
                ids
            }
        };
        for item in occupants {
            if !known.contains(&item) {
                findings.push(format!(
                    "{id}: occupant `{item}` is not in this process's item vocabulary"
                ));
            }
        }
    }

    if remainders != 1 {
        findings.push(format!(
            "{}: {remainders} regions take the remainder; a window has one canvas",
            arrangement.name
        ));
    }

    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_arrangement_declares_one_canvas_and_no_duplicate_region() {
        // The half of `audit_arrangement` that needs no published vocabulary.
        // The occupant half is checked in the shell, where the ids exist —
        // `the_shipped_arrangement_names_only_panes_this_build_has`.
        let arrangement = default_arrangement();
        let mut ids: Vec<RegionId> = arrangement.regions.iter().map(|r| r.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "a region id is declared twice");
        let remainders = arrangement
            .regions
            .iter()
            .filter(|r| r.extent == Extent::Remainder)
            .count();
        assert_eq!(remainders, 1, "a window has one canvas");
    }

    #[test]
    fn each_rail_opens_above_its_own_floor() {
        for region in default_arrangement().regions {
            if let Extent::Rail { default, min } = region.extent {
                assert!(
                    min <= default,
                    "{}: floor {min} is above its default {default}",
                    region.id
                );
                assert!(min > 0.0, "{}: floor {min} is not positive", region.id);
            }
        }
    }

    #[test]
    fn the_canvas_toggles_between_two_projections() {
        // AC2's declaration half: the toggle is built from this slice, so a
        // third entry here would be a third entry on screen.
        let canvas = default_arrangement().expect_region(CANVAS);
        let Occupant::Canvas { projections, .. } = canvas.occupant else {
            panic!("the canvas region is occupied by the canvas");
        };
        assert_eq!(
            projections.len(),
            2,
            "the canvas toggle offers a grid and a chart: {projections:?}"
        );
    }

    /// One region, spelled out, for the audit's own negative cases below.
    const fn region(id: RegionId, edge: Edge, extent: Extent, collapse: Collapse) -> Region {
        Region {
            id,
            edge,
            extent,
            // An empty pane list rather than `Occupant::Chrome`: the audit
            // checks occupants against the item vocabulary, which a workbench
            // unit test has no registry to publish into, and `Chrome` would
            // itself trip the not-a-rail finding one of these tests is asking
            // about.
            occupant: Occupant::Panes(&[]),
            collapse,
        }
    }

    /// The canvas every case below needs, so `audit_arrangement` is answering
    /// the question the case asks rather than the missing-remainder one.
    static ONE_CANVAS: Region = region(CANVAS, Edge::Centre, Extent::Remainder, Collapse::Fixed);

    #[test]
    fn the_shipped_arrangement_lets_a_rail_collapse_and_a_band_not() {
        // Collapsibility follows resizability across this list rather than
        // being enumerated by name here: a rail the user can drag is a rail
        // the user can put away, and a band carries one row of chrome with
        // nothing to give back.
        for region in default_arrangement().regions {
            assert_eq!(
                region.collapse.is_collapsible(),
                region.extent.is_resizable(),
                "{}: declared {:?} against extent {:?}",
                region.id,
                region.collapse,
                region.extent
            );
        }
    }

    #[test]
    fn the_audit_refuses_a_collapsed_extent_that_gives_no_room_back() {
        static REGIONS: &[Region] = &[
            region(
                LEDGER_RAIL,
                Edge::Bottom,
                Extent::Rail {
                    default: LEDGER_RAIL_HEIGHT,
                    min: LEDGER_RAIL_MIN_HEIGHT,
                },
                Collapse::To(LEDGER_RAIL_HEIGHT),
            ),
            ONE_CANVAS,
        ];
        static ARRANGEMENT: Arrangement = Arrangement {
            name: "collapses to what it already is",
            regions: REGIONS,
        };
        let findings = audit_arrangement(&ARRANGEMENT)
            .expect_err("a rail that collapses to its own default gives no room back");
        assert!(
            findings.contains("gives no room back"),
            "the audit said: {findings}"
        );
    }

    #[test]
    fn the_audit_refuses_a_collapsed_extent_on_something_that_is_not_a_rail() {
        static REGIONS: &[Region] = &[
            region(
                TITLE_BAND,
                Edge::Top,
                Extent::Band(TITLE_BAND_HEIGHT),
                Collapse::To(chrome::rail_selector_height()),
            ),
            ONE_CANVAS,
        ];
        static ARRANGEMENT: Arrangement = Arrangement {
            name: "a band that thinks it collapses",
            regions: REGIONS,
        };
        let findings = audit_arrangement(&ARRANGEMENT)
            .expect_err("a band declaring a collapsed extent declares something nothing reads");
        assert!(
            findings.contains("is not a rail"),
            "the audit said: {findings}"
        );
    }

    #[test]
    fn the_overlay_region_takes_no_space_and_the_rest_do() {
        let arrangement = default_arrangement();
        let status = arrangement.expect_region(STATUS_BAND);
        assert!(!status.extent.takes_space());
        for region in arrangement.regions {
            if region.id != STATUS_BAND {
                assert!(
                    region.extent.takes_space(),
                    "{} floats, which was not declared",
                    region.id
                );
            }
        }
    }
}
