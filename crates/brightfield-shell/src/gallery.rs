//! The in-app design gallery: the one surface where the whole component
//! vocabulary is visible at once, inside the real window.
//!
//! # Why a pane and not a separate binary
//!
//! A gallery in its own binary drifts: it applies the theme the day it is
//! written and stops tracking the app the day after, so what it shows is a
//! memory of the product rather than the product. This gallery is an
//! [`Item`] on the workbench contract, drawn by the same shell, under the
//! same `theme::apply`, on the same frame loop as every working pane — it
//! *cannot* disagree with the app about what a primitive looks like, because
//! it is the app.
//!
//! # The contract
//!
//! - [`Component`] is a **two-method trait**: [`Component::info`] declares
//!   identity, status and the conformance contract; [`Component::ui`] draws
//!   one live specimen. Nothing else — a wider trait is where per-component
//!   snowflakes would creep back in.
//! - [`ComponentStatus`] makes unfinished coverage *visible*: a primitive the
//!   gallery cannot yet gate is listed as [`ComponentStatus::Draft`] with its
//!   status pill on show, never silently absent. (The same honesty device the
//!   conformance corpus uses for pending layers.)
//! - Registration is the explicit [`catalog`] function plus a source-grep
//!   completeness test in `tests/gallery_gate.rs` — **not** link-time
//!   inventory collection, which a linker may strip, and which the grep gate
//!   would still be needed to keep honest anyway.
//!
//! # The dev flag
//!
//! The gallery tab joins the chart view's registry only when
//! [`GALLERY_VAR`] is set (any value but `0`). The pane's id is *always*
//! published ([`crate::app::publish_item_ids`]), so a layout saved with the
//! flag on still loads with it off — the tile falls back to the orphan-pane
//! treatment rather than poisoning the file. A layout saved *before* the flag
//! existed simply has no gallery tile; relocate `BRIGHTFIELD_CONFIG_DIR` (or
//! remove the layout file) to see the default arrangement with the tab.
//!
//! # The per-primitive gate (held in `tests/gallery_gate.rs`)
//!
//! Every gated ([`ComponentStatus::gated`]) component must:
//! 1. resolve by **role and label** — an unlabelled primitive is untestable
//!    and screen-reader-invisible at once;
//! 2. **actuate from the keyboard** with no pointer event, when it is
//!    interactive, proven by an evidence label that appears only after;
//! 3. measure a height that equals a **named ladder value** — a
//!    `meridian_design::spacing` row rung, or a `meridian_design::control`
//!    height, each of which is bound to a row rung by construction
//!    (`control::binding`: control + 2·pad = row) — or carry written prose
//!    for why its height is intrinsic;
//! 4. stay **token-clean** — this file lives in `src/`, so the crate's
//!    token-discipline grep (`tests/token_discipline.rs`) covers every line
//!    of it with no opt-in;
//! 5. carry **two goldens**, light and dark, at the `kittest.toml` floor.

use brightfield_keys::{Altitude, BindingContext, RecencyCounter};
use brightfield_workbench::registry::Slot;
use brightfield_workbench::{
    chrome, EmptyState, HideAffordance, Icon as ItemIcon, Item, ItemCtx, ItemId, ItemSpec,
    StatusEntry, StatusSide, Subject, ToolbarEntry, Tone, Verb,
};
use meridian_design::semantic::{semantic, Role};
use meridian_egui::{
    align, icons, key_chip, list_row, query_line, tooltip_for_action, widgets, ListRow, MeridianUi,
    ModalChrome, ModalLayer, Notification, NotificationId, NotificationLayer, Picker, RowHeight,
    Severity, Toast, ToastLayer, PROMPT_GLYPH,
};

use crate::app::ChartDoc;
use crate::design::{to_color32, Mode};
use crate::overlays::CommandPalette;

// ---------------------------------------------------------------------------
// The dev flag
// ---------------------------------------------------------------------------

/// The environment variable that adds the gallery tab to the chart view.
pub const GALLERY_VAR: &str = "BRIGHTFIELD_GALLERY";

/// Whether this process wants the gallery pane. Set [`GALLERY_VAR`] to
/// anything but `0`.
#[must_use]
pub fn enabled() -> bool {
    std::env::var_os(GALLERY_VAR).is_some_and(|v| v != "0")
}

// ---------------------------------------------------------------------------
// Status — the honesty device
// ---------------------------------------------------------------------------

/// Where a catalog entry stands. The point of the vocabulary is that
/// *unfinished is visible*: a primitive whose gallery coverage is not done is
/// listed as [`ComponentStatus::Draft`] with a pill saying so, instead of
/// being silently absent from the one surface meant to show everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentStatus {
    /// Listed and rendered, but the five-item gate does not hold it yet —
    /// typically a context-layer composite whose inline specimen is pending.
    Draft,
    /// Built and gated, not yet drawn by a shipping product surface.
    Ready,
    /// Gated and on at least one shipping surface.
    Live,
    /// Kept visible while call sites migrate off it.
    Deprecated,
}

impl ComponentStatus {
    /// Whether the five-item gate holds this entry. Draft is the honest
    /// "not yet"; Deprecated stays gated — a primitive still on screen can
    /// still regress.
    #[must_use]
    pub fn gated(self) -> bool {
        matches!(self, ComponentStatus::Ready | ComponentStatus::Live)
            || self == ComponentStatus::Deprecated
    }

    /// The pill's wording.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ComponentStatus::Draft => "draft",
            ComponentStatus::Ready => "ready",
            ComponentStatus::Live => "live",
            ComponentStatus::Deprecated => "deprecated",
        }
    }

    /// The pill's role — colour never travels alone, the icon rides along
    /// wherever the pane draws the pill.
    #[must_use]
    pub fn role(self) -> Role {
        match self {
            ComponentStatus::Draft => Role::Info,
            ComponentStatus::Ready => Role::Accent,
            ComponentStatus::Live => Role::Success,
            ComponentStatus::Deprecated => Role::Warning,
        }
    }

    /// The pill's icon.
    #[must_use]
    pub fn icon(self) -> &'static icons::Icon {
        match self {
            ComponentStatus::Draft => &icons::CLOCK,
            ComponentStatus::Ready => &icons::CHECK,
            ComponentStatus::Live => &icons::CIRCLE_CHECK,
            ComponentStatus::Deprecated => &icons::ALERT_TRIANGLE,
        }
    }
}

/// Draw one status pill for `status` — the gallery's own use of the generic
/// [`widgets::status_pill`], supplying the vocabulary the widget deliberately
/// does not own.
fn status_pill_ui(ui: &mut egui::Ui, status: ComponentStatus) {
    widgets::status_pill(ui, status.icon(), status.label(), status.role());
}

// ---------------------------------------------------------------------------
// The gate contract, as data
// ---------------------------------------------------------------------------

/// The accessibility role the gate queries a specimen by. A tiny local enum
/// rather than `accesskit::Role` so the shell's non-test build does not
/// depend on the accessibility crate's feature surface; the gate test maps
/// each variant onto the real role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeRole {
    /// A click target.
    Button,
    /// Static text.
    Label,
    /// A single-line text input.
    TextInput,
}

/// How the gate finds the specimen: role plus label. This is the first gate
/// item made data — a component with no probe cannot be in the catalog at
/// all, so "unlabelled" is unrepresentable rather than merely caught.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    /// The role the specimen node carries.
    pub role: ProbeRole,
    /// The label it carries.
    pub label: &'static str,
}

/// Which node the gate focuses before sending keyboard input.
#[derive(Clone, Copy, Debug)]
pub enum FocusTarget {
    /// The probe node itself.
    Probe,
    /// The unique node of this role in the solo frame (a text input has its
    /// own node beside the chrome the probe names).
    Role(ProbeRole),
}

/// The keyboard input the gate sends.
#[derive(Clone, Copy, Debug)]
pub enum ActuationInput {
    /// Press and release one key.
    Key(egui::Key),
    /// Type text (the input-method channel, still no pointer).
    Text(&'static str),
}

/// The keyboard-actuation half of the gate: focus `focus`, send `input`, and
/// require the `evidence` label to appear — a label the specimen renders
/// *only after* actuating, so the assertion cannot pass vacuously.
#[derive(Clone, Copy, Debug)]
pub struct Actuation {
    /// Which node takes focus first.
    pub focus: FocusTarget,
    /// What is sent.
    pub input: ActuationInput,
    /// The label that must appear afterwards and not before.
    pub evidence: &'static str,
}

/// A named rung of the height ladders. The spacing row ladder is the
/// AC-level vocabulary; the control heights are its control-side names —
/// `meridian_design::control::binding` derives each control height from a row
/// rung (`control + 2 · pad = row`), so naming one names the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedRung {
    /// `spacing::ROW_DENSE`.
    RowDense,
    /// `spacing::ROW_GRID`.
    RowGrid,
    /// `spacing::ROW_PREVIEW`.
    RowPreview,
    /// `spacing::ROW_COMFORTABLE`.
    RowComfortable,
    /// `control::HEIGHT_XS`.
    ControlXs,
    /// `control::HEIGHT_SM`.
    ControlSm,
    /// `control::HEIGHT_MD`.
    ControlMd,
    /// `control::HEIGHT_LG`.
    ControlLg,
}

impl NamedRung {
    /// The rung's value in logical points, straight from the token crate.
    #[must_use]
    pub fn value(self) -> f32 {
        use meridian_design::{control, spacing};
        match self {
            NamedRung::RowDense => spacing::ROW_DENSE,
            NamedRung::RowGrid => spacing::ROW_GRID,
            NamedRung::RowPreview => spacing::ROW_PREVIEW,
            NamedRung::RowComfortable => spacing::ROW_COMFORTABLE,
            NamedRung::ControlXs => control::HEIGHT_XS,
            NamedRung::ControlSm => control::HEIGHT_SM,
            NamedRung::ControlMd => control::HEIGHT_MD,
            NamedRung::ControlLg => control::HEIGHT_LG,
        }
    }

    /// The token's name, for the gate's failure message.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            NamedRung::RowDense => "spacing::ROW_DENSE",
            NamedRung::RowGrid => "spacing::ROW_GRID",
            NamedRung::RowPreview => "spacing::ROW_PREVIEW",
            NamedRung::RowComfortable => "spacing::ROW_COMFORTABLE",
            NamedRung::ControlXs => "control::HEIGHT_XS",
            NamedRung::ControlSm => "control::HEIGHT_SM",
            NamedRung::ControlMd => "control::HEIGHT_MD",
            NamedRung::ControlLg => "control::HEIGHT_LG",
        }
    }
}

/// The height half of the gate. `Rung` is measured; `Intrinsic` must say
/// *why* there is nothing to measure — empty prose fails the completeness
/// test, so "skipped" always reads as a decision.
#[derive(Clone, Copy, Debug)]
pub enum GateHeight {
    /// The measured node's height equals this named ladder value.
    Rung {
        /// Which rung.
        rung: NamedRung,
        /// The label of the node to measure; `None` measures the probe node.
        node: Option<&'static str>,
    },
    /// No fixed height by design — the prose says why.
    Intrinsic(&'static str),
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// What one catalog entry declares about itself.
#[derive(Clone, Copy, Debug)]
pub struct ComponentInfo {
    /// Stable kebab-case id — the `--gallery <id>` address.
    pub id: &'static str,
    /// The display name the pane shows.
    pub name: &'static str,
    /// Where this entry stands. Draft renders with its pill on show.
    pub status: ComponentStatus,
    /// How the gate finds the specimen.
    pub probe: Probe,
    /// The height contract.
    pub height: GateHeight,
    /// Keyboard actuation, for interactive primitives. `None` for statics.
    pub actuation: Option<Actuation>,
    /// The logical size a solo capture of this entry renders at, when
    /// `--size` does not override it.
    pub solo_size: (f32, f32),
}

/// One gallery entry: what it is, and one live specimen of it. Exactly two
/// methods, by design — everything the gate needs is *data* on
/// [`ComponentInfo`], so the trait cannot grow per-component test hooks.
pub trait Component {
    /// Identity, status, and the conformance contract.
    fn info(&self) -> ComponentInfo;

    /// Draw one live specimen. State the demo needs (a latch, a query
    /// string) lives on the implementing struct.
    fn ui(&mut self, ui: &mut egui::Ui);
}

/// The catalog: the explicit, ordered registration of every gallery entry.
///
/// Deliberately a hand-written list rather than link-time collection: a
/// distributed registry can be stripped by the linker in exactly the builds
/// nobody is looking at, and the completeness grep in `tests/gallery_gate.rs`
/// — which counts `impl Component for` in this file against this list — is
/// the real anti-drift gate either way.
#[must_use]
pub fn catalog() -> Vec<Box<dyn Component>> {
    vec![
        Box::new(ListRowDemo::default()),
        Box::new(StatusPillDemo),
        Box::new(StatusRailDemo),
        Box::new(PaneHeaderDemo),
        Box::new(InspectorDemo),
        Box::new(KeyChipDemo),
        Box::new(QueryLineDemo::default()),
        Box::new(FocusRingDemo::default()),
        Box::new(FieldRowDemo),
        Box::new(IconSetDemo),
        Box::new(ModalChromeDemo::default()),
        Box::new(FeedbackLayersDemo::default()),
        Box::new(PickerDemo::default()),
    ]
}

/// Draw one component solo on a themed page — the shared body of the gate
/// harness and the `--gallery` capture, so what a test measures and what an
/// agent screenshots is the same frame the pane composes per entry.
pub fn solo(ui: &mut egui::Ui, mode: Mode, component: &mut dyn Component) {
    crate::design::apply(ui.ctx(), mode);
    egui::CentralPanel::default().show(ui, |ui| {
        let pad = ui.tokens().panel_padding;
        ui.add_space(pad);
        component.ui(ui);
    });
}

// ---------------------------------------------------------------------------
// The pane
// ---------------------------------------------------------------------------

/// The gallery pane's stable id.
pub const GALLERY: ItemId = ItemId::new("design-gallery");

/// The pane's icon name (resolved when the chrome paints named icons).
const ICON_GALLERY: ItemIcon = ItemIcon("layout-grid");

/// The gallery's registry entry: a centre tab beside the chart, present only
/// when [`enabled`] says so. See [`crate::app::chart_registry_with`].
#[must_use]
pub fn gallery_spec() -> ItemSpec<ChartDoc> {
    ItemSpec {
        id: GALLERY,
        slot: Slot::CentreTab,
        toggle: Some(Verb::new("toggle-gallery")),
        make: || Box::new(GalleryPane::new()),
    }
}

/// The pane: the catalog, rendered entry by entry with name, id, and status
/// pill, inside one scroll surface.
pub struct GalleryPane {
    components: Vec<Box<dyn Component>>,
}

impl GalleryPane {
    /// A pane over the full catalog.
    #[must_use]
    pub fn new() -> Self {
        Self {
            components: catalog(),
        }
    }
}

impl Default for GalleryPane {
    fn default() -> Self {
        Self::new()
    }
}

impl Item<ChartDoc> for GalleryPane {
    fn item_id(&self) -> ItemId {
        GALLERY
    }

    /// The gallery rides beside real content — that is its whole argument
    /// over a separate binary. Over an empty document the window's answer is
    /// the front door, so the pane defers to it rather than drawing swatches
    /// in a vacuum.
    fn empty_state(&self, doc: &ChartDoc) -> Option<EmptyState> {
        doc.is_empty().then(|| {
            EmptyState::new(
                ICON_GALLERY,
                "No document open",
                "The gallery draws the shared component vocabulary beside real \
                 content, under the live theme. Open a dashboard or protocol \
                 and it renders here.",
            )
        })
    }

    fn describe(&self, _doc: &ChartDoc) -> Subject {
        Subject::new("Gallery", ICON_GALLERY, BindingContext::Workspace)
    }

    fn ui(&mut self, _doc: &mut ChartDoc, ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
        let section_gap = ui.tokens().section_gap;
        let control_gap = ui.tokens().control_gap;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for component in &mut self.components {
                    let info = component.info();
                    ui.add_space(section_gap);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(info.name).strong());
                        ui.add_space(control_gap);
                        status_pill_ui(ui, info.status);
                        ui.add_space(control_gap);
                        let muted = to_color32(semantic(ui.visuals().dark_mode).text.muted);
                        ui.label(egui::RichText::new(info.id).monospace().color(muted));
                    });
                    ui.add_space(control_gap);
                    component.ui(ui);
                }
                ui.add_space(section_gap);
            });
    }
}

// ---------------------------------------------------------------------------
// The entries
// ---------------------------------------------------------------------------

/// The evidence label [`ListRowDemo`] shows once a row has activated.
const ROW_EVIDENCE: &str = "row activated";

/// `list_row` — the one row treatment, three rungs of picker-style rows.
#[derive(Default)]
struct ListRowDemo {
    activated: bool,
}

impl Component for ListRowDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "list-row",
            name: "List row",
            status: ComponentStatus::Live,
            probe: Probe {
                role: ProbeRole::Button,
                label: "Line chart",
            },
            height: GateHeight::Rung {
                rung: NamedRung::RowGrid,
                node: None,
            },
            actuation: Some(Actuation {
                focus: FocusTarget::Probe,
                input: ActuationInput::Key(egui::Key::Enter),
                evidence: ROW_EVIDENCE,
            }),
            solo_size: (380.0, 220.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        let ink = to_color32(semantic(dark).text.secondary);
        let rows: [(&str, &icons::Icon, bool); 3] = [
            ("Bar chart", &icons::CHART_BAR, false),
            ("Line chart", &icons::CHART_LINE, true),
            ("Data table", &icons::TABLE, false),
        ];
        for (label, icon, selected) in rows {
            let icon_size = meridian_design::control::ICON_SM;
            let response = list_row(
                ui,
                ListRow::new(RowHeight::Grid).selected(selected),
                |ui, state| {
                    icon.show(ui, icon_size, ink);
                    ui.label(label);
                    if state.hovered {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            key_chip(ui, "↩");
                        });
                    }
                },
            );
            // The row is the click target; the demo labels it so the gate —
            // and a screen reader — can address the *row*, not only the text
            // inside it.
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
            });
            if response.clicked() {
                self.activated = true;
            }
        }
        if self.activated {
            ui.label(ROW_EVIDENCE);
        }
    }
}

/// `status_pill` — the generic pill, with the caller-owned vocabulary shown
/// across the roles it can wear.
struct StatusPillDemo;

impl Component for StatusPillDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "status-pill",
            name: "Status pill",
            status: ComponentStatus::Ready,
            probe: Probe {
                role: ProbeRole::Label,
                label: "ok",
            },
            height: GateHeight::Rung {
                rung: NamedRung::ControlXs,
                node: None,
            },
            actuation: None,
            solo_size: (380.0, 120.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let gap = ui.tokens().control_gap;
        ui.horizontal(|ui| {
            widgets::status_pill(ui, &icons::CIRCLE_CHECK, "ok", Role::Success);
            ui.add_space(gap);
            widgets::status_pill(ui, &icons::CLOCK, "waiting", Role::Neutral);
            ui.add_space(gap);
            widgets::status_pill(ui, &icons::ALERT_TRIANGLE, "failing", Role::Danger);
        });
    }
}

/// The idle line's text, standing in for what `window::idle_status_entry`
/// composes from a real dashboard — fixed here because the demo has no
/// `ChartDoc` behind it, just the same `StatusEntry` carrier.
const STATUS_RAIL_IDLE_PROBE: &str = "loaded · 2 marks";

/// `status_rail` / `status_rail_overlay` — the bottom-edge band a pane's
/// `Subject.status` composes into: leading facts at the left (a held
/// selection), standing state at the right (the idle line, an activity
/// report). Live on the shipping window — see `window::status_rail_ui`.
struct StatusRailDemo;

impl Component for StatusRailDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "status-rail",
            name: "Status rail",
            status: ComponentStatus::Live,
            probe: Probe {
                role: ProbeRole::Label,
                label: STATUS_RAIL_IDLE_PROBE,
            },
            // `chrome::status_rail_height()` is `ROW_GRID + 2·SPACE_2` — a
            // composite of two ladder values, not a single named rung, so
            // this list has no rung to measure it against.
            height: GateHeight::Intrinsic(
                "the rail's band is ROW_GRID + 2·SPACE_2, a composite the \
                 named-rung ladder has no single entry for",
            ),
            actuation: None,
            solo_size: (460.0, 96.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let mode = if ui.visuals().dark_mode {
            Mode::Dark
        } else {
            Mode::Light
        };
        let entries = [
            StatusEntry {
                id: "gallery-status-rail-predicate",
                side: StatusSide::Leading,
                text: "$brush = \"x\" BETWEEN 52 AND 88".to_string(),
                tone: Tone::Neutral,
                hide: HideAffordance::WithRail,
            },
            StatusEntry {
                id: "gallery-status-rail-idle",
                side: StatusSide::Trailing,
                text: STATUS_RAIL_IDLE_PROBE.to_string(),
                tone: Tone::Neutral,
                hide: HideAffordance::WithRail,
            },
        ];
        chrome::status_rail(ui, &entries, mode);
    }
}

/// The label the pane-header demo's band node carries for the gate's
/// height measurement (the band is `Sense::hover`, so it needs a name).
const HEADER_BAND: &str = "specimen header band";

/// `pane_header` — the band, with a leading icon and a trailing slot.
struct PaneHeaderDemo;

impl Component for PaneHeaderDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "pane-header",
            name: "Pane header",
            status: ComponentStatus::Ready,
            probe: Probe {
                role: ProbeRole::Label,
                label: "Specimen pane",
            },
            height: GateHeight::Rung {
                rung: NamedRung::ControlMd,
                node: Some(HEADER_BAND),
            },
            actuation: None,
            solo_size: (380.0, 120.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        let ink = to_color32(semantic(dark).text.secondary);
        let icon_size = meridian_design::control::ICON_SM;
        let band = widgets::pane_header_with(ui, Some(&icons::TABLE), "Specimen pane", |right| {
            icons::X.show(right, icon_size, ink);
        });
        band.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), HEADER_BAND)
        });
    }
}

/// `crate::inspector::render_selection` — the chart view's rail body: a
/// selected pane's title and its toolbar, drawn through the same function the
/// shipping inspector calls, over a standing specimen `Subject` rather than a
/// real focused pane.
struct InspectorDemo;

impl Component for InspectorDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "inspector",
            name: "Inspector",
            status: ComponentStatus::Live,
            probe: Probe {
                role: ProbeRole::Label,
                label: "Chart",
            },
            height: GateHeight::Intrinsic(
                "the inspector's body is content-driven — a title and a \
                 collapsing toolbar row, sized by what the selected pane \
                 declares — so there is no fixed rung to measure",
            ),
            actuation: None,
            solo_size: (300.0, 160.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let mode = if ui.visuals().dark_mode {
            Mode::Dark
        } else {
            Mode::Light
        };
        let subject = Subject::new("Chart", ItemIcon("sliders"), BindingContext::Workspace)
            .with_toolbar(ToolbarEntry::button(
                "clear-selection",
                "Clear selection",
                Verb::new("clear-selection"),
            ));
        // The specimen's toolbar click is not wired to anything live — the
        // gallery has no document to clear a selection on — but the same
        // function draws it, so the gallery cannot show a control the
        // shipping pane would render differently.
        let _ = crate::inspector::render_selection(ui, Some(&subject), mode);
    }
}

/// `key_chip` + `tooltip_for_action` — the keyboard on the surface.
struct KeyChipDemo;

impl Component for KeyChipDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "key-chip",
            name: "Key chip",
            status: ComponentStatus::Live,
            probe: Probe {
                role: ProbeRole::Label,
                label: "Esc",
            },
            height: GateHeight::Intrinsic(
                "the chip hugs its keycap galley; its padding is two ladder \
                 steps, and the box has no rung of its own",
            ),
            actuation: None,
            solo_size: (380.0, 120.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let gap = ui.tokens().control_gap;
        ui.horizontal(|ui| {
            key_chip(ui, "Esc");
            ui.add_space(gap);
            key_chip(ui, "⌘S");
            ui.add_space(gap);
            key_chip(ui, "Space");
            ui.add_space(gap);
            let button = ui.button("Save");
            tooltip_for_action(button, "Save the spec", Some("⌘S"));
        });
    }
}

/// The evidence label [`QueryLineDemo`] shows once text has arrived.
const QUERY_EVIDENCE: &str = "query captured";

/// `query_line` — the shared picker input, one prompt glyph.
#[derive(Default)]
struct QueryLineDemo {
    query: String,
    captured: bool,
}

impl Component for QueryLineDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "query-line",
            name: "Query line",
            status: ComponentStatus::Live,
            probe: Probe {
                role: ProbeRole::Label,
                label: PROMPT_GLYPH,
            },
            height: GateHeight::Intrinsic(
                "the primitive pins its input row at the large control rung \
                 internally; the row is pure layout, so there is no widget \
                 node whose box is the rung",
            ),
            actuation: Some(Actuation {
                focus: FocusTarget::Role(ProbeRole::TextInput),
                input: ActuationInput::Text("meridian"),
                evidence: QUERY_EVIDENCE,
            }),
            solo_size: (380.0, 140.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let response = query_line(ui, &mut self.query, "Type to filter…");
        if response.changed && !self.query.is_empty() {
            self.captured = true;
        }
        if self.captured {
            ui.label(QUERY_EVIDENCE);
        }
    }
}

/// The evidence label [`FocusRingDemo`] shows once its button has pressed.
const RING_EVIDENCE: &str = "button pressed";

/// `focus_ring` — the one focus treatment: a standing exemplar ring, and a
/// live button that wears it exactly when focused.
#[derive(Default)]
struct FocusRingDemo {
    pressed: bool,
}

impl Component for FocusRingDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "focus-ring",
            name: "Focus ring",
            status: ComponentStatus::Live,
            probe: Probe {
                role: ProbeRole::Button,
                label: "Focus me",
            },
            height: GateHeight::Rung {
                rung: NamedRung::ControlMd,
                node: None,
            },
            actuation: Some(Actuation {
                focus: FocusTarget::Probe,
                input: ActuationInput::Key(egui::Key::Enter),
                evidence: RING_EVIDENCE,
            }),
            solo_size: (380.0, 140.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let tokens = ui.tokens();
        let gap = tokens.section_gap;
        let radius = tokens.radius_control;
        let swatch = meridian_design::control::HEIGHT_SM;
        ui.horizontal(|ui| {
            // The standing exemplar: the ring's geometry and colour, visible
            // without holding focus — goldens would otherwise show nothing.
            let dark = ui.visuals().dark_mode;
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(swatch, swatch), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, radius, to_color32(semantic(dark).surfaces.raised));
            widgets::focus_ring(ui, rect, radius);
            ui.add_space(gap);
            let response = ui.button("Focus me");
            widgets::focus_ring_for(ui, &response);
            if response.clicked() {
                self.pressed = true;
            }
        });
        if self.pressed {
            ui.label(RING_EVIDENCE);
        }
    }
}

/// `field_row` inside an alignment scope — the shared value column.
struct FieldRowDemo;

impl Component for FieldRowDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "field-row",
            name: "Field row",
            status: ComponentStatus::Ready,
            probe: Probe {
                role: ProbeRole::Label,
                label: "example/things",
            },
            height: GateHeight::Intrinsic(
                "a field row self-sizes from its label galley plus ladder \
                 leading; the designed property is the shared split column, \
                 held by the alignment scope and asserted in the emitter's \
                 own tests",
            ),
            actuation: None,
            solo_size: (380.0, 160.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        align::align_scope(ui, "gallery-fields", |ui| {
            widgets::field_row(
                ui,
                "address",
                "example/things",
                Some("where it lives"),
                true,
            );
            widgets::field_row(ui, "rows", "12,480", None, true);
            widgets::field_row(ui, "owner", "meridian", None, false);
        });
    }
}

/// The Tabler icon set — named glyphs at the ladder sizes.
struct IconSetDemo;

impl Component for IconSetDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "icon-set",
            name: "Icons",
            status: ComponentStatus::Live,
            probe: Probe {
                role: ProbeRole::Label,
                label: "table",
            },
            height: GateHeight::Intrinsic(
                "glyphs draw at the icon ladder's sizes inside whatever row \
                 hosts them; the box belongs to the host",
            ),
            actuation: None,
            solo_size: (380.0, 160.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let gap = ui.tokens().section_gap;
        let dark = ui.visuals().dark_mode;
        let ink = to_color32(semantic(dark).text.secondary);
        let muted = to_color32(semantic(dark).text.muted);
        let size = meridian_design::control::ICON_XL;
        ui.horizontal(|ui| {
            for icon in [
                &icons::TABLE,
                &icons::SEARCH,
                &icons::FILTER,
                &icons::DATABASE,
                &icons::SETTINGS,
                &icons::CHART_BAR,
            ] {
                ui.vertical(|ui| {
                    icon.show(ui, size, ink);
                    ui.label(egui::RichText::new(icon.name).color(muted));
                });
                ui.add_space(gap);
            }
        });
    }
}

/// The modal chrome — a Draft entry: the treatment ships live through
/// `ModalLayer`, and the inline specimen the gate can hold is pending.
#[derive(Default)]
struct ModalChromeDemo {
    open: bool,
}

impl Component for ModalChromeDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "modal-chrome",
            name: "Modal chrome",
            status: ComponentStatus::Draft,
            probe: Probe {
                role: ProbeRole::Button,
                label: "Open modal chrome",
            },
            height: GateHeight::Intrinsic(
                "the card's widths are the two modal rungs and its height is \
                 content-driven; the gate needs an inline specimen before it \
                 can measure one",
            ),
            actuation: None,
            solo_size: (380.0, 140.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        if ui.button("Open modal chrome").clicked() {
            self.open = true;
        }
        if self.open {
            let chrome = ModalChrome::new().title("Specimen modal").narrow();
            let shown = ModalLayer::show(ui.ctx(), "gallery-modal", &chrome, |ui| {
                ui.label(
                    "One overlay treatment: width rung, title row, escape \
                     hint, backdrop. Esc or a backdrop click closes it.",
                );
            });
            if shown.dismissed {
                self.open = false;
            }
        }
    }
}

/// The banner + toast layers — a Draft entry: both layers ship live in the
/// window; the inline specimen the gate can hold is pending.
#[derive(Default)]
struct FeedbackLayersDemo {
    notifications: NotificationLayer,
    toasts: ToastLayer,
}

impl Component for FeedbackLayersDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "feedback-layers",
            name: "Banners and toasts",
            status: ComponentStatus::Draft,
            probe: Probe {
                role: ProbeRole::Button,
                label: "Raise banner",
            },
            height: GateHeight::Intrinsic(
                "banners and toasts are context layers over the whole window; \
                 the gate needs an inline specimen before it can measure one",
            ),
            actuation: None,
            solo_size: (380.0, 140.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let gap = ui.tokens().control_gap;
        ui.horizontal(|ui| {
            if ui.button("Raise banner").clicked() {
                self.notifications.raise(
                    Notification::new(
                        NotificationId::composite("gallery", "specimen"),
                        Severity::Warning,
                        "A standing condition",
                    )
                    .body("Re-raising under the same id replaces this banner."),
                );
            }
            ui.add_space(gap);
            if ui.button("Resolve banner").clicked() {
                // Dismissing an id that is not raised is a no-op by design.
                let _ = self
                    .notifications
                    .dismiss(NotificationId::composite("gallery", "specimen"));
            }
            ui.add_space(gap);
            if ui.button("Pop toast").clicked() {
                self.toasts
                    .push(Toast::new(Severity::Info, "A confirmation"));
            }
        });
        self.notifications.show(ui.ctx());
        self.toasts.show(ui.ctx());
    }
}

/// The evidence label [`PickerDemo`] shows once the query has narrowed the
/// corpus — proof the composite responds to the keyboard, not just decorates
/// (the confirm/dispatch half is proven separately, per verb, by
/// `overlay_wiring.rs`'s dispatchability tests — this specimen exists to show
/// the picker is the live thing, not to re-litigate what a confirm does).
const PICKER_EVIDENCE: &str = "the query narrowed the corpus";

/// The picker — drawn inline rather than behind `ModalLayer`, over the SAME
/// [`CommandPalette`] delegate `window.rs` opens on `space`: `query_line` and
/// `list_row` are gated above on their own; this is the composite that
/// stacks them.
struct PickerDemo {
    picker: Picker<CommandPalette>,
}

impl Default for PickerDemo {
    fn default() -> Self {
        Self {
            picker: Picker::new(CommandPalette::new(
                Altitude::Protocol,
                RecencyCounter::new(),
            )),
        }
    }
}

impl Component for PickerDemo {
    fn info(&self) -> ComponentInfo {
        ComponentInfo {
            id: "picker",
            name: "Picker",
            status: ComponentStatus::Live,
            probe: Probe {
                role: ProbeRole::Label,
                label: PROMPT_GLYPH,
            },
            height: GateHeight::Intrinsic(
                "the picker stacks a query line over its list rows, each \
                 already gated on its own (`query-line`, `list-row`); the \
                 composite's height is the row count times the row rung, so \
                 there is no single rung for the whole stack",
            ),
            actuation: Some(Actuation {
                focus: FocusTarget::Role(ProbeRole::TextInput),
                input: ActuationInput::Text("steps"),
                evidence: PICKER_EVIDENCE,
            }),
            solo_size: (380.0, 260.0),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.picker.show(ui);
        if !self.picker.query().is_empty() {
            ui.label(PICKER_EVIDENCE);
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Every named rung resolves to the token it names — the map cannot
    /// drift from the ladder it points into.
    #[test]
    fn named_rungs_are_the_ladder_values() {
        use meridian_design::{control, spacing};
        assert_eq!(NamedRung::RowDense.value(), spacing::ROW_DENSE);
        assert_eq!(NamedRung::RowGrid.value(), spacing::ROW_GRID);
        assert_eq!(NamedRung::RowPreview.value(), spacing::ROW_PREVIEW);
        assert_eq!(NamedRung::RowComfortable.value(), spacing::ROW_COMFORTABLE);
        assert_eq!(NamedRung::ControlXs.value(), control::HEIGHT_XS);
        assert_eq!(NamedRung::ControlSm.value(), control::HEIGHT_SM);
        assert_eq!(NamedRung::ControlMd.value(), control::HEIGHT_MD);
        assert_eq!(NamedRung::ControlLg.value(), control::HEIGHT_LG);
    }

    /// Every control rung is bound to a spacing row rung by the token
    /// crate's own binding rule — the bridge the gate's height vocabulary
    /// stands on.
    #[test]
    fn control_rungs_are_bound_to_row_rungs() {
        use meridian_design::spacing;
        for rung in [
            NamedRung::ControlXs,
            NamedRung::ControlSm,
            NamedRung::ControlMd,
            NamedRung::ControlLg,
        ] {
            let control = rung.value();
            assert!(
                spacing::ROWS
                    .iter()
                    .any(|row| (row - control).abs() <= spacing::SPACE_2 * 2.0),
                "{} ({control}) is not within one padding pair of any row rung",
                rung.name()
            );
        }
    }

    /// The flag reads as documented: unset off, `0` off, anything else on.
    /// Runs as one test because the environment is process-global.
    #[test]
    fn the_flag_reads_unset_zero_and_set() {
        // Serialised within this one test; no other test reads the variable.
        std::env::remove_var(GALLERY_VAR);
        assert!(!enabled());
        std::env::set_var(GALLERY_VAR, "0");
        assert!(!enabled());
        std::env::set_var(GALLERY_VAR, "1");
        assert!(enabled());
        std::env::remove_var(GALLERY_VAR);
    }
}
