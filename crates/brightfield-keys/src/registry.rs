//! The command registry: every verb as data, the single source of truth
//! for the keymap-as-data vec, the palette corpus, and the help sheet.
//!
//! Framework-free by construction — no UI-framework type crosses this
//! boundary. A shell's keymap adapter turns a [`BindingSpec`] into its own
//! key-binding form and maps a `longname` to its action; nothing here knows
//! about the framework.

use crate::altitude::Altitude;

/// A framework-free keystroke descriptor. Carries NO framework types (the
/// standing framework-free rule): a shell adapter builds its own binding form
/// from this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingSpec {
    /// Space-separated keystrokes as plain data, e.g. `"p"`, `"cmd-e"`,
    /// `"g f"`.
    pub keystrokes: &'static str,
    /// The key-context this binding resolves in.
    pub context: BindingContext,
}

/// The key-context a binding resolves in. A shell adapter maps it to its own context
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
    /// Shelf-scoped: fires only while a chart's shelf holds focus. A distinct
    /// context, as the Protocol panel's is, so the shelf's `h`/`l`/`m`/`x`/`y`/`c`
    /// (the cell or value beside, and a jump to a channel's cell) never collide
    /// with the chart grammar's pop-out, dive-in, mark, axis-lock and colour
    /// bindings.
    Shelf,
    /// Grid-scoped: fires only while the table's grid holds focus. A distinct
    /// context, as the shelf's is, so the grid's `h`/`j`/`k`/`l` and arrows
    /// (the cell beside the cursor) never collide with the chart grammar's
    /// pop-out, dive-in and pan bindings.
    Grid,
    /// Versions-scoped: fires while the ledger's Versions panel holds focus. A
    /// distinct context, as the grid's is, so the panel's `j`/`k`, arrows,
    /// `Enter` and `Esc` (the cursor's row, the step back and the way back to
    /// now) are kept apart from the chart grammar's sibling-focus, dive-in and
    /// clear-selection bindings on the same keys
    /// (`the_versions_context_moves_the_cursor_steps_back_and_returns_to_now`).
    Versions,
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
    /// A structural spec edit that writes the durable protocol document:
    /// change-mark-type / add-mark / set-channel / remove-mark / undo. NOT
    /// `g`-broadcast eligible (that is runtime-dispatch only, `scope::g_eligible`).
    SpecEdit,
    /// A deferred verb, shown but unbound.
    Reserved,
}

/// The command tier — the durability taxonomy, REQUIRED on every verb and
/// enforced from the first commit because it cannot be retrofitted.
///
/// Every verb declares which tier it is, and the split is about **durability**:
/// whether the verb writes the durable protocol document on disk. A **View**
/// command (navigation, folds, panes, palette/help) changes only what you look
/// at — it never touches the protocol. A **Data** command acts on an addressed
/// asset and writes the durable protocol document: it appends or amends a step
/// through arc's own write path, so a session's Data commands leave a protocol,
/// not a screen trace. The durable-write barrier this draws is load-bearing:
/// undo is a Data command precisely because it rewrites the durable document,
/// so it must never be misclassified as a mere view change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTier {
    /// Changes only the view (nav / fold / pane / meta). Writes nothing durable.
    View,
    /// Acts on an addressed asset and writes the durable protocol document
    /// (append/amend a step through arc's write path).
    Data,
}

impl CommandTier {
    /// Whether commands at this tier write the durable protocol document — the
    /// durable-write barrier. `true` for [`CommandTier::Data`] and nothing
    /// else, so a View command can never be mistaken for one that writes.
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

/// Why a reserved verb is not yet available — the named buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservedReason {
    /// `m` / `a` / `e` / `d` / undo — need the durable spec-edit spine (the
    /// `ChartEdit` AST mutation API) to persist a structural change. Now an
    /// empty bucket: those verbs are built, and the variant is retained only for
    /// its `reason()` surface.
    NeedsCommandLog,
    /// `f` / `g f` / `t` / set-param — need a keyboard data-target: a way to
    /// name a predicate without a pointer-derived rectangle or point.
    NeedsKeyboardTarget,
    /// The pane toggles — need the one-app shell. A rail's show/hide verb is
    /// named by the item registry that declares the rail (every rail and tab
    /// must name one, because a pane a user can close and cannot reopen is a
    /// trap), but nothing can *perform* it until the workspace owns the window
    /// and its layout file. Declaring the verb here rather than inventing one
    /// at the call site is the point: the shell may not invent verbs, and a
    /// control with no registered command behind it is exactly how a palette
    /// and a keymap drift apart from what the UI offers.
    NeedsWorkspaceShell,
}

impl ReservedReason {
    /// A human-readable reason, surfaced in the palette flag and in a
    /// scope-resolver rejection.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            ReservedReason::NeedsCommandLog => "needs command log",
            ReservedReason::NeedsKeyboardTarget => "needs a keyboard target",
            ReservedReason::NeedsWorkspaceShell => "needs the workspace shell",
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
    /// The durability tier — REQUIRED on every verb. Governs whether the verb
    /// writes the durable protocol document, and, for `Data`, that it acts by
    /// dotted address (never a screen position). Enforced by [`registry`]'s
    /// construction, so a new verb cannot be added without a deliberate tier call.
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
    let shelf = |k: &'static str| BindingSpec {
        keystrokes: k,
        context: BindingContext::Shelf,
    };
    let grid = |k: &'static str| BindingSpec {
        keystrokes: k,
        context: BindingContext::Grid,
    };
    let versions = |k: &'static str| BindingSpec {
        keystrokes: k,
        context: BindingContext::Versions,
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
        // ---- navigating the frame: pan, zoom, axis lock, reset ----
        VerbEntry {
            longname: "pan-left",
            tier: CommandTier::View,
            binding_specs: vec![ws("left")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Pan the focused plot left",
            scores: Some(Scores { frequency: 4, mnemonic: 5, convention: 5, motor_note: "arrow keys = pan (every map and canvas); free of the hjkl focus grammar" }),
        },
        VerbEntry {
            longname: "pan-right",
            tier: CommandTier::View,
            binding_specs: vec![ws("right")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Pan the focused plot right",
            scores: Some(Scores { frequency: 4, mnemonic: 5, convention: 5, motor_note: "arrow keys = pan (every map and canvas); free of the hjkl focus grammar" }),
        },
        VerbEntry {
            longname: "pan-up",
            tier: CommandTier::View,
            binding_specs: vec![ws("up")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Pan the focused plot up",
            scores: Some(Scores { frequency: 4, mnemonic: 5, convention: 5, motor_note: "arrow keys = pan (every map and canvas); free of the hjkl focus grammar" }),
        },
        VerbEntry {
            longname: "pan-down",
            tier: CommandTier::View,
            binding_specs: vec![ws("down")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Pan the focused plot down",
            scores: Some(Scores { frequency: 4, mnemonic: 5, convention: 5, motor_note: "arrow keys = pan (every map and canvas); free of the hjkl focus grammar" }),
        },
        VerbEntry {
            longname: "zoom-in",
            tier: CommandTier::View,
            binding_specs: vec![ws("=")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Zoom the focused plot in",
            scores: Some(Scores { frequency: 4, mnemonic: 5, convention: 5, motor_note: "= is the unshifted + (browsers, maps, editors); no shift reach for the common direction" }),
        },
        VerbEntry {
            longname: "zoom-out",
            tier: CommandTier::View,
            binding_specs: vec![ws("-")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Zoom the focused plot out",
            scores: Some(Scores { frequency: 4, mnemonic: 5, convention: 5, motor_note: "- = zoom out, the universal twin of +; adjacent to = on every layout" }),
        },
        VerbEntry {
            longname: "cycle-axis-lock",
            tier: CommandTier::View,
            binding_specs: vec![ws("x")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Cycle which axes pan and zoom move",
            scores: Some(Scores { frequency: 2, mnemonic: 4, convention: 3, motor_note: "x = axis (mnemonic); a view-scoped mode toggle, free of the shipped chart grammar" }),
        },
        VerbEntry {
            longname: "reset-extent",
            tier: CommandTier::View,
            binding_specs: vec![ws("0")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Return every plot to its full extent",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 5, motor_note: "0 = reset zoom (browsers cmd-0, maps); bare here because the chart owns the digit row" }),
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
        VerbEntry {
            longname: "reload-data",
            tier: CommandTier::View,
            binding_specs: vec![global("cmd-shift-r")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Copy the open data file into the table again and recompose (picks up an edit on disk)",
            scores: Some(Scores { frequency: 1, mnemonic: 4, convention: 5, motor_note: "cmd-shift-r = hard reload (browser convention); distinct chord from cmd-r's spec reload, since this re-reads the DATA file rather than the YAML" }),
        },
        VerbEntry {
            longname: "open-home",
            tier: CommandTier::View,
            binding_specs: vec![global("cmd-shift-h")],
            scope_applicability: vec![Dashboard, View, Protocol],
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Return to the front door (keeps your place under Continue)",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 4, motor_note: "cmd-shift-h = home; free of the editor chord set" }),
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
            help: "Save the Protocol this data file opened as",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 5, motor_note: "cmd-s = save (universal; shipped, editor-scoped)" }),
        },
        // The run: the verb behind the ledger strip's Run control. It writes
        // the spec as save-spec does and asks `arc` to run it, and what it
        // leaves is `arc`'s run record rather than a change to the document —
        // so View, beside save-spec, not Data. Global, so it reaches from the
        // canvas as well as from the strip.
        VerbEntry {
            longname: "run-protocol",
            tier: CommandTier::View,
            binding_specs: vec![global("cmd-enter")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Run the Protocol this data file opened as",
            scores: Some(Scores { frequency: 3, mnemonic: 3, convention: 5, motor_note: "cmd-enter = run (notebook cells, SQL consoles); cmd-r is taken by reload-spec" }),
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
        // ---- durable structural edits: m/a/e/d at View, undo at
        //      Dashboard+View. Flipped Reserved -> Built — the ChartEdit spine
        //      + Session::reload_spec seam now back them. All Data-tier: each
        //      writes the durable protocol document. ----
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
            help: "Go to the focused view's shelf and put a column on a channel, applied live",
            scores: Some(Scores { frequency: 3, mnemonic: 3, convention: 3, motor_note: "e = encode/edit-channel; opens the shelf, whose cells pick the channel and whose list picks the column" }),
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
            // Data-tier: undo rewrites the durable protocol document (it pops the
            // spec-edit stack back over durable writes), so it sits on the
            // durable side of the barrier. Tagging it View would let it be
            // mistaken for a mere view change and cross a durable-write barrier —
            // exactly the misclassification the tier taxonomy exists to prevent.
            longname: "undo",
            tier: CommandTier::Data,
            binding_specs: vec![ws("u"), shelf("u"), shelf("cmd-z")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Undo the last uncommitted edit (cannot cross a commit)",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 5, motor_note: "u = undo (vim); snapshot-stack pop, stops at a commit barrier; in the shelf u works from any state and cmd-z from the query, where a letter is typed" }),
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
            longname: "toggle-fold",
            tier: CommandTier::View,
            binding_specs: vec![proto("z a")],
            scope_applicability: vec![Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Open/close the detail under the cursor: a parameterised family's members, the CTEs inside a sql: step, or a run of single hand-offs folded to the asset it ends at",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 5, motor_note: "za = toggle fold (vim fold family); one verb over both folds, resolved by what the cursor is on; a fold is a view change, never logged" }),
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
        // ---- the Outline's column rows: put the column under the cursor on a
        //      channel. `z` opens a chord beside `z a`, and the rows answer in the
        //      Protocol context. Each acts on a column's row and does nothing on a
        //      spine row; a column's row has nothing to fold, so `z a` is left
        //      as it is. Data-tier: each sets a channel, as `set-channel` does. ----
        VerbEntry {
            longname: "put-column-on-x",
            tier: CommandTier::Data,
            binding_specs: vec![proto("z x")],
            scope_applicability: vec![Protocol],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Put the Outline's column under the cursor on the x channel of the focused view's shelf",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 3, motor_note: "z x = column to x; the z chord sits beside z a, and the channel's own letter is the second key; acts on a column's row and does nothing on a spine row" }),
        },
        VerbEntry {
            longname: "put-column-on-y",
            tier: CommandTier::Data,
            binding_specs: vec![proto("z y")],
            scope_applicability: vec![Protocol],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Put the Outline's column under the cursor on the y channel of the focused view's shelf",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 3, motor_note: "z y = column to y; the z chord sits beside z a, and the channel's own letter is the second key; acts on a column's row and does nothing on a spine row" }),
        },
        VerbEntry {
            longname: "put-column-on-colour",
            tier: CommandTier::Data,
            binding_specs: vec![proto("z c")],
            scope_applicability: vec![Protocol],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Put the Outline's column under the cursor on the colour channel of the focused view's shelf",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 3, motor_note: "z c = column to colour; the z chord sits beside z a, and vim's zc, which closes a fold, has nothing to close on a column's row; u takes it back" }),
        },
        // ---- the shelf: a band of one cell per channel at the head of a chart's
        //      tile, and an open list of columns or settings under a cell. An open
        //      list takes letters as verbs and `/` gives the query the keys. These
        //      resolve in the Shelf context, apart from the Workspace's and the
        //      Protocol panel's bindings on the same keys. Motion and back-out are
        //      View-tier; keeping a choice sets a channel, so it is Data-tier. ----
        VerbEntry {
            longname: "go-to-mark-cell",
            tier: CommandTier::View,
            binding_specs: vec![shelf("m")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Go to the shelf's mark cell, keeping the list's tab",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 3, motor_note: "m = mark, printed on the cell; the Shelf context keeps it apart from the Workspace's change-mark-type" }),
        },
        VerbEntry {
            longname: "go-to-x-cell",
            tier: CommandTier::View,
            binding_specs: vec![shelf("x")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Go to the shelf's x cell, keeping the list's tab",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 3, motor_note: "x = the x channel, printed on the cell; the Shelf context keeps it apart from the Workspace's cycle-axis-lock" }),
        },
        VerbEntry {
            longname: "go-to-y-cell",
            tier: CommandTier::View,
            binding_specs: vec![shelf("y")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Go to the shelf's y cell, keeping the list's tab",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 3, motor_note: "y = the y channel, printed on the cell; the Shelf context keeps it apart from the Protocol panel's yank-address" }),
        },
        VerbEntry {
            longname: "go-to-colour-cell",
            tier: CommandTier::View,
            binding_specs: vec![shelf("c")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Go to the shelf's colour cell, keeping the list's tab",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 3, motor_note: "c = colour, printed on the cell; the Shelf context keeps it apart from the Workspace's cycle-colour-scheme" }),
        },
        VerbEntry {
            longname: "move-shelf-next-row",
            tier: CommandTier::View,
            binding_specs: vec![shelf("j"), shelf("down")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move to the next row of the shelf's open list",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row j = down/next (vim, lazygit); the arrow is its twin, and both agree with the Workspace's j and the Protocol panel's j" }),
        },
        VerbEntry {
            longname: "move-shelf-prev-row",
            tier: CommandTier::View,
            binding_specs: vec![shelf("k"), shelf("up")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move to the previous row of the shelf's open list",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row k = up/prev (vim, lazygit); the arrow is its twin, and both agree with the Workspace's k and the Protocol panel's k" }),
        },
        VerbEntry {
            longname: "move-shelf-left",
            tier: CommandTier::View,
            binding_specs: vec![shelf("h"), shelf("left")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "On the band and on the columns, the cell to the left, stopping at the mark's cell; on a settings row, step its value back and draw the chart at once: scale from symlog to log to linear, format from custom back through its presets to auto, colour's scheme from meridian back through turbo and blues to viridis, a switch the other way, a row that does not apply left as it is",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row h = left (vim), as drawn: mark, x, y, colour run left to right; stops at the mark rather than popping out, because Esc is the way out" }),
        },
        VerbEntry {
            longname: "move-shelf-right",
            tier: CommandTier::View,
            binding_specs: vec![shelf("l"), shelf("right")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "On the band and on the columns, the cell to the right, stopping at colour; on a settings row, step its value forward and draw the chart at once: scale from linear to log to symlog, format from auto through its presets to custom, which opens a field, colour's scheme from viridis through blues and turbo to meridian, a switch the other way, a row that does not apply left as it is",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row l = right (vim), as drawn: mark, x, y, colour run left to right; the Protocol panel's l is likewise the node drawn to the right" }),
        },
        VerbEntry {
            longname: "set-shelf-setting-to-auto",
            tier: CommandTier::Data,
            binding_specs: vec![shelf("backspace")],
            scope_applicability: vec![View],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Put the settings row under the cursor back to auto: its key comes out of the file and the chart draws brightfield's own",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 4, motor_note: "backspace = take the value back out (the key that deletes in every text field); the query takes it as an edit while it has the keys, so the row answers only when the rows hold them" }),
        },
        VerbEntry {
            longname: "narrow-shelf-list",
            tier: CommandTier::View,
            binding_specs: vec![shelf("/")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Narrow the shelf's open list to the rows matching what is typed; Esc clears",
            scores: Some(Scores { frequency: 4, mnemonic: 4, convention: 5, motor_note: "/ = search/narrow (vim, less, lazygit); the query takes letters as text until Esc, and the Workspace's / is focus-jump, a search by name" }),
        },
        VerbEntry {
            longname: "turn-shelf-list",
            tier: CommandTier::View,
            binding_specs: vec![shelf("tab")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Turn the open list of x, y or colour between its columns and its settings; on the mark's list it stays on the columns",
            scores: Some(Scores { frequency: 4, mnemonic: 4, convention: 4, motor_note: "Tab = the next tab of the open list (browser tabs, IDE panes, lazygit's panels); the Shelf context keeps it apart from the Workspace's focus-next-sibling, and the list's tab strip prints the word it turns to" }),
        },
        VerbEntry {
            longname: "keep-shelf-choice",
            tier: CommandTier::Data,
            binding_specs: vec![shelf("enter")],
            scope_applicability: vec![View],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Keep the row under the cursor: the previewed column goes on the channel, or the value is set",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "enter = keep the row under the cursor, in the list and in the query (telescope, fzf); a Data verb, since it sets a channel" }),
        },
        VerbEntry {
            longname: "back-out-of-shelf",
            tier: CommandTier::View,
            binding_specs: vec![shelf("escape")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Back out one level: a value not kept, then the query, then the list, then the shelf",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "esc = back out one level (the Esc ladder); the same steps down as clear-selection and protocol-drill-out" }),
        },
        // ---- the grid's cursor: one cell of the table, so a row and a column at
        //      once, moved a cell at a time. These resolve in the Grid context,
        //      apart from the Workspace's dive-in, pop-out and pan bindings on
        //      the same keys. View-tier: the cursor is where the analyst is
        //      looking, and moving it changes no data. ----
        VerbEntry {
            longname: "move-cursor-down",
            tier: CommandTier::View,
            binding_specs: vec![grid("j"), grid("down")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move the grid's cursor to the cell below; stops at the table's last row",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row j = down (vim, VisiData); the arrow is its twin, as in every spreadsheet; agrees with the shelf's and the Protocol panel's j" }),
        },
        VerbEntry {
            longname: "move-cursor-up",
            tier: CommandTier::View,
            binding_specs: vec![grid("k"), grid("up")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move the grid's cursor to the cell above; stops at the table's first row",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row k = up (vim, VisiData); the arrow is its twin, as in every spreadsheet; agrees with the shelf's and the Protocol panel's k" }),
        },
        VerbEntry {
            longname: "move-cursor-left",
            tier: CommandTier::View,
            binding_specs: vec![grid("h"), grid("left")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move the grid's cursor to the cell on the left; stops at the first column",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row h = left (vim, VisiData); the arrow is its twin; the Grid context keeps it apart from the Workspace's pop-out and pan-left" }),
        },
        VerbEntry {
            longname: "move-cursor-right",
            tier: CommandTier::View,
            binding_specs: vec![grid("l"), grid("right")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move the grid's cursor to the cell on the right; stops at the last column",
            scores: Some(Scores { frequency: 5, mnemonic: 4, convention: 5, motor_note: "home-row l = right (vim, VisiData); the arrow is its twin; the Grid context keeps it apart from the Workspace's dive-in and pan-right" }),
        },
        // ---- the Versions panel's cursor: one row of the ledger's list of a
        //      chart's saved versions. The chart is drawn as the version under
        //      the cursor; Enter steps the chart back to it as an unsaved edit,
        //      and Esc draws the chart as it was. These resolve in the Versions
        //      context, apart from the Workspace's sibling focus, dive-in and
        //      clear-selection on the same keys. ----
        VerbEntry {
            longname: "move-version-cursor-down",
            tier: CommandTier::View,
            binding_specs: vec![versions("j"), versions("down")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Draw the chart as the next older version in the Versions panel; stops at the oldest",
            scores: Some(Scores { frequency: 4, mnemonic: 4, convention: 5, motor_note: "home-row j = down (vim, lazygit's commit list); the arrow is its twin; agrees with the grid's, the shelf's and the Protocol panel's j" }),
        },
        VerbEntry {
            longname: "move-version-cursor-up",
            tier: CommandTier::View,
            binding_specs: vec![versions("k"), versions("up")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Draw the chart as the next newer version in the Versions panel; stops at the newest",
            scores: Some(Scores { frequency: 4, mnemonic: 4, convention: 5, motor_note: "home-row k = up (vim, lazygit's commit list); the arrow is its twin; agrees with the grid's, the shelf's and the Protocol panel's k" }),
        },
        VerbEntry {
            longname: "step-back-to-version",
            tier: CommandTier::Data,
            binding_specs: vec![versions("enter")],
            scope_applicability: vec![View],
            drives: D::SpecEdit,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Step the chart back to the version under the cursor, as an unsaved edit that Save writes",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 5, motor_note: "enter = take the row under the cursor (the shelf's keep, telescope, fzf); a Data verb, since Save writes it; u takes it back" }),
        },
        VerbEntry {
            longname: "return-to-now",
            tier: CommandTier::View,
            binding_specs: vec![versions("escape")],
            scope_applicability: vec![View],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Draw the chart as it was before the Versions panel's cursor moved",
            scores: Some(Scores { frequency: 4, mnemonic: 4, convention: 5, motor_note: "esc = back out one level (the Esc ladder), as the shelf's back-out-of-shelf; writes nothing" }),
        },
        // ---- pane toggles: the show/hide verbs the item registries name.
        //      Reserved rather than bound: the pane toggles cannot be performed
        //      until the workspace shell owns the window and its layout, and a
        //      keystroke that silently does nothing is worse than no keystroke.
        //      They are here because a rail must NAME its toggle (a pane the
        //      user can close and cannot reopen is a trap) and the shell may not
        //      invent a verb to name it with. View-tier: showing a rail changes
        //      what you look at, never the data, so it is never logged. They
        //      take the general `reserved` bucket's shape but not its helper,
        //      which builds Data-tier entries. ----
        // The navigator rail's toggle, and the one pane toggle that is BOUND.
        // The protocol is the container the work sits inside rather than a
        // peer surface, so reaching it is a dock focus toggle with a round
        // trip — press once for the spine, press again for the work — and it
        // is a mnemonic binding rather than a positional numeric because the
        // surfaces it addresses are not a bounded set of peers.
        VerbEntry {
            longname: "toggle-outline-rail",
            tier: CommandTier::View,
            binding_specs: vec![global("cmd-b")],
            scope_applicability: vec![Dashboard, View, Protocol],
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move focus to the protocol spine, or back where it came from",
            scores: Some(Scores { frequency: 4, mnemonic: 2, convention: 5, motor_note: "cmd-b = the left dock (Zed workspace::ToggleLeftDock, VS Code toggleSidebarVisibility); round-trip focus, never a numeric" }),
        },
        VerbEntry {
            longname: "toggle-inspector-rail",
            tier: CommandTier::View,
            binding_specs: Vec::new(),
            scope_applicability: vec![Protocol],
            drives: D::Reserved,
            status: VerbStatus::Reserved,
            reserved_reason: Some(ReservedReason::NeedsWorkspaceShell),
            help: "Show or hide the protocol inspector rail",
            scores: None,
        },
        // The chart view's rail, on the same terms. It sits at Dashboard and
        // View rather than Protocol: the controls rail belongs to the chart
        // grammar, and a rail toggle that fired inside the protocol panel would
        // be a verb reaching into a view that does not have the pane.
        VerbEntry {
            longname: "toggle-controls-rail",
            tier: CommandTier::View,
            binding_specs: Vec::new(),
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Reserved,
            status: VerbStatus::Reserved,
            reserved_reason: Some(ReservedReason::NeedsWorkspaceShell),
            help: "Show or hide the chart controls rail",
            scores: None,
        },
        // The ledger rail's Versions panel: a centre tab in the chart view's item
        // registry, so it carries a show verb the item audit requires, reserved
        // and unbound as the Log's and the Quality panel's are; the panel's own
        // keys are the Versions context's, below the grid's. It sits at Protocol, where the other ledger panes' verbs do, because the
        // chart file it lists is the Protocol's.
        VerbEntry {
            longname: "open-chart-versions",
            tier: CommandTier::View,
            binding_specs: Vec::new(),
            scope_applicability: vec![Protocol],
            drives: D::Reserved,
            status: VerbStatus::Reserved,
            reserved_reason: Some(ReservedReason::NeedsWorkspaceShell),
            help: "Open the chart's versions — what the store keeps of it, in the ledger rail",
            scores: None,
        },
        // The ledger rail's two run panes. Each is a centre tab in its view's
        // item registry, which requires a show/hide verb the shell may not
        // invent (the item-registry audit enforces it), and each is unbound
        // until the workspace shell performs pane toggles — the rail's own
        // strip is how a reader reaches them today. They sit at Protocol
        // because they report a run of the Protocol.
        VerbEntry {
            longname: "open-run-log",
            tier: CommandTier::View,
            binding_specs: Vec::new(),
            scope_applicability: vec![Protocol],
            drives: D::Reserved,
            status: VerbStatus::Reserved,
            reserved_reason: Some(ReservedReason::NeedsWorkspaceShell),
            help: "Open the run log — the last run's log, in the ledger rail",
            scores: None,
        },
        VerbEntry {
            longname: "open-run-quality",
            tier: CommandTier::View,
            binding_specs: Vec::new(),
            scope_applicability: vec![Protocol],
            drives: D::Reserved,
            status: VerbStatus::Reserved,
            reserved_reason: Some(ReservedReason::NeedsWorkspaceShell),
            help: "Open the run quality output, per step, in the ledger rail",
            scores: None,
        },
        // The table's one grid, moved between its two spots: beside the hero on
        // the canvas, and the ledger rail's Rows spot. One verb for both
        // directions, because it is one grid and one fact about where it is —
        // not a pane per spot, each with its own show/hide. Global, so it
        // reaches the grid from whichever spot holds it, and from the rest of
        // the window, and it is the toggle both spots' item specs name.
        VerbEntry {
            longname: "move-grid",
            tier: CommandTier::View,
            binding_specs: vec![global("cmd-j")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Move the grid between the canvas and the ledger",
            scores: Some(Scores { frequency: 3, mnemonic: 2, convention: 4, motor_note: "cmd-j = the bottom dock (Zed workspace::ToggleBottomDock, VS Code togglePanel); here it moves the grid into the bottom rail and back" }),
        },
        // The dev-flagged design gallery tab, on the same terms as the pane
        // toggles above: a centre tab must name its show/hide verb (the
        // item-registry audit enforces it) and the shell may not invent one.
        // Unbound until the workspace shell performs pane toggles; the tab
        // itself stays reachable by pointer when the gallery flag is set.
        VerbEntry {
            longname: "toggle-gallery",
            tier: CommandTier::View,
            binding_specs: Vec::new(),
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Reserved,
            status: VerbStatus::Reserved,
            reserved_reason: Some(ReservedReason::NeedsWorkspaceShell),
            help: "Show or hide the design gallery — the component vocabulary, drawn live",
            scores: None,
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

/// One bound key in the keymap-as-data vec: the projection a shell's keymap
/// adapter consumes to build its own bindings, and the input to the
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

/// The keymap-as-data vec: the SINGLE binding source. A shell adapter maps
/// `longname` → action and `context` → predicate to build its bindings.
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
        // The pane toggles: named by each view's item registry, which requires
        // every rail to name the verb that shows and hides it, and unbound
        // until the workspace shell can perform one.
        let mut needs_shell: Vec<&str> = reg
            .iter()
            .filter(|v| v.reserved_reason == Some(ReservedReason::NeedsWorkspaceShell))
            .map(|v| v.longname)
            .collect();
        needs_shell.sort_unstable();
        // `toggle-outline-rail` left this bucket when the navigator rail
        // landed: it is bound, performed and scored. The rest stay reserved.
        assert_eq!(
            needs_shell,
            [
                "open-chart-versions",
                "open-run-log",
                "open-run-quality",
                "toggle-controls-rail",
                "toggle-gallery",
                "toggle-inspector-rail"
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
            "pan-left",
            "pan-right",
            "pan-up",
            "pan-down",
            "zoom-in",
            "zoom-out",
            "cycle-axis-lock",
            "reset-extent",
            "reload-spec",
            "reload-data",
            "open-home",
            "toggle-presentation",
            "save-spec",
            "run-protocol",
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
            "toggle-fold",
            "protocol-drill-in",
            "protocol-drill-out",
            "open-steps-sheet",
            "yank-address",
            "put-column-on-x",
            "put-column-on-y",
            "put-column-on-colour",
            "go-to-mark-cell",
            "go-to-x-cell",
            "go-to-y-cell",
            "go-to-colour-cell",
            "move-shelf-next-row",
            "move-shelf-prev-row",
            "move-shelf-left",
            "move-shelf-right",
            "set-shelf-setting-to-auto",
            "narrow-shelf-list",
            "turn-shelf-list",
            "keep-shelf-choice",
            "back-out-of-shelf",
            "move-cursor-down",
            "move-cursor-up",
            "move-cursor-left",
            "move-cursor-right",
            "move-version-cursor-down",
            "move-version-cursor-up",
            "step-back-to-version",
            "return-to-now",
            "toggle-outline-rail",
            "toggle-inspector-rail",
            "toggle-controls-rail",
            "open-chart-versions",
            "open-run-log",
            "open-run-quality",
            "move-grid",
            "toggle-gallery",
        ];
        assert_eq!(got, expected);
    }

    #[test]
    fn every_verb_declares_a_tier_and_only_data_writes_durably() {
        // The durability taxonomy's REQUIRED field: every verb carries a tier,
        // and exactly the Data tier writes the durable protocol document.
        // Navigation/fold/pane/meta verbs are View; the spec-edit + object verbs
        // that name a dotted address (and undo, which rewrites the document) are
        // Data.
        let reg = registry();
        let durable: Vec<&str> = reg
            .iter()
            .filter(|v| v.tier.is_logged())
            .map(|v| v.longname)
            .collect();
        // The Data-tier set is exactly the addressed spec-edit verbs plus undo,
        // the Outline's put-a-column-on-a-channel verbs, the shelf's keep, and the
        // Versions panel's step back, which Save writes.
        let mut got = durable.clone();
        got.sort_unstable();
        let mut expected = vec![
            "change-mark-type",
            "add-mark",
            "set-channel",
            "remove-mark",
            "undo",
            "filter-view",
            "cross-filter-all",
            "toggle-point-select",
            "set-param",
            "yank-address",
            "put-column-on-x",
            "put-column-on-y",
            "put-column-on-colour",
            "keep-shelf-choice",
            "set-shelf-setting-to-auto",
            "step-back-to-version",
        ];
        expected.sort_unstable();
        assert_eq!(got, expected, "only durable-writing (Data) verbs write");
        // Every protocol MOTION verb is View-tier (a fold/nav is never logged).
        for name in [
            "protocol-producer",
            "protocol-consumer",
            "protocol-sibling-next",
            "protocol-sibling-prev",
            "toggle-fold",
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
    fn undo_is_a_durable_write_never_a_view_change() {
        // The undo-barrier invariant. Undo carries a durable spec edit (it
        // rewrites the protocol document by popping the edit stack), so it must
        // sit on the durable side of the barrier — Data tier — and never be
        // tagged View. A View-tagged undo could be treated as a mere view change
        // and slip across a durable-write barrier, which is the bug this guards.
        let reg = registry();
        let undo = reg
            .iter()
            .find(|v| v.longname == "undo")
            .expect("undo is registered");
        assert_eq!(
            undo.drives,
            Drives::SpecEdit,
            "undo drives a structural spec edit"
        );
        assert_eq!(
            undo.tier,
            CommandTier::Data,
            "undo writes durably, so it is Data-tier, never View"
        );
        assert!(
            undo.tier.is_logged(),
            "a durable-write verb reports it writes the durable document"
        );
        // Cross-check the whole taxonomy: no verb that drives a structural spec
        // edit may be tagged View — a durable write is never a view change.
        for v in reg.iter().filter(|v| v.drives == Drives::SpecEdit) {
            assert_eq!(
                v.tier,
                CommandTier::Data,
                "{} drives a spec edit, so it must be Data-tier (durable)",
                v.longname
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
                || v.longname == "toggle-fold"
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
    fn the_shelf_context_carries_a_verb_for_each_of_the_shelfs_keys() {
        let reg = registry();
        let bound = keymap_bindings(&reg);
        let in_shelf = |keys: &str| -> Vec<&'static str> {
            bound
                .iter()
                .filter(|b| b.context == BindingContext::Shelf && b.keystrokes == keys)
                .map(|b| b.longname)
                .collect()
        };
        let expected = [
            // The mark, x, y and colour cells.
            ("m", "go-to-mark-cell"),
            ("x", "go-to-x-cell"),
            ("y", "go-to-y-cell"),
            ("c", "go-to-colour-cell"),
            // The next and previous row.
            ("j", "move-shelf-next-row"),
            ("down", "move-shelf-next-row"),
            ("k", "move-shelf-prev-row"),
            ("up", "move-shelf-prev-row"),
            // The cell or value beside.
            ("h", "move-shelf-left"),
            ("left", "move-shelf-left"),
            ("l", "move-shelf-right"),
            ("right", "move-shelf-right"),
            // A settings row put back to auto.
            ("backspace", "set-shelf-setting-to-auto"),
            // Narrowing, keeping, and backing out one level.
            ("/", "narrow-shelf-list"),
            // The list turned: columns, settings.
            ("tab", "turn-shelf-list"),
            ("enter", "keep-shelf-choice"),
            ("escape", "back-out-of-shelf"),
        ];
        for (keys, longname) in expected {
            assert_eq!(in_shelf(keys), vec![longname], "Shelf context, `{keys}`");
            let verb = reg.iter().find(|v| v.longname == longname).unwrap();
            assert!(!verb.help.is_empty(), "{longname} has no help line");
            let scores = verb
                .scores
                .as_ref()
                .unwrap_or_else(|| panic!("{longname} has no scores"));
            for score in [scores.frequency, scores.mnemonic, scores.convention] {
                assert!((1..=5).contains(&score), "{longname} score {score}");
            }
            assert!(
                !scores.motor_note.is_empty(),
                "{longname} has no motor note"
            );
        }
        // The context holds these bindings and `undo`'s two; a stray binding
        // would be a key the help sheet lists that nothing here accounts for.
        let in_shelf_count = bound
            .iter()
            .filter(|b| b.context == BindingContext::Shelf)
            .count();
        assert_eq!(in_shelf_count, expected.len() + 2, "Shelf bindings");
    }

    #[test]
    fn the_grid_context_moves_the_cursor_a_cell_on_each_of_its_keys() {
        let reg = registry();
        let bound = keymap_bindings(&reg);
        let in_grid = |keys: &str| -> Vec<&'static str> {
            bound
                .iter()
                .filter(|b| b.context == BindingContext::Grid && b.keystrokes == keys)
                .map(|b| b.longname)
                .collect()
        };
        let expected = [
            ("j", "move-cursor-down"),
            ("down", "move-cursor-down"),
            ("k", "move-cursor-up"),
            ("up", "move-cursor-up"),
            ("h", "move-cursor-left"),
            ("left", "move-cursor-left"),
            ("l", "move-cursor-right"),
            ("right", "move-cursor-right"),
        ];
        for (keys, longname) in expected {
            assert_eq!(in_grid(keys), vec![longname], "Grid context, `{keys}`");
            let verb = reg.iter().find(|v| v.longname == longname).unwrap();
            assert_eq!(verb.tier, CommandTier::View, "{longname} writes nothing");
            let scores = verb
                .scores
                .as_ref()
                .unwrap_or_else(|| panic!("{longname} has no scores"));
            for score in [scores.frequency, scores.mnemonic, scores.convention] {
                assert!((1..=5).contains(&score), "{longname} score {score}");
            }
        }
        // Exactly these: a stray binding in the context would be a key the
        // grid answers that nothing here accounts for.
        let in_grid_count = bound
            .iter()
            .filter(|b| b.context == BindingContext::Grid)
            .count();
        assert_eq!(in_grid_count, expected.len(), "Grid bindings");
    }

    #[test]
    fn the_versions_context_moves_the_cursor_steps_back_and_returns_to_now() {
        let reg = registry();
        let bound = keymap_bindings(&reg);
        let in_versions = |keys: &str| -> Vec<&'static str> {
            bound
                .iter()
                .filter(|b| b.context == BindingContext::Versions && b.keystrokes == keys)
                .map(|b| b.longname)
                .collect()
        };
        let expected = [
            ("j", "move-version-cursor-down"),
            ("down", "move-version-cursor-down"),
            ("k", "move-version-cursor-up"),
            ("up", "move-version-cursor-up"),
            ("enter", "step-back-to-version"),
            ("escape", "return-to-now"),
        ];
        for (keys, longname) in expected {
            assert_eq!(
                in_versions(keys),
                vec![longname],
                "Versions context, `{keys}`"
            );
            let verb = reg.iter().find(|v| v.longname == longname).unwrap();
            assert!(!verb.help.is_empty(), "{longname} has no help line");
            let scores = verb
                .scores
                .as_ref()
                .unwrap_or_else(|| panic!("{longname} has no scores"));
            for score in [scores.frequency, scores.mnemonic, scores.convention] {
                assert!((1..=5).contains(&score), "{longname} score {score}");
            }
        }
        // The step back is written by Save, so it is a Data verb; the cursor and
        // the way back to now write nothing.
        for (longname, tier) in [
            ("move-version-cursor-down", CommandTier::View),
            ("move-version-cursor-up", CommandTier::View),
            ("step-back-to-version", CommandTier::Data),
            ("return-to-now", CommandTier::View),
        ] {
            let verb = reg.iter().find(|v| v.longname == longname).unwrap();
            assert_eq!(verb.tier, tier, "{longname}'s tier");
        }
        let in_versions_count = bound
            .iter()
            .filter(|b| b.context == BindingContext::Versions)
            .count();
        assert_eq!(in_versions_count, expected.len(), "Versions bindings");
    }

    #[test]
    fn set_channels_help_line_names_the_shelf_not_an_argument_overlay() {
        let reg = registry();
        let help = reg
            .iter()
            .find(|v| v.longname == "set-channel")
            .unwrap()
            .help;
        assert!(help.contains("shelf"), "set-channel help: {help}");
        assert!(
            !help.contains("overlay") && !help.contains("prompts"),
            "set-channel help still names an overlay: {help}"
        );
    }

    #[test]
    fn no_two_verbs_in_one_context_share_a_keystroke_sequence() {
        let bound = keymap_bindings(&registry());
        // `BindingContext` is not `Hash`, so this is a plain pairwise scan.
        for (i, a) in bound.iter().enumerate() {
            for b in &bound[i + 1..] {
                assert!(
                    !(a.context == b.context && a.keystrokes == b.keystrokes),
                    "{} and {} both bind `{}` in {:?}",
                    a.longname,
                    b.longname,
                    a.keystrokes,
                    a.context
                );
            }
        }
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
