//! The command registry: every verb as data, the single source of truth
//! for the keymap-as-data vec, the palette corpus, and the help sheet.
//!
//! Framework-free by construction — no gpui types cross this boundary. The GPUI
//! adapter turns a [`BindingSpec`] into a `gpui::KeyBinding` and maps a `longname`
//! to its action; nothing here knows about gpui.

use crate::altitude::Altitude;

/// A framework-free keystroke descriptor. Carries NO gpui types (the standing
/// framework-free rule): the adapter builds `gpui::KeyBinding` from this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingSpec {
    /// Space-separated keystrokes, mirroring gpui's `KeyBinding` string but as
    /// plain data, e.g. `"p"`, `"cmd-e"`, `"g f"`.
    pub keystrokes: &'static str,
    /// The key-context this binding resolves in.
    pub context: BindingContext,
}

/// The key-context a binding resolves in. The adapter maps it to a gpui context
/// predicate: `Workspace` → `"BrightfieldWorkspace"`, `Editor` →
/// `"BrightfieldEditor"`, `Global` → `None` (fires regardless of focus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingContext {
    /// Canvas-scoped: fires only while the canvas holds focus (bare verbs).
    Workspace,
    /// Editor-scoped: fires only while the YAML editor holds focus.
    Editor,
    /// Protocol-panel-scoped: fires only while the protocol asset-graph panel
    /// holds focus. A distinct context so the panel's topological
    /// `h`/`l`/`j`/`k` never collide with the chart grammar's nav bindings.
    Protocol,
    /// Global (`context = None`): fires from any focus (palette twin, focus
    /// toggle, save/reload-from-anywhere).
    Global,
}

/// Recorded provenance for a bound key: the scores that DEFEND the key
/// choice, traced to `keymap-research.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scores {
    /// How often the verb is used (frequency tier, 1–5).
    pub frequency: u8,
    /// How well the key mnemonically fits the verb (1–5).
    pub mnemonic: u8,
    /// How well the key matches cross-tool convention (1–5).
    pub convention: u8,
    /// A short motor-cost / rationale note.
    pub motor_note: &'static str,
}

/// What a verb ultimately drives (mirrors the spec ontology's `drives` enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drives {
    /// A runtime engine effect (clear-selection, reload).
    RuntimeDispatch,
    /// Focus movement across the ComponentPath tree.
    Navigation,
    /// Presentation-mode toggle.
    Presentation,
    /// Palette / help meta-surfaces.
    PaletteMeta,
    /// The transient colour-scheme preview.
    ColourPreview,
    /// A structural spec edit through the command log: change-mark-
    /// type / add-mark / set-channel / remove-mark / undo. NOT `g`-broadcast
    /// eligible (that is runtime-dispatch only, `scope::g_eligible`).
    SpecEdit,
    /// A deferred verb, shown but unbound.
    Reserved,
}

/// The command tier — the cmdlog discipline's REQUIRED taxonomy,
/// enforced from the first commit because it cannot be retrofitted.
///
/// Every verb declares which tier it is. The split governs the command log: a
/// **View** command (navigation, folds, panes, palette/help) is never logged —
/// it changes only what you look at. A **Data** command carries a stable dotted
/// address (an asset id / focused node), and IS logged as `longname + address +
/// input`, JSONL, **never** a screen position or pointer movement. A logged row
/// is shape-identical to an `op:`/`with:` step, so a session's Data rows compile
/// straight into a protocol fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTier {
    /// Changes only the view (nav / fold / pane / meta). Never logged.
    View,
    /// Acts on addressed data. Logged by longname + dotted address + input.
    Data,
}

impl CommandTier {
    /// Whether commands at this tier are written to the command log.
    #[must_use]
    pub fn is_logged(self) -> bool {
        matches!(self, CommandTier::Data)
    }
}

/// Lifecycle status of a verb in this card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbStatus {
    /// Wired this card (a live binding).
    Built,
    /// `c` = cycle-colour-scheme: wired but transient / non-durable.
    Preview,
    /// Shown greyed in the palette, unbound — deferred to a follow-up card.
    Reserved,
}

/// Why a reserved verb is not yet available — the two named buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservedReason {
    /// `m` / `a` / `e` / `d` / undo — need the command log (the SpecEdit AST
    /// mutation API) to persist a structural change.
    NeedsCommandLog,
    /// `f` / `g f` / `t` / set-param — need a keyboard data-target: a way to
    /// name a predicate without a pointer-derived rectangle or point.
    NeedsKeyboardTarget,
}

impl ReservedReason {
    /// A human-readable reason, surfaced in the palette flag and in a
    /// scope-resolver rejection.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            ReservedReason::NeedsCommandLog => "needs command log",
            ReservedReason::NeedsKeyboardTarget => "needs a keyboard target",
        }
    }
}

/// One row of the command registry — the single verb-metadata record.
#[derive(Debug, Clone, PartialEq)]
pub struct VerbEntry {
    /// Stable kebab-case canonical name, e.g. `cycle-colour-scheme`.
    pub longname: &'static str,
    /// Framework-free keystroke descriptors; EMPTY for reserved (palette-only) verbs.
    pub binding_specs: Vec<BindingSpec>,
    /// The altitudes at which the verb is meaningful (`no mark in v1`). The SAME
    /// set governs bare-key resolution and palette candidacy.
    pub scope_applicability: Vec<Altitude>,
    /// What the verb drives.
    pub drives: Drives,
    /// The cmdlog tier — REQUIRED on every verb. Governs whether the
    /// verb logs, and, for `Data`, that it logs by dotted address (never a
    /// screen position). Enforced by [`registry`]'s construction, so a new verb
    /// cannot be added without a deliberate tier call.
    pub tier: CommandTier,
    /// Lifecycle status.
    pub status: VerbStatus,
    /// For reserved verbs: which bucket blocks it. `None` for active verbs.
    pub reserved_reason: Option<ReservedReason>,
    /// One-line description; part of the palette fuzzy corpus alongside the longname.
    pub help: &'static str,
    /// Provenance for a bound key. Present for every bound key; `None` for reserved.
    pub scores: Option<Scores>,
}

impl VerbEntry {
    /// Whether this verb has at least one live binding.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        !self.binding_specs.is_empty()
    }

    /// Whether this verb is reserved (shown, unbound).
    #[must_use]
    pub fn is_reserved(&self) -> bool {
        matches!(self.status, VerbStatus::Reserved)
    }

    /// Whether the verb applies at `altitude`.
    #[must_use]
    pub fn applies_at(&self, altitude: Altitude) -> bool {
        self.scope_applicability.contains(&altitude)
    }

    /// The first bound keystroke, shown inline in the palette / help. `None` for reserved.
    #[must_use]
    pub fn primary_key(&self) -> Option<&'static str> {
        self.binding_specs.first().map(|b| b.keystrokes)
    }
}

// ---------------------------------------------------------------------------
// The v1 registry
// ---------------------------------------------------------------------------

const DASHBOARD_AND_VIEW: &[Altitude] = &[Altitude::Dashboard, Altitude::View];

/// Build the v1 command registry: every verb (built, preview, reserved) as data.
///
/// This is the sole verb-metadata input to [`keymap_bindings`], [`palette_corpus`],
/// and [`help_sheet`].
#[must_use]
pub fn registry() -> Vec<VerbEntry> {
    use Altitude::{Dashboard, Protocol, View};
    use BindingContext::{Editor, Global, Workspace};
    use Drives as D;

    let ws = |k: &'static str| BindingSpec {
        keystrokes: k,
        context: Workspace,
    };
    let global = |k: &'static str| BindingSpec {
        keystrokes: k,
        context: Global,
    };
    let editor = |k: &'static str| BindingSpec {
        keystrokes: k,
        context: Editor,
    };
    let proto = |k: &'static str| BindingSpec {
        keystrokes: k,
        context: BindingContext::Protocol,
    };

    vec![
        // ---- navigation ----
        VerbEntry {
            longname: "dive-in",
            tier: CommandTier::View,
            binding_specs: vec![ws("l"), ws("enter")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Dive into the focused container (stops at a view)",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row l = right/in (ranger, miller-columns)" }),
        },
        VerbEntry {
            longname: "pop-out",
            tier: CommandTier::View,
            binding_specs: vec![ws("h"), ws("q")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Pop focus out to the parent",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row h = left/out (ranger, miller-columns)" }),
        },
        VerbEntry {
            longname: "focus-next-sibling",
            tier: CommandTier::View,
            binding_specs: vec![ws("j"), ws("tab")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move focus to the next sibling view",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row j = down/next (vim)" }),
        },
        VerbEntry {
            longname: "focus-prev-sibling",
            tier: CommandTier::View,
            binding_specs: vec![ws("k"), ws("shift-tab")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move focus to the previous sibling view",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row k = up/prev (vim)" }),
        },
        VerbEntry {
            longname: "toggle-focus",
            tier: CommandTier::View,
            binding_specs: vec![global("cmd-e")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Toggle focus between the canvas and the editor",
            scores: Some(Scores { frequency: 3, mnemonic: 3, convention: 3, motor_note: "cmd-e = editor swap; free of gpui-component Input's chord set" }),
        },
        VerbEntry {
            longname: "focus-jump",
            tier: CommandTier::View,
            binding_specs: vec![ws("/")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Fuzzy-jump focus to a component by name",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 5, motor_note: "/ = search/jump (vim, less)" }),
        },
        // ---- palette + help ----
        VerbEntry {
            longname: "open-palette",
            tier: CommandTier::View,
            binding_specs: vec![ws("space"), global("cmd-shift-p")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::PaletteMeta,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Open the command palette to find a verb by meaning",
            scores: Some(Scores { frequency: 5, mnemonic: 5, convention: 5, motor_note: "space = palette (helix, which-key); cmd-shift-p global twin (VS Code)" }),
        },
        VerbEntry {
            longname: "open-help",
            tier: CommandTier::View,
            binding_specs: vec![ws("?")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::PaletteMeta,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Show the keyboard help sheet",
            scores: Some(Scores { frequency: 2, mnemonic: 4, convention: 5, motor_note: "? = help (near-universal convention)" }),
        },
        // ---- runtime verbs ----
        VerbEntry {
            longname: "clear-selection",
            tier: CommandTier::View,
            binding_specs: vec![ws("escape")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Clear the focused view's selection",
            scores: Some(Scores { frequency: 4, mnemonic: 4, convention: 5, motor_note: "esc = cancel/clear (universal); terminal rung of the Esc ladder" }),
        },
        VerbEntry {
            longname: "reload-spec",
            tier: CommandTier::View,
            binding_specs: vec![global("cmd-r")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Reload the spec from disk (guards unsaved editor edits)",
            scores: Some(Scores { frequency: 2, mnemonic: 4, convention: 4, motor_note: "cmd-r = reload (browser); bare r NOT bound (dirty-guard)" }),
        },
        // ---- presentation + save: shipped fixed points, sourced here so
        //      the registry is the single binding source ----
        VerbEntry {
            longname: "toggle-presentation",
            tier: CommandTier::View,
            binding_specs: vec![ws("p")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Presentation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Toggle presentation mode (hide authoring chrome)",
            scores: Some(Scores { frequency: 2, mnemonic: 3, convention: 3, motor_note: "p = present (shipped fixed point)" }),
        },
        VerbEntry {
            longname: "save-spec",
            tier: CommandTier::View,
            binding_specs: vec![editor("cmd-s")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Save the spec (editor)",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 5, motor_note: "cmd-s = save (universal; shipped, editor-scoped)" }),
        },
        // ---- colour preview: transient, view-scoped ----
        VerbEntry {
            longname: "cycle-colour-scheme",
            tier: CommandTier::View,
            binding_specs: vec![ws("c")],
            scope_applicability: vec![View],
            drives: D::ColourPreview,
            status: VerbStatus::Preview,
            reserved_reason: None,
            help: "Cycle the focused view's sequential colour scheme (transient preview)",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 3, motor_note: "c = colour (mnemonic); view-scoped, no write" }),
        },
        // ---- reserved: needs a keyboard data-target (f / g f / t / set-param) ----
        reserved("filter-view", vec![View], ReservedReason::NeedsKeyboardTarget, "Filter to the focused view's selection (needs a keyboard target)"),
        reserved("cross-filter-all", vec![Dashboard], ReservedReason::NeedsKeyboardTarget, "Broadcast a cross-filter to every view (needs a keyboard target)"),
        reserved("toggle-point-select", vec![View], ReservedReason::NeedsKeyboardTarget, "Toggle a point selection (needs a keyboard target)"),
        reserved("set-param", DASHBOARD_AND_VIEW.to_vec(), ReservedReason::NeedsKeyboardTarget, "Set a parameter's value (needs a keyboard target)"),
        // ---- command-log structural edits: m/a/e/d at View, undo
        //      at Dashboard+View. Flipped Reserved -> Built — the SpecEdit spine
        //      + Session::reload_spec seam now back them. ----
        VerbEntry {
            longname: "change-mark-type",
            tier: CommandTier::Data,
            binding_specs: vec![ws("m")],
            scope_applicability: vec![View],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Change the focused view's mark type (dot -> bar), applied live",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 3, motor_note: "m = mark/morph; bare, view-scoped structural edit" }),
        },
        VerbEntry {
            longname: "add-mark",
            tier: CommandTier::Data,
            binding_specs: vec![ws("a")],
            scope_applicability: vec![View],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Add a mark to the focused view (prompts for a kind), applied live",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 4, motor_note: "a = add/append (vim insert family); argument overlay picks the kind" }),
        },
        VerbEntry {
            longname: "set-channel",
            tier: CommandTier::Data,
            binding_specs: vec![ws("e")],
            scope_applicability: vec![View],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Bind a channel to a column on the focused view (prompts), applied live",
            scores: Some(Scores { frequency: 3, mnemonic: 3, convention: 3, motor_note: "e = encode/edit-channel; argument overlay picks channel then column" }),
        },
        VerbEntry {
            longname: "remove-mark",
            tier: CommandTier::Data,
            binding_specs: vec![ws("d")],
            scope_applicability: vec![View],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Remove the focused view's primary mark, applied live",
            scores: Some(Scores { frequency: 2, mnemonic: 4, convention: 4, motor_note: "d = delete (vim); refused if it would empty the plot" }),
        },
        VerbEntry {
            longname: "undo",
            tier: CommandTier::View,
            binding_specs: vec![ws("u")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Undo the last uncommitted edit (cannot cross a commit)",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 5, motor_note: "u = undo (vim); snapshot-stack pop, stops at a commit barrier" }),
        },
        // ---- protocol altitude: the asset-graph grammar. All the
        //      motion/fold/drill verbs are View-tier (never logged); the object
        //      verb that names an asset is Data-tier (logged by dotted address). ----
        VerbEntry {
            longname: "protocol-producer",
            tier: CommandTier::View,
            binding_specs: vec![proto("h")],
            scope_applicability: vec![Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Vim h (left): the producer when the flow runs left-to-right, else the sibling to the left — resolved by the drawn layout",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "h = left; producer along a horizontal flow, sibling-left across a vertical one — follows the rendered axis" }),
        },
        VerbEntry {
            longname: "protocol-consumer",
            tier: CommandTier::View,
            binding_specs: vec![proto("l")],
            scope_applicability: vec![Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Vim l (right): the consumer when the flow runs left-to-right, else the sibling to the right — resolved by the drawn layout",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "l = right; consumer along a horizontal flow, sibling-right across a vertical one — follows the rendered axis" }),
        },
        VerbEntry {
            longname: "protocol-sibling-next",
            tier: CommandTier::View,
            binding_specs: vec![proto("j")],
            scope_applicability: vec![Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Vim j (down): the consumer when the flow runs top-to-bottom, else the next sibling below — resolved by the drawn layout",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "j = down; consumer along a vertical flow, next sibling across a horizontal one — nearest in the pressed direction" }),
        },
        VerbEntry {
            longname: "protocol-sibling-prev",
            tier: CommandTier::View,
            binding_specs: vec![proto("k")],
            scope_applicability: vec![Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Vim k (up): the producer when the flow runs top-to-bottom, else the previous sibling above — resolved by the drawn layout",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "k = up; producer along a vertical flow, previous sibling across a horizontal one — nearest in the pressed direction" }),
        },
        VerbEntry {
            longname: "toggle-fold-family",
            tier: CommandTier::View,
            binding_specs: vec![proto("z a")],
            scope_applicability: vec![Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Fold/unfold the parameterised family under the cursor",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 5, motor_note: "za = toggle fold (vim fold family); a fold is a view change, never logged" }),
        },
        VerbEntry {
            longname: "protocol-drill-in",
            tier: CommandTier::View,
            binding_specs: vec![proto("enter")],
            scope_applicability: vec![Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Drill into the focused asset (push the drill stack)",
            scores: Some(Scores { frequency: 4, mnemonic: 4, convention: 5, motor_note: "enter = dive (miller-columns); pushes a breadcrumb the drill stack tracks" }),
        },
        VerbEntry {
            longname: "protocol-drill-out",
            tier: CommandTier::View,
            binding_specs: vec![proto("escape")],
            scope_applicability: vec![Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Pop one level off the drill stack",
            scores: Some(Scores { frequency: 4, mnemonic: 4, convention: 5, motor_note: "esc = pop one level (Esc ladder); breadcrumb tracks the pop" }),
        },
        VerbEntry {
            longname: "open-steps-sheet",
            tier: CommandTier::View,
            binding_specs: vec![proto("shift-s")],
            scope_applicability: vec![Protocol],
            drives: D::PaletteMeta,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Open the S steps sheet — the flat step list as a grid",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 4, motor_note: "S = steps sheet (VisiData sheet family); answers 'where is my step list'" }),
        },
        VerbEntry {
            longname: "yank-address",
            tier: CommandTier::Data,
            binding_specs: vec![proto("y")],
            scope_applicability: vec![Protocol],
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Yank the focused asset's dotted address to the clipboard",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 4, motor_note: "y = yank (vim); a Data verb — logged by longname + dotted address" }),
        },
    ]
}

/// Construct a reserved (unbound, palette-visible) verb entry.
fn reserved(
    longname: &'static str,
    scope_applicability: Vec<Altitude>,
    reason: ReservedReason,
    help: &'static str,
) -> VerbEntry {
    VerbEntry {
        longname,
        // Every reserved verb here needs a keyboard data-target — i.e. a dotted
        // address to act on — so each is a Data-tier command by construction.
        tier: CommandTier::Data,
        binding_specs: Vec::new(),
        scope_applicability,
        drives: Drives::Reserved,
        status: VerbStatus::Reserved,
        reserved_reason: Some(reason),
        help,
        scores: None,
    }
}

// ---------------------------------------------------------------------------
// Producers: each takes the registry as its sole verb-metadata input
// ---------------------------------------------------------------------------

/// One bound key in the keymap-as-data vec: the projection the GPUI adapter
/// consumes to build a `gpui::KeyBinding`, and the input to the
/// dispatch-resolution table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundKey {
    /// The verb this key runs.
    pub longname: &'static str,
    /// Space-separated keystrokes.
    pub keystrokes: &'static str,
    /// The context this binding resolves in.
    pub context: BindingContext,
}

/// The keymap-as-data vec: the SINGLE binding source. The adapter maps
/// `longname` → action and `context` → predicate to build `gpui::KeyBinding`s.
#[must_use]
pub fn keymap_bindings(reg: &[VerbEntry]) -> Vec<BoundKey> {
    reg.iter()
        .flat_map(|v| {
            v.binding_specs.iter().map(move |b| BoundKey {
                longname: v.longname,
                keystrokes: b.keystrokes,
                context: b.context,
            })
        })
        .collect()
}

/// One palette row derived from the registry.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteEntry {
    /// The verb.
    pub longname: &'static str,
    /// One-line help (part of the fuzzy corpus).
    pub help: &'static str,
    /// The bound key shown inline; `None` for reserved.
    pub primary_key: Option<&'static str>,
    /// If reserved, the bucket it is flagged with.
    pub reserved_reason: Option<ReservedReason>,
    /// Frequency tier for empty-query ordering (0 if unscored).
    pub frequency: u8,
    /// The altitudes the verb applies at (for scope filtering downstream).
    pub scope_applicability: Vec<Altitude>,
}

/// The full palette corpus: one row per verb, reserved included. The
/// scope filtering / fuzzy ranking is [`crate::palette::palette_filter`].
#[must_use]
pub fn palette_corpus(reg: &[VerbEntry]) -> Vec<PaletteEntry> {
    reg.iter()
        .map(|v| PaletteEntry {
            longname: v.longname,
            help: v.help,
            primary_key: v.primary_key(),
            reserved_reason: v.reserved_reason,
            frequency: v.scores.as_ref().map_or(0, |s| s.frequency),
            scope_applicability: v.scope_applicability.clone(),
        })
        .collect()
}

/// One row of the help sheet, grouped by scope in the overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct HelpRow {
    /// The verb.
    pub longname: &'static str,
    /// Every bound keystroke (empty for reserved).
    pub keys: Vec<&'static str>,
    /// One-line help.
    pub help: &'static str,
    /// The altitudes the verb applies at (the grouping key).
    pub altitudes: Vec<Altitude>,
    /// If reserved, the bucket it is flagged with.
    pub reserved_reason: Option<ReservedReason>,
}

/// The help sheet: every verb with its keys, help, scope, and (if
/// reserved) its bucket.
#[must_use]
pub fn help_sheet(reg: &[VerbEntry]) -> Vec<HelpRow> {
    reg.iter()
        .map(|v| HelpRow {
            longname: v.longname,
            keys: v.binding_specs.iter().map(|b| b.keystrokes).collect(),
            help: v.help,
            altitudes: v.scope_applicability.clone(),
            reserved_reason: v.reserved_reason,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_kebab_case(s: &str) -> bool {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !s.starts_with('-')
            && !s.ends_with('-')
    }

    #[test]
    fn longnames_unique_and_kebab_case() {
        let reg = registry();
        let mut seen = std::collections::HashSet::new();
        for v in &reg {
            assert!(is_kebab_case(v.longname), "not kebab-case: {}", v.longname);
            assert!(
                seen.insert(v.longname),
                "duplicate longname: {}",
                v.longname
            );
        }
    }

    #[test]
    fn reserved_buckets_present_with_reasons() {
        let reg = registry();
        // The two named reserved vocab sets, exactly.
        let mut needs_log: Vec<&str> = reg
            .iter()
            .filter(|v| v.reserved_reason == Some(ReservedReason::NeedsCommandLog))
            .map(|v| v.longname)
            .collect();
        let mut needs_target: Vec<&str> = reg
            .iter()
            .filter(|v| v.reserved_reason == Some(ReservedReason::NeedsKeyboardTarget))
            .map(|v| v.longname)
            .collect();
        needs_log.sort_unstable();
        needs_target.sort_unstable();
        // The command log flipped the 5 verbs Reserved -> Built, so the
        // NeedsCommandLog bucket is now EMPTY (deliberate update; the enum
        // variant + its reason() surface are retained). NeedsKeyboardTarget is
        // unchanged.
        assert!(
            needs_log.is_empty(),
            "NeedsCommandLog bucket is now empty: {needs_log:?}"
        );
        assert_eq!(
            needs_target,
            [
                "cross-filter-all",
                "filter-view",
                "set-param",
                "toggle-point-select"
            ]
        );
        // Every reserved verb is unbound and unscored; every bound verb is scored.
        for v in &reg {
            if v.is_reserved() {
                assert!(
                    v.binding_specs.is_empty(),
                    "reserved {} is bound",
                    v.longname
                );
                assert!(v.scores.is_none(), "reserved {} is scored", v.longname);
                assert!(
                    v.reserved_reason.is_some(),
                    "reserved {} has no reason",
                    v.longname
                );
            } else {
                assert!(v.is_bound(), "active {} is unbound", v.longname);
                assert!(v.scores.is_some(), "bound {} is unscored", v.longname);
                assert!(
                    v.reserved_reason.is_none(),
                    "active {} has a reserved reason",
                    v.longname
                );
            }
        }
    }

    #[test]
    fn longname_snapshot_is_stable() {
        // A committed snapshot of longnames: any add/remove/rename is a deliberate
        // change that must update this list (stability guard).
        let got: Vec<&str> = registry().iter().map(|v| v.longname).collect();
        let expected = [
            "dive-in",
            "pop-out",
            "focus-next-sibling",
            "focus-prev-sibling",
            "toggle-focus",
            "focus-jump",
            "open-palette",
            "open-help",
            "clear-selection",
            "reload-spec",
            "toggle-presentation",
            "save-spec",
            "cycle-colour-scheme",
            "filter-view",
            "cross-filter-all",
            "toggle-point-select",
            "set-param",
            "change-mark-type",
            "add-mark",
            "set-channel",
            "remove-mark",
            "undo",
            "protocol-producer",
            "protocol-consumer",
            "protocol-sibling-next",
            "protocol-sibling-prev",
            "toggle-fold-family",
            "protocol-drill-in",
            "protocol-drill-out",
            "open-steps-sheet",
            "yank-address",
        ];
        assert_eq!(got, expected);
    }

    #[test]
    fn every_verb_declares_a_tier_and_only_data_logs() {
        // The cmdlog discipline's REQUIRED field: every verb carries a tier, and
        // exactly the Data tier is logged. Navigation/fold/pane/meta verbs are
        // View; the spec-edit + object verbs that name a dotted address are Data.
        let reg = registry();
        let logged: Vec<&str> = reg
            .iter()
            .filter(|v| v.tier.is_logged())
            .map(|v| v.longname)
            .collect();
        // The Data-tier set is exactly the addressed verbs.
        let mut got = logged.clone();
        got.sort_unstable();
        let mut expected = vec![
            "change-mark-type",
            "add-mark",
            "set-channel",
            "remove-mark",
            "filter-view",
            "cross-filter-all",
            "toggle-point-select",
            "set-param",
            "yank-address",
        ];
        expected.sort_unstable();
        assert_eq!(got, expected, "only addressed (Data) verbs log");
        // Every protocol MOTION verb is View-tier (a fold/nav is never logged).
        for name in [
            "protocol-producer",
            "protocol-consumer",
            "protocol-sibling-next",
            "protocol-sibling-prev",
            "toggle-fold-family",
            "protocol-drill-in",
            "protocol-drill-out",
            "open-steps-sheet",
        ] {
            let v = reg.iter().find(|v| v.longname == name).unwrap();
            assert_eq!(
                v.tier,
                CommandTier::View,
                "{name} is a view command, never logged"
            );
        }
    }

    #[test]
    fn protocol_altitude_verbs_are_scoped_to_protocol_only() {
        // Protocol verbs never leak into the chart grammar: they apply at
        // Protocol and nowhere else.
        let reg = registry();
        for v in reg.iter().filter(|v| {
            v.longname.starts_with("protocol-")
                || v.longname == "toggle-fold-family"
                || v.longname == "open-steps-sheet"
                || v.longname == "yank-address"
        }) {
            assert_eq!(
                v.scope_applicability,
                vec![Altitude::Protocol],
                "{} is protocol-only",
                v.longname
            );
            assert!(
                !v.applies_at(Altitude::View),
                "{} must not fire at View",
                v.longname
            );
            assert!(
                !v.applies_at(Altitude::Dashboard),
                "{} must not fire at Dashboard",
                v.longname
            );
        }
    }

    #[test]
    fn producers_take_only_the_registry() {
        // The three producers each derive purely from the registry.
        let reg = registry();
        let keys = keymap_bindings(&reg);
        let corpus = palette_corpus(&reg);
        let help = help_sheet(&reg);
        // Palette + help enumerate every verb (reserved included).
        assert_eq!(corpus.len(), reg.len());
        assert_eq!(help.len(), reg.len());
        // The keymap contains only bound verbs' keystrokes.
        let bound_count: usize = reg.iter().map(|v| v.binding_specs.len()).sum();
        assert_eq!(keys.len(), bound_count);
        // open-palette contributes its two-key twin (bare space + cmd-shift-p).
        let palette_keys: Vec<_> = keys
            .iter()
            .filter(|k| k.longname == "open-palette")
            .collect();
        assert_eq!(palette_keys.len(), 2);
    }

    #[test]
    fn cycle_colour_scheme_is_view_only_preview() {
        let reg = registry();
        let c = reg
            .iter()
            .find(|v| v.longname == "cycle-colour-scheme")
            .unwrap();
        assert_eq!(c.status, VerbStatus::Preview);
        assert_eq!(c.scope_applicability, vec![Altitude::View]);
        assert!(!c.applies_at(Altitude::Dashboard));
    }
}
