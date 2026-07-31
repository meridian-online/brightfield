//! What a pane declares about itself, once per frame.
//!
//! Nothing in this module is an egui type, a colour, or a GPU handle. That is
//! not minimalism for its own sake: it means the entire visible chrome of any
//! surface — its title, its breadcrumb, every control it offers, every status
//! line it raises, and the empty state it falls back to — can be built and
//! asserted in a unit test with no adapter, no window and no renderer. A
//! `Subject` is comparable, printable data; a snapshot test can pin what a
//! pane *says* without rendering a pixel of it.
//!
//! The temptation to put an `egui::Color32` or an `egui::Response` in here
//! will arrive, and it should be refused. The moment a `Subject` carries
//! paint, the pane is drawing chrome again, just at one remove.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use brightfield_keys::BindingContext;
use meridian_design::a11y::WidgetRole;

// ---------------------------------------------------------------------------
// Verb
// ---------------------------------------------------------------------------

/// A command longname from the keyboard registry.
///
/// The newtype on its own guarantees nothing — a typo is a `&'static str`
/// like any other, and would ship as a control that quietly does nothing when
/// clicked. So construction goes through [`Verb::new`], which checks the
/// longname against `brightfield_keys::registry()` and asserts in debug.
/// Every `Subject` the suite builds runs through the contract test, so a typo
/// fails a test rather than reaching a user; [`Verb::is_registered`] is the
/// release-mode backstop the contract test actually asserts on.
///
/// The rule this enforces is that **the shell may not invent verbs**. If a
/// pane wants to offer an action, the action is a registered command with a
/// keystroke and a help line, discoverable in the palette — not a button that
/// exists only where someone happened to put it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Verb(&'static str);

fn longnames() -> &'static BTreeSet<&'static str> {
    static NAMES: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        brightfield_keys::registry()
            .iter()
            .map(|v| v.longname)
            .collect()
    })
}

impl Verb {
    /// A verb by its registry longname.
    ///
    /// # Panics
    ///
    /// In debug builds, if `longname` is not a registered command.
    #[must_use]
    pub fn new(longname: &'static str) -> Self {
        debug_assert!(
            longnames().contains(longname),
            "verb {longname:?} is not in the keyboard registry — the shell may not invent verbs"
        );
        Self(longname)
    }

    /// The longname, for a test assertion or a log line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// Whether this verb is a real registered command. The release-mode form
    /// of the check [`Verb::new`] makes in debug.
    #[must_use]
    pub fn is_registered(self) -> bool {
        longnames().contains(self.0)
    }

    /// The rendered primary keystroke, for an affordance line or a tooltip.
    ///
    /// Derived, never stored: an [`Affordance`] that carried its own copy of
    /// the keystroke would be one rebinding away from telling the user
    /// something untrue.
    #[must_use]
    pub fn keys(self) -> Option<&'static str> {
        brightfield_keys::registry()
            .iter()
            .find(|v| v.longname == self.0)
            .and_then(brightfield_keys::VerbEntry::primary_key)
    }
}

// ---------------------------------------------------------------------------
// Plain-data parts
// ---------------------------------------------------------------------------

/// A name from the Meridian icon set — a name, never a glyph.
///
/// Resolved to paint at draw time, so a `Subject` stays comparable data and
/// a test can assert *which* icon a pane asked for without knowing what it
/// looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Icon(pub &'static str);

impl Icon {
    /// The icon's name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Unsaved-edit state.
///
/// An enum rather than a `bool` so that "clean" and "this surface has no
/// notion of saving" are the same value, and neither can be mistaken for
/// "edited" by a `!` in the wrong place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dirty {
    /// Nothing to save.
    #[default]
    Clean,
    /// Edited since the last save.
    Edited,
}

/// One hop of a breadcrumb trail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    /// The hop's label.
    pub label: String,
    /// An optional leading icon.
    pub icon: Option<Icon>,
}

impl Crumb {
    /// A crumb with no icon.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
        }
    }

    /// Give this crumb a leading icon.
    #[must_use]
    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// Semantic tone.
///
/// Resolved to paint by [`crate::chrome`], so a `Subject` holds no colour and
/// a pane cannot reach for an accent that is reserved for something else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tone {
    /// The default. Chrome ink.
    #[default]
    Neutral,
    /// The interaction accent. Reserved — see [`crate::chrome::tone_colour`].
    Accent,
    /// Something succeeded.
    Good,
    /// Something needs attention but is not broken.
    Warning,
    /// Something is broken.
    Critical,
}

/// Where a toolbar entry sits in the row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolbarLocation {
    /// Left group, in declaration order.
    #[default]
    Leading,
    /// Right group, in declaration order.
    Trailing,
    /// Behind the overflow affordance.
    Overflow,
    /// Declared but not offered right now.
    ///
    /// This variant exists so an item can *declare* a control it is currently
    /// withholding instead of conditionally omitting it from the vector. Two
    /// things follow: the declaration stays greppable, so the full vocabulary
    /// of a surface is one `Subject` away rather than scattered across
    /// branches; and the row does not reflow as controls appear and vanish,
    /// which is the thing that makes a toolbar feel unstable.
    Hidden,
}

/// One control in a pane's toolbar row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolbarEntry {
    /// Stable id, for a test hook and for the overflow menu's ordering.
    pub id: &'static str,
    /// Where it sits.
    pub location: ToolbarLocation,
    /// The visible label.
    pub label: String,
    /// An optional leading icon.
    pub icon: Option<Icon>,
    /// The command this control runs.
    pub verb: Verb,
    /// What this control *is*, for assistive technology. Taken from the
    /// design system's role roster rather than re-derived here, so the shell
    /// and the web speak one accessibility vocabulary.
    pub role: WidgetRole,
    /// Whether it can be activated right now.
    pub enabled: bool,
    /// A toggle reports its own on-state here.
    ///
    /// Painted with the selection wash and nothing else: a second accent for
    /// "this toggle is on" is how a palette acquires two meanings for one
    /// colour.
    pub on: bool,
    /// Hover text. The keystroke is appended by the chrome from
    /// [`Verb::keys`]; do not spell it here.
    pub tooltip: Option<String>,
}

impl ToolbarEntry {
    /// A leading, enabled, un-toggled button.
    #[must_use]
    pub fn button(id: &'static str, label: impl Into<String>, verb: Verb) -> Self {
        Self {
            id,
            location: ToolbarLocation::Leading,
            label: label.into(),
            icon: None,
            verb,
            role: WidgetRole::Button,
            enabled: true,
            on: false,
            tooltip: None,
        }
    }

    /// Move this entry to a different group.
    #[must_use]
    pub fn at(mut self, location: ToolbarLocation) -> Self {
        self.location = location;
        self
    }

    /// Mark this entry as a toggle in the given state.
    #[must_use]
    pub fn toggle(mut self, on: bool) -> Self {
        self.role = WidgetRole::Switch;
        self.on = on;
        self
    }

    /// Enable or disable the entry.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Which end of the status rail an entry sits at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatusSide {
    /// Left, where transient messages land.
    #[default]
    Leading,
    /// Right, where standing state sits.
    Trailing,
}

/// How the user gets rid of a status entry.
///
/// Required rather than optional, and that is the whole design: a status
/// entry the user cannot dismiss, cannot hide, and that will not clear itself
/// is chrome nobody asked for and nobody can get rid of. Making the answer a
/// required field means the question is asked every time one is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HideAffordance {
    /// A command clears it.
    Verb(Verb),
    /// It goes when the whole status rail is hidden.
    WithRail,
    /// It clears itself after `ms` — a yank confirmation, a save notice.
    Transient {
        /// Lifetime in milliseconds.
        ms: u32,
    },
}

/// One entry in the status rail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    /// Stable name for this line. **Not a replacement key** — nothing here
    /// dedups by it: [`Subject::with_status`] pushes onto a `Vec` and
    /// `chrome::status_rail` draws every entry it is handed, in order, so two
    /// entries sharing an id draw twice
    /// (`tests/chrome_rules.rs::two_entries_sharing_an_id_both_draw`).
    ///
    /// It is read in exactly one place in production: [`Activity::of_entry`]
    /// matches it against the ids in [`Activity::ALL`], which is how the window
    /// tells a pane's own activity report from its other lines and folds it into
    /// the one merged indicator instead of railing it twice. Every other id is a
    /// handle a headless test selects a line by without matching on its text,
    /// and the name the rail records in `chrome::StatusDrawn::drawn`.
    ///
    /// Dismissal does **not** travel on it — that is the entry's
    /// [`HideAffordance`].
    pub id: &'static str,
    /// Which end it sits at.
    pub side: StatusSide,
    /// The text.
    pub text: String,
    /// Its tone.
    pub tone: Tone,
    /// How it goes away.
    pub hide: HideAffordance,
}

/// How the data a surface previews relates to what the pipeline currently
/// specifies — the honesty channel of the shell contract.
///
/// A document has one state. A pipeline has two: what is *specified* and what
/// has been *materialised*, and the two diverge the moment a step is edited.
/// A surface that previews materialised data is honest only if it labels that
/// data against the current specification — and this enum is the label's
/// whole vocabulary. There is deliberately no second one anywhere in the
/// workspace: the viz status inks reach it through [`RunState::tone`] →
/// [`Tone`] → the design system's reserved status colours, and the old
/// shell's transient feedback severities fold into [`Tone`] the same way,
/// rather than growing a parallel palette.
///
/// **Values of this type are ingested, never computed here.** The engine that
/// runs the pipeline is the only thing that computes staleness (it fingerprints
/// SQL and content and records a typed skip reason in the emitted run
/// contract); a surface reads those recorded fields, overlays the not-yet-run
/// edits the user just made, and labels. Nothing in the shell re-derives
/// freshness from hashes.
///
/// The default is [`RunState::NeverRun`] because that is the safe direction:
/// nothing reads as current until a run's record proves it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunState {
    /// No run has materialised this data — or none this preview can vouch
    /// for. Never presented as current, and never silently indistinguishable
    /// from [`RunState::Fresh`]: the two differ in both label and tone.
    #[default]
    NeverRun,
    /// The materialised data matches the step as currently specified — the
    /// run's record says so (a clean success, or a skip the engine recorded
    /// as fresh). The only state a preview may present as current.
    Fresh,
    /// This step's own definition changed after its data was materialised.
    /// The preview shows the *old* definition's output.
    StaleOwnEdit,
    /// A step upstream changed after this data was materialised. Nothing here
    /// was touched and nothing has run — which is exactly why the label
    /// exists: the data is downstream of an edit it has not seen.
    StaleUpstream,
    /// The last attempt to materialise this data failed.
    Failed,
}

impl RunState {
    /// Every state, for a test that walks the vocabulary.
    pub const ALL: [RunState; 5] = [
        RunState::NeverRun,
        RunState::Fresh,
        RunState::StaleOwnEdit,
        RunState::StaleUpstream,
        RunState::Failed,
    ];

    /// The short label a surface renders beside previewed data. Always shown
    /// with the tone, never replaced by it — colour alone is not a label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            RunState::NeverRun => "never run",
            RunState::Fresh => "fresh",
            RunState::StaleOwnEdit => "stale · edited",
            RunState::StaleUpstream => "stale · upstream edited",
            RunState::Failed => "failed",
        }
    }

    /// A one-line explainer for an inspector or tooltip.
    #[must_use]
    pub const fn gloss(self) -> &'static str {
        match self {
            RunState::NeverRun => {
                "No run has produced this data yet — nothing to preview as current."
            }
            RunState::Fresh => {
                "The last run's record says this data matches the step as specified."
            }
            RunState::StaleOwnEdit => {
                "This step was edited after its data was produced — run to refresh."
            }
            RunState::StaleUpstream => {
                "A step upstream was edited — this data has not seen that change."
            }
            RunState::Failed => "The last run of this step failed — this data was not produced.",
        }
    }

    /// The tone this state takes, which is how it reaches ink: the chrome
    /// resolves [`Tone::Good`]/[`Tone::Warning`]/[`Tone::Critical`] to the
    /// design system's reserved status colours, and [`Tone::Neutral`] to
    /// quiet text ink — so never-run can never wear fresh's green.
    #[must_use]
    pub const fn tone(self) -> Tone {
        match self {
            RunState::NeverRun => Tone::Neutral,
            RunState::Fresh => Tone::Good,
            RunState::StaleOwnEdit | RunState::StaleUpstream => Tone::Warning,
            RunState::Failed => Tone::Critical,
        }
    }

    /// Whether a preview may present this data as current. True for
    /// [`RunState::Fresh`] and nothing else — the honesty rule in one
    /// predicate: materialised data is never shown as though it were current
    /// unless the run's record proves it is.
    #[must_use]
    pub const fn is_current(self) -> bool {
        matches!(self, RunState::Fresh)
    }

    /// Whether the data exists but no longer matches the specification.
    #[must_use]
    pub const fn is_stale(self) -> bool {
        matches!(self, RunState::StaleOwnEdit | RunState::StaleUpstream)
    }

    /// This state as a standing status-rail entry: trailing (standing state
    /// sits right), cleared with the rail. The words and tone come from
    /// [`RunState::label`] and [`RunState::tone`], so every surface that rails
    /// a run-state says it the same way.
    #[must_use]
    pub fn status_entry(self, id: &'static str) -> StatusEntry {
        StatusEntry {
            id,
            side: StatusSide::Trailing,
            text: self.label().to_string(),
            tone: self.tone(),
            hide: HideAffordance::WithRail,
        }
    }
}

/// What activating an [`Affordance`] does.
///
/// # Why this is an enum, and why it is not a hole in "the shell may not
/// invent verbs"
///
/// [`Verb`]'s rule is about **commands**: a thing with a keystroke, a help
/// line and a palette entry, so that an action offered in one corner of the UI
/// is reachable from everywhere. That rule is why [`Action::Verb`] exists and
/// why the audit checks it.
///
/// It does not fit an affordance whose whole content is *which object to
/// open*. A shipped starting point has nothing to bind — a verb minted to
/// carry one would be a palette entry meaning "open the thing I happen to be
/// standing next to", and the keyboard registry would grow an entry per
/// fixture. So [`Action::Open`] names the object instead, and the two
/// consequences are deliberate: it renders **no keystroke** (see
/// [`crate::chrome::empty_state`]), so a button cannot claim a key that does
/// not exist; and [`Subject::declared_verbs`] does not see it, so it neither
/// weakens nor bypasses the registration gate — there is simply no verb to
/// check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Run a registered command.
    Verb(Verb),
    /// Open something the shell can name and the keyboard cannot: a starting
    /// point that ships with the binary, addressed by a stable id.
    ///
    /// The id is opaque to this crate. A shell handed one it does not
    /// recognise should do nothing, which is the same thing it does for a
    /// pane id it does not recognise.
    Open(&'static str),
}

/// The action that resolves an empty state.
///
/// Deliberately without a `keys` field: for an [`Action::Verb`] the keystroke
/// is derivable from the verb ([`Verb::keys`]), and storing a second copy is an
/// invitation for the two to disagree after a rebinding. An [`Action::Open`]
/// has no keystroke at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Affordance {
    /// What the action is called.
    pub label: String,
    /// What activating it does.
    pub action: Action,
}

impl Affordance {
    /// An affordance that runs `verb`.
    #[must_use]
    pub fn new(label: impl Into<String>, verb: Verb) -> Self {
        Self {
            label: label.into(),
            action: Action::Verb(verb),
        }
    }

    /// An affordance that opens the shipped starting point `id`.
    #[must_use]
    pub fn open(label: impl Into<String>, id: &'static str) -> Self {
        Self {
            label: label.into(),
            action: Action::Open(id),
        }
    }
}

/// What a pane shows when it has nothing to show.
///
/// A required struct, not a `bool` and not an `Option<String>`, because the
/// two worst empty states found in the shell this crate replaces were the two
/// that did not exist: an empty outline rendered a header and silence, and an
/// empty steps sheet rendered a column header and silence. Neither read as
/// "nothing here yet" — both read as broken. A count of existing treatments
/// would have missed exactly those two, and they are the two a first-time
/// user is most likely to hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyState {
    /// The illustration.
    pub icon: Icon,
    /// One line, sentence case, no terminal period.
    pub headline: String,
    /// One or two lines naming what would put content here.
    pub body: String,
    /// The action that resolves it, when there is one.
    pub next: Option<Affordance>,
}

impl EmptyState {
    /// An empty state with no resolving action.
    #[must_use]
    pub fn new(icon: Icon, headline: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            icon,
            headline: headline.into(),
            body: body.into(),
            next: None,
        }
    }

    /// Give this empty state a resolving action.
    #[must_use]
    pub fn with_next(mut self, next: Affordance) -> Self {
        self.next = Some(next);
        self
    }
}

// ---------------------------------------------------------------------------
// Subject
// ---------------------------------------------------------------------------

/// Everything the shell needs in order to draw chrome for one pane.
///
/// Returned once per frame from [`crate::Item::subject`], which takes `&self`
/// and `&D` — so building a `Subject` can never become a second update path,
/// and the shell can always ask any pane for one, including a pane that is
/// not currently drawing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    /// The pane's name. Drawn once — as the tab title if the pane is in a tab
    /// strip, as the header band otherwise, never both.
    pub title: String,
    /// The pane's icon.
    pub icon: Icon,
    /// Where the pane's content sits, most general first. Empty for a pane
    /// with no drill state.
    pub breadcrumb: Vec<Crumb>,
    /// Whether the pane has unsaved edits.
    pub dirty: Dirty,
    /// Which keyboard context this pane resolves bindings in. The keyboard
    /// registry's own type, not a parallel vocabulary — a second enum with
    /// four overlapping variants would drift within a release.
    pub key_context: BindingContext,
    /// The controls this pane offers.
    pub toolbar: Vec<ToolbarEntry>,
    /// The status lines this pane raises.
    pub status: Vec<StatusEntry>,
    /// `Some` means there is nothing to show: the shell paints it and does
    /// **not** call [`crate::Item::ui`] — an empty state that is not a branch
    /// inside the pane's draw code, and so not one anybody can forget to
    /// write.
    ///
    /// **Populated by [`crate::Item::subject`], never by a pane.** The answer
    /// comes from [`crate::Item::empty_state`], which is a *required* trait
    /// method — a pane that has not decided what it shows when empty does not
    /// compile. `describe()` implementations leave this `None`; the provided
    /// `subject()` glue folds the method's answer in, and asserts in debug
    /// that nothing set it early.
    pub empty_state: Option<EmptyState>,
}

impl Subject {
    /// A subject with a title, an icon and a key context, and nothing else.
    #[must_use]
    pub fn new(title: impl Into<String>, icon: Icon, key_context: BindingContext) -> Self {
        Self {
            title: title.into(),
            icon,
            breadcrumb: Vec::new(),
            dirty: Dirty::Clean,
            key_context,
            toolbar: Vec::new(),
            status: Vec::new(),
            empty_state: None,
        }
    }

    /// Add a toolbar control.
    #[must_use]
    pub fn with_toolbar(mut self, entry: ToolbarEntry) -> Self {
        self.toolbar.push(entry);
        self
    }

    /// Add a status line.
    #[must_use]
    pub fn with_status(mut self, entry: StatusEntry) -> Self {
        self.status.push(entry);
        self
    }

    /// Set the breadcrumb trail.
    #[must_use]
    pub fn with_breadcrumb(mut self, crumbs: Vec<Crumb>) -> Self {
        self.breadcrumb = crumbs;
        self
    }

    /// Mark the pane as having unsaved edits.
    #[must_use]
    pub fn dirty(mut self, dirty: Dirty) -> Self {
        self.dirty = dirty;
        self
    }

    /// Every verb this subject declares anywhere — toolbar controls, the
    /// empty state's resolving action, and any status entry that names a
    /// command to clear it. The contract test walks this rather than
    /// hand-listing the fields, so a new verb-bearing field cannot escape the
    /// gate by being forgotten in a test.
    #[must_use]
    pub fn declared_verbs(&self) -> Vec<Verb> {
        let mut verbs: Vec<Verb> = self.toolbar.iter().map(|t| t.verb).collect();
        if let Some(empty) = &self.empty_state {
            // An `Action::Open` declares no verb — there is nothing here to
            // check against the registry, which is the point of it.
            if let Some(Action::Verb(v)) = empty.next.as_ref().map(|next| next.action) {
                verbs.push(v);
            }
        }
        for entry in &self.status {
            if let HideAffordance::Verb(v) = entry.hide {
                verbs.push(v);
            }
        }
        verbs
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

/// Unit tests rather than integration tests, for one reason: [`Verb`]'s field
/// is private, so this is the only place in the workspace that can build the
/// case the check exists to catch — a `Verb` that never went through
/// [`Verb::new`] and names a command the registry does not have.
///
/// An integration test cannot construct that value, which is why the contract
/// suite could assert `is_registered()` fifteen times over and still stay
/// green with the function replaced by `true`: every verb it could reach had
/// already been through `Verb::new`'s debug assertion.
#[cfg(test)]
mod tests {
    use super::*;

    /// A longname no keyboard registry will ever carry.
    const INVENTED: &str = "summon-a-pony";

    #[test]
    fn the_keyboard_registry_really_does_not_have_the_invented_verb() {
        // Otherwise the negative case below proves nothing.
        assert!(!longnames().contains(INVENTED));
    }

    #[test]
    fn a_verb_that_bypassed_the_constructor_is_not_registered() {
        // Built by hand precisely as a release build could: the tuple
        // constructor, no `Verb::new`, no debug assertion.
        let smuggled = Verb(INVENTED);
        assert!(
            !smuggled.is_registered(),
            "is_registered() accepted {INVENTED:?} — the release-mode backstop is gone"
        );
    }

    #[test]
    fn a_real_verb_is_registered() {
        let real = *longnames()
            .iter()
            .next()
            .expect("the keyboard registry is not empty");
        assert!(Verb(real).is_registered());
        assert!(Verb::new(real).is_registered());
    }

    #[test]
    fn an_unregistered_verb_has_no_keystroke() {
        assert_eq!(Verb(INVENTED).keys(), None);
    }

    /// `Verb::new` asserts with `debug_assert!`, so this holds only where
    /// debug assertions are on — every normal test build, including CI's, but
    /// not `--release`.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "the shell may not invent verbs")]
    fn constructing_an_unregistered_verb_panics_in_debug() {
        let _ = Verb::new(INVENTED);
    }

    // -- RunState: the data-honesty vocabulary ------------------------------

    /// The floor of the honesty rule: a step that has never run must not be
    /// silently indistinguishable from one that ran clean. Both the words and
    /// the tone differ, so neither channel alone carries the distinction.
    #[test]
    fn never_run_is_distinct_from_fresh_in_both_label_and_tone() {
        assert_ne!(RunState::NeverRun.label(), RunState::Fresh.label());
        assert_ne!(RunState::NeverRun.tone(), RunState::Fresh.tone());
        assert_ne!(RunState::NeverRun.gloss(), RunState::Fresh.gloss());
    }

    /// The honesty predicate: materialised data may be presented as current
    /// under exactly one state. Walked over the whole vocabulary so a new
    /// variant cannot join the current club by forgetting a match arm.
    #[test]
    fn a_preview_may_claim_current_only_when_fresh() {
        for state in RunState::ALL {
            assert_eq!(
                state.is_current(),
                state == RunState::Fresh,
                "{state:?} may not present data as current unless it is Fresh"
            );
        }
    }

    /// Every state says something different — five states, five labels, five
    /// glosses. A shared label would make two states indistinguishable in the
    /// one channel a user actually reads.
    #[test]
    fn every_run_state_labels_itself_distinctly() {
        for a in RunState::ALL {
            for b in RunState::ALL {
                if a != b {
                    assert_ne!(a.label(), b.label(), "{a:?} and {b:?} share a label");
                    assert_ne!(a.gloss(), b.gloss(), "{a:?} and {b:?} share a gloss");
                }
            }
        }
        // Both stale states say "stale" — same family, different cause.
        assert!(RunState::StaleOwnEdit.label().contains("stale"));
        assert!(RunState::StaleUpstream.label().contains("stale"));
    }

    /// The tone mapping is the whole reconciliation with the design system's
    /// reserved status inks: fresh takes good, both stales take warning,
    /// failure takes critical, and never-run stays neutral — it is the
    /// *absence* of a run, not a good or bad one.
    #[test]
    fn run_state_tones_reconcile_into_the_one_tone_vocabulary() {
        assert_eq!(RunState::Fresh.tone(), Tone::Good);
        assert_eq!(RunState::StaleOwnEdit.tone(), Tone::Warning);
        assert_eq!(RunState::StaleUpstream.tone(), Tone::Warning);
        assert_eq!(RunState::Failed.tone(), Tone::Critical);
        assert_eq!(RunState::NeverRun.tone(), Tone::Neutral);
        // Staleness is never dressed as success or as the interaction accent.
        for state in RunState::ALL {
            if state.is_stale() {
                assert_ne!(state.tone(), Tone::Good);
                assert_ne!(state.tone(), Tone::Accent);
            }
        }
    }

    /// The default is the safe direction: nothing reads as current until a
    /// run's record proves it.
    #[test]
    fn the_default_run_state_never_claims_current() {
        assert_eq!(RunState::default(), RunState::NeverRun);
        assert!(!RunState::default().is_current());
    }

    /// A railed run-state carries its own words and tone — one spelling for
    /// every surface — and is standing state: trailing, cleared with the rail,
    /// declaring no verb (so the registration gate has nothing to check).
    #[test]
    fn a_run_state_status_entry_carries_its_own_words_and_tone() {
        for state in RunState::ALL {
            let entry = state.status_entry("run-state");
            assert_eq!(entry.id, "run-state");
            assert_eq!(entry.side, StatusSide::Trailing);
            assert_eq!(entry.text, state.label());
            assert_eq!(entry.tone, state.tone());
            assert_eq!(entry.hide, HideAffordance::WithRail);
        }
        let subject = Subject::new("t", Icon("i"), BindingContext::Global)
            .with_status(RunState::StaleUpstream.status_entry("run-state"));
        assert!(subject.declared_verbs().is_empty());
    }
}
