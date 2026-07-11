//! The command registry (ac-01): every verb as data, the single source of truth
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
    /// Global (`context = None`): fires from any focus (palette twin, focus
    /// toggle, save/reload-from-anywhere).
    Global,
}

/// Recorded provenance for a bound key (ac-08): the scores that DEFEND the key
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
    /// A deferred verb, shown but unbound.
    Reserved,
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
    /// set governs bare-key resolution and palette candidacy (ac-04).
    pub scope_applicability: Vec<Altitude>,
    /// What the verb drives.
    pub drives: Drives,
    /// Lifecycle status.
    pub status: VerbStatus,
    /// For reserved verbs: which bucket blocks it. `None` for active verbs.
    pub reserved_reason: Option<ReservedReason>,
    /// One-line description; part of the palette fuzzy corpus alongside the longname.
    pub help: &'static str,
    /// Provenance for a bound key (ac-08). Present for every bound key; `None` for reserved.
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
    use Altitude::{Dashboard, View};
    use BindingContext::{Editor, Global, Workspace};
    use Drives as D;

    let ws = |k: &'static str| BindingSpec { keystrokes: k, context: Workspace };
    let global = |k: &'static str| BindingSpec { keystrokes: k, context: Global };
    let editor = |k: &'static str| BindingSpec { keystrokes: k, context: Editor };

    vec![
        // ---- navigation (ac-10) ----
        VerbEntry {
            longname: "dive-in",
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
            binding_specs: vec![ws("/")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Navigation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Fuzzy-jump focus to a component by name",
            scores: Some(Scores { frequency: 3, mnemonic: 4, convention: 5, motor_note: "/ = search/jump (vim, less)" }),
        },
        // ---- palette + help (ac-12, ac-19) ----
        VerbEntry {
            longname: "open-palette",
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
            binding_specs: vec![ws("?")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::PaletteMeta,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Show the keyboard help sheet",
            scores: Some(Scores { frequency: 2, mnemonic: 4, convention: 5, motor_note: "? = help (near-universal convention)" }),
        },
        // ---- runtime verbs (ac-11) ----
        VerbEntry {
            longname: "clear-selection",
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
            binding_specs: vec![global("cmd-r")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Reload the spec from disk (guards unsaved editor edits)",
            scores: Some(Scores { frequency: 2, mnemonic: 4, convention: 4, motor_note: "cmd-r = reload (browser); bare r NOT bound (dirty-guard)" }),
        },
        // ---- presentation (ac-16) + save: shipped fixed points, sourced here so
        //      the registry is the single binding source ----
        VerbEntry {
            longname: "toggle-presentation",
            binding_specs: vec![ws("p")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::Presentation,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Toggle presentation mode (hide authoring chrome)",
            scores: Some(Scores { frequency: 2, mnemonic: 3, convention: 3, motor_note: "p = present (shipped fixed point, card 0016)" }),
        },
        VerbEntry {
            longname: "save-spec",
            binding_specs: vec![editor("cmd-s")],
            scope_applicability: DASHBOARD_AND_VIEW.to_vec(),
            drives: D::RuntimeDispatch,
            status: VerbStatus::Built,
            reserved_reason: None,
            help: "Save the spec (editor)",
            scores: Some(Scores { frequency: 3, mnemonic: 5, convention: 5, motor_note: "cmd-s = save (universal; shipped, editor-scoped)" }),
        },
        // ---- colour preview (ac-13): transient, view-scoped ----
        VerbEntry {
            longname: "cycle-colour-scheme",
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
        // ---- reserved: needs the command log (m / a / e / d / undo) ----
        reserved("change-mark-type", vec![View], ReservedReason::NeedsCommandLog, "Change a mark's type (needs command log)"),
        reserved("add-mark", vec![View], ReservedReason::NeedsCommandLog, "Add a mark to the focused view (needs command log)"),
        reserved("set-channel", vec![View], ReservedReason::NeedsCommandLog, "Bind a channel to a column (needs command log)"),
        reserved("remove-mark", vec![View], ReservedReason::NeedsCommandLog, "Remove a mark (needs command log)"),
        reserved("undo", DASHBOARD_AND_VIEW.to_vec(), ReservedReason::NeedsCommandLog, "Undo the last edit (needs command log)"),
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
// Producers (ac-01): each takes the registry as its sole verb-metadata input
// ---------------------------------------------------------------------------

/// One bound key in the keymap-as-data vec: the projection the GPUI adapter
/// consumes to build a `gpui::KeyBinding`, and the input to the
/// dispatch-resolution table (ac-07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundKey {
    /// The verb this key runs.
    pub longname: &'static str,
    /// Space-separated keystrokes.
    pub keystrokes: &'static str,
    /// The context this binding resolves in.
    pub context: BindingContext,
}

/// The keymap-as-data vec (ac-01): the SINGLE binding source. The adapter maps
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

/// One palette row derived from the registry (ac-01 / ac-05).
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

/// The full palette corpus (ac-01): one row per verb, reserved included. The
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

/// One row of the help sheet (ac-01 / ac-19), grouped by scope in the overlay.
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

/// The help sheet (ac-01): every verb with its keys, help, scope, and (if
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
            && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !s.starts_with('-')
            && !s.ends_with('-')
    }

    #[test]
    fn kbg_ac01_longnames_unique_and_kebab_case() {
        let reg = registry();
        let mut seen = std::collections::HashSet::new();
        for v in &reg {
            assert!(is_kebab_case(v.longname), "not kebab-case: {}", v.longname);
            assert!(seen.insert(v.longname), "duplicate longname: {}", v.longname);
        }
    }

    #[test]
    fn kbg_ac01_reserved_buckets_present_with_reasons() {
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
        assert_eq!(needs_log, ["add-mark", "change-mark-type", "remove-mark", "set-channel", "undo"]);
        assert_eq!(needs_target, ["cross-filter-all", "filter-view", "set-param", "toggle-point-select"]);
        // Every reserved verb is unbound and unscored; every bound verb is scored.
        for v in &reg {
            if v.is_reserved() {
                assert!(v.binding_specs.is_empty(), "reserved {} is bound", v.longname);
                assert!(v.scores.is_none(), "reserved {} is scored", v.longname);
                assert!(v.reserved_reason.is_some(), "reserved {} has no reason", v.longname);
            } else {
                assert!(v.is_bound(), "active {} is unbound", v.longname);
                assert!(v.scores.is_some(), "bound {} is unscored", v.longname);
                assert!(v.reserved_reason.is_none(), "active {} has a reserved reason", v.longname);
            }
        }
    }

    #[test]
    fn kbg_ac01_longname_snapshot_is_stable() {
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
        ];
        assert_eq!(got, expected);
    }

    #[test]
    fn kbg_ac01_producers_take_only_the_registry() {
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
        let palette_keys: Vec<_> = keys.iter().filter(|k| k.longname == "open-palette").collect();
        assert_eq!(palette_keys.len(), 2);
    }

    #[test]
    fn kbg_ac01_cycle_colour_scheme_is_view_only_preview() {
        let reg = registry();
        let c = reg.iter().find(|v| v.longname == "cycle-colour-scheme").unwrap();
        assert_eq!(c.status, VerbStatus::Preview);
        assert_eq!(c.scope_applicability, vec![Altitude::View]);
        assert!(!c.applies_at(Altitude::Dashboard));
    }
}
